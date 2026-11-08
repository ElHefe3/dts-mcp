use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rmcp::transport::{
    streamable_http_server::{session::local::LocalSessionManager, StreamableHttpServerConfig},
    StreamableHttpService,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    error::AppError,
    model::{Deleted, Document},
    server::{CreateDocumentRequest, DocumentMcpServer, PreviewDocumentRequest},
    store::Store,
};

const APP_HTML: &str = include_str!("../web/dist/index.html");
const SESSION_COOKIE: &str = "dts_session";
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
struct WebState {
    store: Arc<Store>,
    token: Arc<str>,
}

pub async fn serve(store: Store, address: SocketAddr, token: String) -> anyhow::Result<()> {
    if token.len() < 32 {
        anyhow::bail!("DOCUMENT_TEMPLATING_MCP_TOKEN must contain at least 32 characters");
    }
    let store = Arc::new(store);
    let server = DocumentMcpServer::with_shared_store(store.clone());
    let app = router(store, server, token);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "HTTP server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn router(store: Arc<Store>, server: DocumentMcpServer, token: String) -> Router {
    let state = Arc::new(WebState {
        store,
        token: Arc::from(token),
    });
    let mcp_service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let mcp = Router::new().nest_service("/mcp", mcp_service).route_layer(
        middleware::from_fn_with_state(state.clone(), require_mcp_auth),
    );
    let protected_api = Router::new()
        .route("/api/templates", get(list_templates))
        .route("/api/documents", get(list_documents).post(create_document))
        .route(
            "/api/documents/{id}",
            get(get_document)
                .put(update_document)
                .delete(delete_document),
        )
        .route("/api/documents/{id}/render", post(render_document))
        .route("/api/documents/{id}/pdf", post(render_pdf))
        .route("/api/preview", post(preview_document))
        .route("/renders/{id}", get(rendered_document))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_browser_auth,
        ));

    Router::new()
        .route("/", get(index))
        .route("/healthz", get(health))
        .route("/api/auth/check", get(auth_check))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .merge(protected_api)
        .merge(mcp)
        .with_state(state)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(validate_origin))
}

async fn validate_origin(request: Request, next: Next) -> Response {
    let valid = match request.headers().get(header::ORIGIN) {
        None => true,
        Some(origin) => {
            let origin = origin
                .to_str()
                .ok()
                .and_then(|value| value.parse::<Uri>().ok());
            let host = request
                .headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok());
            match (origin, host) {
                (Some(origin), Some(host)) => origin
                    .authority()
                    .is_some_and(|value| value.as_str() == host),
                _ => false,
            }
        }
    };
    if valid {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "origin is not allowed"})),
        )
            .into_response()
    }
}

async fn require_mcp_auth(
    State(state): State<Arc<WebState>>,
    request: Request,
    next: Next,
) -> Response {
    let authenticated = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == state.token.as_ref());
    if authenticated {
        next.run(request).await
    } else {
        unauthorized()
    }
}

async fn require_browser_auth(
    State(state): State<Arc<WebState>>,
    request: Request,
    next: Next,
) -> Response {
    if browser_authenticated(request.headers(), &state.token) {
        next.run(request).await
    } else {
        unauthorized()
    }
}

fn browser_authenticated(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .into_iter()
        .flat_map(|value| value.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .any(|(name, value)| name == SESSION_COOKIE && value == token)
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(json!({"error": "authentication required"})),
    )
        .into_response()
}

async fn index() -> Response {
    (
        [(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; frame-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'",
        )],
        Html(APP_HTML),
    )
        .into_response()
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true}))
}

async fn auth_check(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Json<Value> {
    Json(json!({"authenticated": browser_authenticated(&headers, &state.token)}))
}

#[derive(Deserialize)]
struct LoginRequest {
    token: String,
}

async fn login(State(state): State<Arc<WebState>>, Json(request): Json<LoginRequest>) -> Response {
    if request.token != state.token.as_ref() {
        return unauthorized();
    }
    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=86400",
        state.token
    );
    (
        StatusCode::OK,
        [(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap())],
        Json(json!({"authenticated": true})),
    )
        .into_response()
}

async fn logout() -> Response {
    let cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0");
    (
        [(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap())],
        Json(json!({"authenticated": false})),
    )
        .into_response()
}

async fn list_templates(State(state): State<Arc<WebState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(state.store.template_gallery()?)?))
}

async fn list_documents(State(state): State<Arc<WebState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(state.store.list_documents()?)?))
}

async fn get_document(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(state.store.editor_state(&id)?)?))
}

async fn create_document(
    State(state): State<Arc<WebState>>,
    Json(request): Json<CreateDocumentRequest>,
) -> Result<Json<Document>, ApiError> {
    let document = Document {
        schema_version: 1,
        id: request.id,
        name: request.name,
        template_id: request.template_id,
        data: Value::Object(Default::default()),
    };
    Ok(Json(
        state
            .store
            .create_document(document, request.data.as_ref())?,
    ))
}

async fn update_document(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
    Json(document): Json<Document>,
) -> Result<Json<Document>, ApiError> {
    if id != document.id {
        return Err(ApiError(AppError::Invalid(
            "path id must match document.id".into(),
        )));
    }
    Ok(Json(state.store.update_document(document)?))
}

async fn delete_document(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Result<Json<Deleted>, ApiError> {
    state.store.delete_document(&id)?;
    Ok(Json(Deleted { id, deleted: true }))
}

async fn preview_document(
    State(state): State<Arc<WebState>>,
    Json(request): Json<PreviewDocumentRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(
        state
            .store
            .preview_document(&request.template_id, &request.data)?,
    )?))
}

async fn render_document(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(
        state.store.render_document(&id)?,
    )?))
}

async fn render_pdf(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(state.store.render_pdf(&id)?)?))
}

async fn rendered_pdf(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = id.strip_suffix(".pdf").unwrap_or(&id);
    let bytes = state.store.rendered_pdf_bytes(id)?;
    let disposition = format!("inline; filename=\"{id}.pdf\"");
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/pdf"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition).unwrap(),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn rendered_document(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    if id.ends_with(".pdf") {
        return rendered_pdf(State(state), Path(id)).await;
    }
    Ok((
        [(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; img-src data: https:; object-src 'none'; base-uri 'none'",
        )],
        Html(state.store.rendered_document_html(&id)?),
    )
        .into_response())
}

struct ApiError(AppError);

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self(AppError::storage(error))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::AlreadyExists(_) | AppError::InUse(_) => StatusCode::CONFLICT,
            AppError::Invalid(_) => StatusCode::BAD_REQUEST,
            AppError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error": self.0.to_string()}))).into_response()
    }
}
