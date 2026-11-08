use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use base64::Engine;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    assets,
    error::{AppError, Result},
    inspect,
    model::{
        AssetSummary, Document, DocumentSummary, EditorState, GalleryTemplate, PageImages,
        PdfReport, RenderedDocument, RenderedPdf, RenderedPreview, Template, TemplateGallery,
        TemplateSummary,
    },
    pdf::{self, PdfRenderer},
    raster, render,
};

pub struct Store {
    root: PathBuf,
    write_lock: Mutex<()>,
}

impl Store {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for directory in [
            root.join("templates"),
            root.join("documents"),
            root.join("renders"),
            root.join("assets"),
        ] {
            fs::create_dir_all(&directory).map_err(AppError::storage)?;
        }
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    pub fn create_template(&self, template: Template) -> Result<Template> {
        render::validate_template(&template)?;
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        let path = self.template_path(&template.id);
        if path.exists() {
            return Err(AppError::AlreadyExists(format!(
                "template '{}' already exists",
                template.id
            )));
        }
        write_json(&path, &template)?;
        Ok(template)
    }

    pub fn update_template(&self, template: Template) -> Result<Template> {
        render::validate_template(&template)?;
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        let path = self.template_path(&template.id);
        if !path.is_file() {
            return Err(not_found("template", &template.id));
        }
        write_json(&path, &template)?;
        Ok(template)
    }

    pub fn get_template(&self, id: &str) -> Result<Template> {
        render::validate_template_id(id)?;
        read_json(&self.template_path(id)).map_err(|error| match error {
            AppError::Storage(_) if !self.template_path(id).is_file() => not_found("template", id),
            other => other,
        })
    }

    pub fn list_templates(&self) -> Result<Vec<TemplateSummary>> {
        let mut templates = self.read_all::<Template>(&self.root.join("templates"))?;
        templates.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(templates.iter().map(TemplateSummary::from).collect())
    }

    pub fn template_gallery(&self) -> Result<TemplateGallery> {
        let mut templates = self.read_all::<Template>(&self.root.join("templates"))?;
        templates.sort_by(|a, b| a.id.cmp(&b.id));
        let templates = templates
            .iter()
            .map(|template| {
                let data = render::merge_defaults(&template.defaults, &template.preview_data)?;
                Ok(GalleryTemplate {
                    id: template.id.clone(),
                    name: template.name.clone(),
                    version: template.version.clone(),
                    description: template.description.clone(),
                    preview_html: self
                        .inline_assets(&render::render_preview(template, &data)?, false)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(TemplateGallery { templates })
    }

    pub fn delete_template(&self, id: &str) -> Result<()> {
        render::validate_template_id(id)?;
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        if !self.template_path(id).is_file() {
            return Err(not_found("template", id));
        }
        let documents = self.read_all::<Document>(&self.root.join("documents"))?;
        if documents.iter().any(|document| document.template_id == id) {
            return Err(AppError::InUse(format!(
                "template '{id}' is used by a document; delete or move those documents first"
            )));
        }
        fs::remove_file(self.template_path(id)).map_err(AppError::storage)
    }

    pub fn create_document(
        &self,
        mut document: Document,
        supplied_data: Option<&serde_json::Value>,
    ) -> Result<Document> {
        render::validate_document_id(&document.id)?;
        if document.schema_version != 1 {
            return Err(AppError::Invalid("schema_version must be 1".into()));
        }
        if document.name.trim().is_empty() {
            return Err(AppError::Invalid("document name is required".into()));
        }
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        let path = self.document_path(&document.id);
        if path.exists() {
            return Err(AppError::AlreadyExists(format!(
                "document '{}' already exists",
                document.id
            )));
        }
        let template = self.get_template(&document.template_id)?;
        document.data = match supplied_data {
            Some(data) => render::merge_defaults(&template.defaults, data)?,
            None => template.defaults.clone(),
        };
        render::validate_document(&template, &document.data, false)?;
        write_json(&path, &document)?;
        Ok(document)
    }

    pub fn update_document(&self, document: Document) -> Result<Document> {
        render::validate_document_id(&document.id)?;
        if document.schema_version != 1 {
            return Err(AppError::Invalid("schema_version must be 1".into()));
        }
        if document.name.trim().is_empty() {
            return Err(AppError::Invalid("document name is required".into()));
        }
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        if !self.document_path(&document.id).is_file() {
            return Err(not_found("document", &document.id));
        }
        let template = self.get_template(&document.template_id)?;
        render::validate_document(&template, &document.data, false)?;
        write_json(&self.document_path(&document.id), &document)?;
        Ok(document)
    }

    pub fn get_document(&self, id: &str) -> Result<Document> {
        render::validate_document_id(id)?;
        read_json(&self.document_path(id)).map_err(|error| match error {
            AppError::Storage(_) if !self.document_path(id).is_file() => not_found("document", id),
            other => other,
        })
    }

    pub fn editor_state(&self, id: &str) -> Result<EditorState> {
        let document = self.get_document(id)?;
        let template = self.get_template(&document.template_id)?;
        Ok(EditorState { document, template })
    }

    pub fn preview_document(
        &self,
        template_id: &str,
        supplied: &serde_json::Value,
    ) -> Result<RenderedPreview> {
        let template = self.get_template(template_id)?;
        let data = render::merge_defaults(&template.defaults, supplied)?;
        render::validate_document(&template, &data, false)?;
        Ok(RenderedPreview {
            html: self.inline_assets(&render::render_preview(&template, &data)?, false)?,
        })
    }

    pub fn list_documents(&self) -> Result<Vec<DocumentSummary>> {
        let mut documents = self.read_all::<Document>(&self.root.join("documents"))?;
        documents.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(documents.iter().map(DocumentSummary::from).collect())
    }

    pub fn delete_document(&self, id: &str) -> Result<()> {
        render::validate_document_id(id)?;
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        let path = self.document_path(id);
        if !path.is_file() {
            return Err(not_found("document", id));
        }
        fs::remove_file(path).map_err(AppError::storage)?;
        let render = self.root.join("renders").join(format!("{id}.html"));
        if render.exists() {
            fs::remove_file(render).map_err(AppError::storage)?;
        }
        let pdf = self.root.join("renders").join(format!("{id}.pdf"));
        if pdf.exists() {
            fs::remove_file(pdf).map_err(AppError::storage)?;
        }
        Ok(())
    }

    pub fn render_document(&self, id: &str) -> Result<RenderedDocument> {
        let document = self.get_document(id)?;
        let template = self.get_template(&document.template_id)?;
        let html = self.inline_assets(&render::render(&template, &document)?, true)?;
        let path = self.root.join("renders").join(format!("{id}.html"));
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        write_bytes(&path, html.as_bytes())?;
        Ok(RenderedDocument {
            document_id: document.id,
            template_id: template.id,
            output_path: path.display().to_string(),
            html,
        })
    }

    pub fn render_pdf(&self, id: &str) -> Result<RenderedPdf> {
        self.render_pdf_with_renderer(id, &pdf::ChromiumRenderer::discover())
    }

    pub fn render_pdf_with_renderer(
        &self,
        id: &str,
        renderer: &dyn PdfRenderer,
    ) -> Result<RenderedPdf> {
        let rendered = self.render_document(id)?;
        let template = self.get_template(&rendered.template_id)?;
        let html_path = self.root.join("renders").join(format!("{id}.html"));
        let pdf_path = self.root.join("renders").join(format!("{id}.pdf"));
        let bytes = renderer.render(&html_path)?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(AppError::storage("PDF renderer returned invalid PDF bytes"));
        }
        {
            let _guard = self.write_lock.lock().map_err(AppError::storage)?;
            write_bytes(&pdf_path, &bytes)?;
        }
        let report = inspect::inspect(
            &bytes,
            &pdf_path,
            &inspect::expectations(&rendered.html),
            template.page.max_pages,
        );
        Ok(RenderedPdf {
            document_id: rendered.document_id,
            template_id: rendered.template_id,
            output_path: pdf_path.display().to_string(),
            filename: format!("{id}.pdf"),
            mime_type: pdf::MIME_TYPE.into(),
            resource_uri: format!("document://renders/{id}.pdf"),
            report,
            pdf_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        })
    }

    /// Renders a fresh PDF and returns PNG images of its first pages.
    pub fn render_page_images(
        &self,
        id: &str,
        dpi: Option<u32>,
        max_pages: Option<u32>,
    ) -> Result<(PageImages, Vec<Vec<u8>>)> {
        let dpi = dpi.unwrap_or(raster::DEFAULT_DPI);
        if !(36..=raster::MAX_DPI).contains(&dpi) {
            return Err(AppError::Invalid(format!(
                "dpi must be between 36 and {}",
                raster::MAX_DPI
            )));
        }
        let limit = max_pages
            .unwrap_or(raster::MAX_PAGES)
            .clamp(1, raster::MAX_PAGES);
        let rendered = self.render_pdf(id)?;
        let report: PdfReport = rendered.report;
        let last_page = report.page_count.clamp(1, limit);
        let images = raster::page_pngs(Path::new(&rendered.output_path), dpi, last_page)?;
        Ok((
            PageImages {
                document_id: rendered.document_id,
                dpi,
                page_count: report.page_count,
                rendered_pages: images.len() as u32,
                report,
            },
            images,
        ))
    }

    pub fn put_asset(&self, name: &str, bytes: &[u8], replace: bool) -> Result<AssetSummary> {
        let mime_type = assets::validate_name(name)?;
        assets::validate_content(name, mime_type, bytes)?;
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        let path = self.asset_path(name);
        if path.exists() && !replace {
            return Err(AppError::AlreadyExists(format!(
                "asset '{name}' already exists; pass replace=true to overwrite it"
            )));
        }
        write_bytes(&path, bytes)?;
        Ok(asset_summary(name, mime_type, bytes.len() as u64))
    }

    pub fn list_assets(&self) -> Result<Vec<AssetSummary>> {
        let mut assets = Vec::new();
        for entry in fs::read_dir(self.root.join("assets")).map_err(AppError::storage)? {
            let entry = entry.map_err(AppError::storage)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(mime_type) = assets::validate_name(&name) {
                let bytes = entry.metadata().map_err(AppError::storage)?.len();
                assets.push(asset_summary(&name, mime_type, bytes));
            }
        }
        assets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(assets)
    }

    pub fn get_asset(&self, name: &str) -> Result<(&'static str, Vec<u8>)> {
        let mime_type = assets::validate_name(name)?;
        let path = self.asset_path(name);
        if !path.is_file() {
            return Err(not_found("asset", name));
        }
        Ok((mime_type, fs::read(path).map_err(AppError::storage)?))
    }

    /// Deletes an asset unless a template or document still references it.
    pub fn delete_asset(&self, name: &str) -> Result<()> {
        assets::validate_name(name)?;
        let _guard = self.write_lock.lock().map_err(AppError::storage)?;
        if !self.asset_path(name).is_file() {
            return Err(not_found("asset", name));
        }
        let mut users = Vec::new();
        for (kind, directory) in [("template", "templates"), ("document", "documents")] {
            for entry in fs::read_dir(self.root.join(directory)).map_err(AppError::storage)? {
                let path = entry.map_err(AppError::storage)?.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let text = fs::read_to_string(&path).map_err(AppError::storage)?;
                if assets::references(&text).contains(name) {
                    let id = path.file_stem().unwrap_or_default().to_string_lossy();
                    users.push(format!("{kind} '{id}'"));
                }
            }
        }
        if !users.is_empty() {
            return Err(AppError::InUse(format!(
                "asset '{name}' is used by {}",
                users.join(", ")
            )));
        }
        fs::remove_file(self.asset_path(name)).map_err(AppError::storage)
    }

    fn inline_assets(&self, html: &str, strict: bool) -> Result<String> {
        assets::inline(html, strict, |name| self.get_asset(name))
    }

    fn asset_path(&self, name: &str) -> PathBuf {
        self.root.join("assets").join(name)
    }

    pub fn rendered_pdf_bytes(&self, id: &str) -> Result<Vec<u8>> {
        render::validate_document_id(id)?;
        let path = self.root.join("renders").join(format!("{id}.pdf"));
        if !path.is_file() {
            return Err(not_found("rendered PDF", id));
        }
        fs::read(path).map_err(AppError::storage)
    }

    pub fn rendered_document_html(&self, id: &str) -> Result<String> {
        render::validate_document_id(id)?;
        let path = self.root.join("renders").join(format!("{id}.html"));
        if !path.is_file() {
            return Err(not_found("rendered document", id));
        }
        fs::read_to_string(path).map_err(AppError::storage)
    }

    fn read_all<T: DeserializeOwned>(&self, directory: &Path) -> Result<Vec<T>> {
        let mut values = Vec::new();
        for entry in fs::read_dir(directory).map_err(AppError::storage)? {
            let path = entry.map_err(AppError::storage)?.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                values.push(read_json(&path)?);
            }
        }
        Ok(values)
    }

    fn template_path(&self, id: &str) -> PathBuf {
        self.root.join("templates").join(format!("{id}.json"))
    }

    fn document_path(&self, id: &str) -> PathBuf {
        self.root.join("documents").join(format!("{id}.json"))
    }
}

fn asset_summary(name: &str, mime_type: &str, bytes: u64) -> AssetSummary {
    AssetSummary {
        name: name.into(),
        mime_type: mime_type.into(),
        bytes,
        uri: format!("{}{name}", assets::SCHEME),
    }
}

fn not_found(kind: &str, id: &str) -> AppError {
    AppError::NotFound(format!("{kind} '{id}' was not found"))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).map_err(AppError::storage)?;
    serde_json::from_str(&text)
        .map_err(|error| AppError::Storage(format!("failed to parse {}: {error}", path.display())))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(AppError::storage)?;
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes).map_err(AppError::storage)?;
    // rename replaces atomically on Unix; Windows needs the old file removed first.
    if fs::rename(&temporary, path).is_err() {
        if path.exists() {
            fs::remove_file(path).map_err(AppError::storage)?;
        }
        fs::rename(&temporary, path).map_err(AppError::storage)?;
    }
    Ok(())
}
