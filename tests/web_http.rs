use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use document_templating_mcp::{
    model::{FieldDefinition, FieldType, Template, TemplateRender, TemplateSection},
    server::DocumentMcpServer,
    store::Store,
    web,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "test-token-that-is-longer-than-32-characters";

fn template() -> Template {
    Template {
        schema_version: 1,
        id: "letter".into(),
        name: "Letter".into(),
        version: "1.0.0".into(),
        description: "Reusable letter".into(),
        sections: vec![TemplateSection {
            id: "content".into(),
            label: "Content".into(),
            description: String::new(),
            fields: vec![FieldDefinition {
                id: "recipient".into(),
                path: "recipient".into(),
                label: "Recipient".into(),
                field_type: FieldType::Text,
                required: true,
                fields: vec![],
            }],
        }],
        defaults: json!({"recipient": "", "message": "Hello"}),
        preview_data: json!({"recipient": "Mock recipient"}),
        render: TemplateRender {
            html: "<h1>{{recipient}}</h1><p>{{message}}</p>".into(),
            css: "h1 { color: navy; }".into(),
        },
        page: Default::default(),
        lang: "en".into(),
    }
}

fn app(directory: &TempDir) -> (Arc<Store>, Router) {
    let store = Arc::new(Store::open(directory.path()).unwrap());
    store.create_template(template()).unwrap();
    let server = DocumentMcpServer::with_shared_store(store.clone());
    (store.clone(), web::router(store, server, TOKEN.into()))
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> (StatusCode, header::HeaderMap, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "documents.test");
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let body = body.map_or_else(Body::empty, |value| Body::from(value.to_string()));
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, headers, value)
}

fn session_cookie(headers: &header::HeaderMap) -> String {
    headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn browser_api_supports_the_complete_visual_workflow() {
    let directory = TempDir::new().unwrap();
    let (_store, app) = app(&directory);

    let (status, _, _) = request(&app, "GET", "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(&app, "GET", "/api/templates", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, headers, _) = request(
        &app,
        "POST",
        "/api/auth/login",
        Some(json!({"token": TOKEN})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = session_cookie(&headers);

    let (status, _, gallery) = request(&app, "GET", "/api/templates", None, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(gallery["templates"][0]["preview_html"]
        .as_str()
        .unwrap()
        .contains("Mock recipient"));

    let (status, _, document) = request(
        &app,
        "POST",
        "/api/documents",
        Some(json!({"id": "draft", "name": "Draft", "template_id": "letter"})),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(document["data"]["message"], "Hello");

    let (status, _, preview) = request(
        &app,
        "POST",
        "/api/preview",
        Some(json!({"template_id": "letter", "data": {"recipient": "Unsaved"}})),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(preview["html"].as_str().unwrap().contains("Unsaved"));

    let mut saved: document_templating_mcp::model::Document =
        serde_json::from_value(document).unwrap();
    saved.data["recipient"] = json!("Saved recipient");
    let (status, _, _) = request(
        &app,
        "PUT",
        "/api/documents/draft",
        Some(serde_json::to_value(saved).unwrap()),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/documents/draft/render",
        Some(json!({})),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, html) = request(&app, "GET", "/renders/draft", None, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.as_str().unwrap().contains("Saved recipient"));

    if document_templating_mcp::pdf::ChromiumRenderer::discover().is_available() {
        let (status, _, pdf) = request(
            &app,
            "POST",
            "/api/documents/draft/pdf",
            Some(json!({})),
            Some(&cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(pdf["mime_type"], "application/pdf");
        assert!(pdf.get("pdf_base64").is_none());
        let (status, headers, pdf) =
            request(&app, "GET", "/renders/draft.pdf", None, Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "application/pdf");
        assert!(pdf.as_str().unwrap().starts_with("%PDF-"));
    }
}

#[tokio::test]
async fn rejects_invalid_origins_and_requires_mcp_bearer_auth() {
    let directory = TempDir::new().unwrap();
    let (_store, app) = app(&directory);
    let origin_request = Request::builder()
        .method("GET")
        .uri("/api/templates")
        .header(header::HOST, "documents.test")
        .header(header::ORIGIN, "https://attacker.test")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(origin_request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "http-test", "version": "1.0.0"}
        }
    });
    let (status, _, _) = request(&app, "POST", "/mcp", Some(initialize.clone()), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let authorized = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from(initialize.to_string()))
        .unwrap();
    let response = app.clone().oneshot(authorized).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("document-templating-mcp"), "{text}");
}
