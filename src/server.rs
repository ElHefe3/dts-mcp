use std::{future::Future, sync::Arc};

use base64::Engine;

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, Implementation, ListResourceTemplatesResult,
        ListResourcesResult, MetaObject, PaginatedRequestParams, ProtocolVersion,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, ResourceTemplate, ServerCapabilities, ServerConfig,
    },
    schemars,
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router, ErrorData as McpError, Json, ServerHandler,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    error::AppError,
    model::{
        Deleted, Document, DocumentSummary, EditorState, RenderedPreview, Template, TemplateGallery,
    },
    store::Store,
};

const GALLERY_UI_URI: &str = "ui://document-mcp/gallery";
const EDITOR_UI_URI: &str = "ui://document-mcp/editor";
const UI_MIME_TYPE: &str = "text/html;profile=mcp-app";
const DOCUMENT_URI_PREFIX: &str = "document://renders/";
const APP_HTML: &str = include_str!("../web/dist/index.html");

fn gallery_ui_meta() -> MetaObject {
    ui_tool_meta(GALLERY_UI_URI)
}

fn editor_ui_meta() -> MetaObject {
    ui_tool_meta(EDITOR_UI_URI)
}

fn ui_tool_meta(resource_uri: &str) -> MetaObject {
    serde_json::from_value(json!({"ui": {"resourceUri": resource_uri}}))
        .expect("valid static MCP App metadata")
}

#[derive(Clone)]
pub struct DocumentMcpServer {
    store: Arc<Store>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TemplateRequest {
    pub template: Template,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IdRequest {
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateDocumentRequest {
    /// Stable lowercase identifier for the new document.
    pub id: String,
    pub name: String,
    /// ID of a previously saved template.
    pub template_id: String,
    /// Optional values deeply merged over the template defaults.
    pub data: Option<Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateDocumentRequest {
    pub document: Document,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PreviewDocumentRequest {
    /// ID of a previously saved template.
    pub template_id: String,
    /// Unsaved values deeply merged over the template defaults.
    pub data: Value,
}

#[tool_router]
impl DocumentMcpServer {
    pub fn new(store: Store) -> Self {
        Self::with_shared_store(Arc::new(store))
    }

    pub fn with_shared_store(store: Arc<Store>) -> Self {
        Self { store }
    }

    #[tool(description = "Save a new reusable document template")]
    fn create_template(
        &self,
        Parameters(request): Parameters<TemplateRequest>,
    ) -> Result<Json<Template>, McpError> {
        self.store
            .create_template(request.template)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "Show saved templates in a visual grid with mock-data previews",
        meta = gallery_ui_meta()
    )]
    fn list_templates(&self) -> Result<Json<TemplateGallery>, McpError> {
        self.store.template_gallery().map(Json).map_err(mcp_error)
    }

    #[tool(description = "Get a complete template, including fields, defaults, HTML, and CSS")]
    fn get_template(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<Json<Template>, McpError> {
        self.store
            .get_template(&request.id)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "Replace an existing reusable template; documents keep referencing its ID"
    )]
    fn update_template(
        &self,
        Parameters(request): Parameters<TemplateRequest>,
    ) -> Result<Json<Template>, McpError> {
        self.store
            .update_template(request.template)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(description = "Delete a template if no documents reference it")]
    fn delete_template(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<Json<Deleted>, McpError> {
        self.store.delete_template(&request.id).map_err(mcp_error)?;
        Ok(Json(Deleted {
            id: request.id,
            deleted: true,
        }))
    }

    #[tool(description = "Create a document from a saved template and optional initial data")]
    fn create_document(
        &self,
        Parameters(request): Parameters<CreateDocumentRequest>,
    ) -> Result<Json<Document>, McpError> {
        let document = Document {
            schema_version: 1,
            id: request.id,
            name: request.name,
            template_id: request.template_id,
            data: Value::Object(Default::default()),
        };
        self.store
            .create_document(document, request.data.as_ref())
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(description = "List saved documents and their template references")]
    fn list_documents(&self) -> Result<Json<Vec<DocumentSummary>>, McpError> {
        self.store.list_documents().map(Json).map_err(mcp_error)
    }

    #[tool(description = "Get a saved document and its structured data")]
    fn get_document(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<Json<Document>, McpError> {
        self.store
            .get_document(&request.id)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "Open an interactive document editor for a saved document",
        meta = editor_ui_meta()
    )]
    fn open_document_editor(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<Json<EditorState>, McpError> {
        self.store
            .editor_state(&request.id)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(description = "Replace a document's name, template reference, or structured data")]
    fn update_document(
        &self,
        Parameters(request): Parameters<UpdateDocumentRequest>,
    ) -> Result<Json<Document>, McpError> {
        self.store
            .update_document(request.document)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(description = "Delete a document and its rendered HTML")]
    fn delete_document(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<Json<Deleted>, McpError> {
        self.store.delete_document(&request.id).map_err(mcp_error)?;
        Ok(Json(Deleted {
            id: request.id,
            deleted: true,
        }))
    }

    #[tool(description = "Preview unsaved document data without changing the saved draft")]
    fn preview_document(
        &self,
        Parameters(request): Parameters<PreviewDocumentRequest>,
    ) -> Result<Json<RenderedPreview>, McpError> {
        self.store
            .preview_document(&request.template_id, &request.data)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "Render a saved document and return its HTML file as an embedded resource. Present the document attachment or local_path to the user."
    )]
    fn render_document(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let rendered = self.store.render_document(&request.id).map_err(mcp_error)?;
        let filename = format!("{}.html", rendered.document_id);
        let uri = format!("{DOCUMENT_URI_PREFIX}{filename}");
        let mut metadata = serde_json::to_value(&rendered)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        metadata["filename"] = json!(filename);
        metadata["mime_type"] = json!("text/html");
        metadata["resource_uri"] = json!(uri);
        let mut result = CallToolResult::structured(metadata);
        result.content.push(ContentBlock::resource(
            ResourceContents::text(rendered.html, uri).with_mime_type("text/html"),
        ));
        Ok(result)
    }

    #[tool(
        description = "Render a document as a real PDF with selectable text, assets, and clickable links"
    )]
    fn render_pdf(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let rendered = self.store.render_pdf(&request.id).map_err(mcp_error)?;
        let metadata = serde_json::to_value(&rendered)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let contents = ResourceContents::blob(rendered.pdf_base64, &rendered.resource_uri)
            .with_mime_type("application/pdf");
        let mut result = CallToolResult::structured(metadata);
        result.content.push(ContentBlock::resource(contents));
        Ok(result)
    }
}

#[tool_handler]
impl ServerHandler for DocumentMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Use list_templates to show the visual template gallery. Create a document, then use \
             open_document_editor to display its interactive editor. Drafts may be incomplete; \
             required fields are checked when rendering. render_document returns the document itself \
             as an HTML resource, not just a server path. Always present the attachment to the user; \
             if the client supplies local_path, provide a clickable link to that local file.",
        )
    }

    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        std::borrow::Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult::with_all_items(vec![
            ui_resource(
                GALLERY_UI_URI,
                "template-gallery",
                "Visual template gallery",
            ),
            ui_resource(
                EDITOR_UI_URI,
                "document-editor",
                "Interactive document editor",
            ),
        ])))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("document://renders/{id}.html", "rendered-document")
                .with_description("A saved HTML document returned by render_document")
                .with_mime_type("text/html"),
            ResourceTemplate::new("document://renders/{id}.pdf", "rendered-pdf")
                .with_description("A saved PDF returned by render_pdf")
                .with_mime_type("application/pdf"),
        ])))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, McpError>> + Send + '_ {
        if let Some(id) = request
            .uri
            .strip_prefix(DOCUMENT_URI_PREFIX)
            .and_then(|value| value.strip_suffix(".pdf"))
        {
            let result = self
                .store
                .rendered_pdf_bytes(id)
                .map_err(mcp_error)
                .map(|bytes| {
                    let contents = ResourceContents::blob(
                        base64::engine::general_purpose::STANDARD.encode(bytes),
                        &request.uri,
                    )
                    .with_mime_type("application/pdf");
                    ReadResourceResult::new(vec![contents]).into()
                });
            return std::future::ready(result);
        }
        let result = match request.uri.as_str() {
            GALLERY_UI_URI | EDITOR_UI_URI => {
                let meta = serde_json::from_value(json!({"ui": {"prefersBorder": false}}))
                    .expect("valid static MCP App resource metadata");
                let contents = ResourceContents::text(APP_HTML, &request.uri)
                    .with_mime_type(UI_MIME_TYPE)
                    .with_meta(meta);
                Ok(ReadResourceResult::new(vec![contents]).into())
            }
            uri if uri.starts_with(DOCUMENT_URI_PREFIX) && uri.ends_with(".html") => {
                let id = &uri[DOCUMENT_URI_PREFIX.len()..uri.len() - ".html".len()];
                self.store
                    .rendered_document_html(id)
                    .map(|html| {
                        ReadResourceResult::new(vec![
                            ResourceContents::text(html, uri).with_mime_type("text/html")
                        ])
                        .into()
                    })
                    .map_err(mcp_error)
            }
            _ => Err(McpError::invalid_params(
                format!("unknown resource: {}", request.uri),
                None,
            )),
        };
        std::future::ready(result)
    }
}

fn ui_resource(uri: &str, name: &str, description: &str) -> Resource {
    Resource::new(uri, name)
        .with_description(description)
        .with_mime_type(UI_MIME_TYPE)
}

fn mcp_error(error: AppError) -> McpError {
    if error.is_client_error() {
        McpError::invalid_params(error.to_string(), None)
    } else {
        McpError::internal_error(error.to_string(), None)
    }
}
