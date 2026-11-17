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
    assets,
    error::AppError,
    model::{Deleted, Document},
    plugins::{PluginImport, PluginManager},
    server::{
        CreateDocumentRequest, DocumentMcpServer, PreviewDocumentRequest, UploadAssetRequest,
    },
    store::Store,
};

const APP_HTML: &str = include_str!("../web/dist/index.html");
const SESSION_COOKIE: &str = "dts_session";
/// Large enough for a 5 MB asset after base64 expansion plus JSON framing.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct WebState {
    store: Arc<Store>,
    plugins: Arc<PluginManager>,
    token: Arc<str>,
}

pub async fn serve(store: Store, address: SocketAddr, token: String) -> anyhow::Result<()> {
    if token.len() < 32 {
        anyhow::bail!("DOCUMENT_TEMPLATING_MCP_TOKEN must contain at least 32 characters");
    }
    let store = Arc::new(store);
    let plugins = Arc::new(PluginManager::open(store.plugins_dir()).map_err(anyhow::Error::new)?);
    let server = DocumentMcpServer::with_shared_store_and_plugins(store.clone(), plugins.clone());
    let app = router_with_plugins(store, server, token, plugins);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "HTTP server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn router(store: Arc<Store>, server: DocumentMcpServer, token: String) -> Router {
    let plugins = Arc::new(
        PluginManager::open(store.plugins_dir()).expect("plugin directory must be available"),
    );
    router_with_plugins(store, server.with_plugins(plugins.clone()), token, plugins)
}

fn router_with_plugins(
    store: Arc<Store>,
    server: DocumentMcpServer,
    token: String,
    plugins: Arc<PluginManager>,
) -> Router {
    let state = Arc::new(WebState {
        store,
        plugins,
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
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/{id}", axum::routing::put(set_plugin_enabled))
        .route("/api/plugin-contributions", get(list_plugin_contributions))
        .route(
            "/api/plugins/{plugin_id}/operations/{contribution_id}",
            post(invoke_plugin_operation),
        )
        .route(
            "/api/documents/{id}/plugins/{plugin_id}/{contribution_id}",
            post(invoke_document_action),
        )
        .route("/api/plugin-import", post(import_document))
        .route(
            "/api/documents/{id}/plugin-export/{plugin_id}/{contribution_id}",
            get(export_document),
        )
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
        .route("/api/assets", get(list_assets).post(upload_asset))
        .route("/api/assets/{name}", get(get_asset).delete(delete_asset))
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

async fn list_plugins(State(state): State<Arc<WebState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"plugins": state.plugins.list()?})))
}

#[derive(Deserialize)]
struct PluginEnabledRequest {
    enabled: bool,
}

async fn set_plugin_enabled(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
    Json(request): Json<PluginEnabledRequest>,
) -> Result<Json<Value>, ApiError> {
    state.plugins.set_enabled(&id, request.enabled)?;
    Ok(Json(json!({"id": id, "enabled": request.enabled})))
}

async fn list_plugin_contributions(
    State(state): State<Arc<WebState>>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"contributions": state.plugins.contributions()?}),
    ))
}

/// Browser clients always call DTS. The host chooses whether and how a local
/// HTTP plugin is reachable, so plugin ports and per-instance credentials never
/// leave this process.
async fn invoke_plugin_operation(
    State(state): State<Arc<WebState>>,
    Path((plugin_id, contribution_id)): Path<(String, String)>,
    Json(input): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(state.plugins.invoke_http_operation(
        &plugin_id,
        &contribution_id,
        input,
    )?))
}

async fn invoke_document_action(
    State(state): State<Arc<WebState>>,
    Path((id, plugin_id, contribution_id)): Path<(String, String, String)>,
) -> Result<Json<Document>, ApiError> {
    Ok(Json(state.plugins.invoke_document_action(
        &state.store,
        &plugin_id,
        &contribution_id,
        &id,
    )?))
}

#[derive(Deserialize)]
struct PluginImportRequest {
    plugin_id: String,
    contribution_id: String,
    id: String,
    name: String,
    template_id: String,
    text: String,
}

async fn import_document(
    State(state): State<Arc<WebState>>,
    Json(request): Json<PluginImportRequest>,
) -> Result<Json<Document>, ApiError> {
    Ok(Json(state.plugins.import_text(
        &state.store,
        &request.plugin_id,
        &request.contribution_id,
        PluginImport {
            id: request.id,
            name: request.name,
            template_id: request.template_id,
            text: request.text,
        },
    )?))
}

async fn export_document(
    State(state): State<Arc<WebState>>,
    Path((id, plugin_id, contribution_id)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let export = state
        .plugins
        .export_text(&state.store, &plugin_id, &contribution_id, &id)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_str(&export.mime_type).map_err(AppError::storage)?,
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{}\"", export.filename))
                    .map_err(AppError::storage)?,
            ),
        ],
        export.content,
    )
        .into_response())
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

async fn list_assets(State(state): State<Arc<WebState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"assets": state.store.list_assets()?})))
}

async fn upload_asset(
    State(state): State<Arc<WebState>>,
    Json(request): Json<UploadAssetRequest>,
) -> Result<Json<Value>, ApiError> {
    let bytes = assets::decode_base64(&request.name, &request.data_base64)?;
    Ok(Json(serde_json::to_value(state.store.put_asset(
        &request.name,
        &bytes,
        request.replace,
    )?)?))
}

async fn get_asset(
    State(state): State<Arc<WebState>>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let (mime_type, bytes) = state.store.get_asset(&name)?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(mime_type)),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn delete_asset(
    State(state): State<Arc<WebState>>,
    Path(name): Path<String>,
) -> Result<Json<Deleted>, ApiError> {
    state.store.delete_asset(&name)?;
    Ok(Json(Deleted {
        id: name,
        deleted: true,
    }))
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
