use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Template {
    pub schema_version: u32,
    /// Stable lowercase identifier, for example `cover-letter`.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sections: Vec<TemplateSection>,
    /// Initial JSON data copied into new documents.
    pub defaults: Value,
    /// Mock data merged over defaults for visual template previews.
    #[serde(default = "empty_object")]
    pub preview_data: Value,
    pub render: TemplateRender,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemplateSection {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fields: Vec<FieldDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FieldDefinition {
    pub id: String,
    /// Dot-separated path in the document JSON, for example `profile.name`.
    pub path: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDefinition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Textarea,
    RichText,
    Url,
    StringList,
    ObjectList,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemplateRender {
    /// Handlebars HTML body. Document values are available directly and under `document`.
    pub html: String,
    #[serde(default)]
    pub css: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
}

impl From<&Template> for TemplateSummary {
    fn from(template: &Template) -> Self {
        Self {
            id: template.id.clone(),
            name: template.name.clone(),
            version: template.version.clone(),
            description: template.description.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GalleryTemplate {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub preview_html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemplateGallery {
    pub templates: Vec<GalleryTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditorState {
    pub document: Document,
    pub template: Template,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderedPreview {
    pub html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Document {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub template_id: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentSummary {
    pub id: String,
    pub name: String,
    pub template_id: String,
}

impl From<&Document> for DocumentSummary {
    fn from(document: &Document) -> Self {
        Self {
            id: document.id.clone(),
            name: document.name.clone(),
            template_id: document.template_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderedDocument {
    pub document_id: String,
    pub template_id: String,
    pub output_path: String,
    pub html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderedPdf {
    pub document_id: String,
    pub template_id: String,
    pub output_path: String,
    pub filename: String,
    pub mime_type: String,
    pub resource_uri: String,
    #[serde(skip)]
    pub pdf_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Deleted {
    pub id: String,
    pub deleted: bool,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}
