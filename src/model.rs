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
    /// Printed page settings, emitted as a CSS `@page` rule before the template CSS.
    #[serde(default)]
    pub page: PageSettings,
    /// BCP 47 language tag for `<html lang>`, used by the tagged PDF and screen readers.
    #[serde(default = "default_lang")]
    pub lang: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageSettings {
    /// Paper size: A3, A4, A5, Letter or Legal, optionally followed by `landscape`.
    #[serde(default = "default_page_size")]
    pub size: String,
    /// CSS margin shorthand with 1-4 lengths, for example `10mm` or `12mm 15mm`.
    #[serde(default = "default_page_margin")]
    pub margin: String,
    /// Optional page budget. Exceeding it produces a render warning, not an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<u32>,
}

impl Default for PageSettings {
    fn default() -> Self {
        Self {
            size: default_page_size(),
            margin: default_page_margin(),
            max_pages: None,
        }
    }
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
    /// Checks of the produced PDF against its HTML source.
    pub report: PdfReport,
    #[serde(skip)]
    pub pdf_base64: String,
}

/// Machine-checked comparison of a rendered PDF with the HTML it came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PdfReport {
    pub page_count: u32,
    /// First page width and height in millimetres.
    pub page_size_mm: Option<[f32; 2]>,
    pub max_pages: Option<u32>,
    /// True when the PDF carries a structure tree (tagged PDF).
    pub tagged: bool,
    pub title: Option<String>,
    /// Link targets found as PDF link annotations.
    pub links: Vec<String>,
    /// Links present in the HTML but missing from the PDF.
    pub missing_links: Vec<String>,
    /// Distinct raster images referenced by `<img>` in the HTML.
    pub images_expected: usize,
    /// Distinct raster images embedded in the PDF.
    pub images_found: usize,
    pub text_check: TextCheck,
    /// Human-readable problems an agent or person should act on. Empty means all checks passed.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TextCheck {
    /// `passed`, `failed` or `skipped` (no text extractor available).
    pub status: String,
    /// Words in the HTML text that are absent from the PDF text layer (first 25).
    pub missing_words: Vec<String>,
    /// Characters in the PDF text layer that did not map to Unicode.
    pub unmapped_characters: usize,
    pub extracted_characters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssetSummary {
    pub name: String,
    pub mime_type: String,
    pub bytes: u64,
    /// Reference to use in template HTML/CSS or document data, for example `asset://photo.png`.
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PageImages {
    pub document_id: String,
    pub dpi: u32,
    pub page_count: u32,
    pub rendered_pages: u32,
    pub report: PdfReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Deleted {
    pub id: String,
    pub deleted: bool,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn default_lang() -> String {
    "en".into()
}

fn default_page_size() -> String {
    "A4".into()
}

fn default_page_margin() -> String {
    "10mm".into()
}
