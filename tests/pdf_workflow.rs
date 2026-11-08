use std::process::Command;

use base64::Engine;
use document_templating_mcp::{
    model::{Document, Template, TemplateRender},
    pdf::PdfRenderer,
    store::Store,
};
use serde_json::json;
use tempfile::TempDir;

struct StubRenderer(Option<Vec<u8>>);

impl PdfRenderer for StubRenderer {
    fn render(
        &self,
        html_path: &std::path::Path,
    ) -> document_templating_mcp::error::Result<Vec<u8>> {
        assert!(html_path.is_file());
        self.0
            .clone()
            .ok_or_else(|| document_templating_mcp::error::AppError::storage("converter failed"))
    }
}

#[test]
fn adapter_failures_never_replace_the_last_valid_pdf() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .create_template(Template {
            schema_version: 1,
            id: "note".into(),
            name: "Note".into(),
            version: String::new(),
            description: String::new(),
            sections: vec![],
            defaults: json!({}),
            preview_data: json!({}),
            render: TemplateRender {
                html: "<p>hello</p>".into(),
                css: String::new(),
            },
        })
        .unwrap();
    store
        .create_document(
            Document {
                schema_version: 1,
                id: "note".into(),
                name: "Note".into(),
                template_id: "note".into(),
                data: json!({}),
            },
            None,
        )
        .unwrap();
    let bytes = b"%PDF-valid".to_vec();
    let result = store
        .render_pdf_with_renderer("note", &StubRenderer(Some(bytes.clone())))
        .unwrap();
    assert_eq!(result.resource_uri, "document://renders/note.pdf");
    assert_eq!(result.filename, "note.pdf");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(result.pdf_base64)
            .unwrap(),
        bytes
    );
    assert!(store
        .render_pdf_with_renderer("note", &StubRenderer(None))
        .is_err());
    assert!(store
        .render_pdf_with_renderer("note", &StubRenderer(Some(b"not a pdf".to_vec())))
        .is_err());
    assert_eq!(store.rendered_pdf_bytes("note").unwrap(), bytes);
}

#[test]
fn renders_a_binary_pdf_with_links_and_embedded_assets_when_wkhtmltopdf_is_available() {
    let renderer = std::env::var("DOCUMENT_TEMPLATING_MCP_WKHTMLTOPDF")
        .unwrap_or_else(|_| "wkhtmltopdf".to_string());
    if Command::new(&renderer).arg("--version").output().is_err() {
        eprintln!("skipping PDF integration test: wkhtmltopdf is not installed");
        return;
    }

    let directory = TempDir::new().unwrap();
    let store = Store::open(directory.path()).unwrap();
    store
        .create_template(Template {
            schema_version: 1,
            id: "linked-document".into(),
            name: "Linked document".into(),
            version: "1.0.0".into(),
            description: "PDF integration fixture".into(),
            sections: vec![],
            defaults: json!({}),
            preview_data: json!({}),
            render: TemplateRender {
                html: concat!(
                    "<h1>Selectable PDF text</h1>",
                    "<p><a href=\"https://example.com\">Example link</a></p>",
                    "<img width=\"20\" height=\"20\" src=\"data:image/png;base64,",
                    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=\">"
                )
                .into(),
                css: "@page { size: A4; margin: 10mm; } body { font-family: sans-serif; }".into(),
            },
        })
        .unwrap();
    store
        .create_document(
            Document {
                schema_version: 1,
                id: "pdf-check".into(),
                name: "PDF check".into(),
                template_id: "linked-document".into(),
                data: json!({}),
            },
            None,
        )
        .unwrap();

    let rendered = store.render_pdf("pdf-check").unwrap();
    assert_eq!(rendered.mime_type, "application/pdf");
    assert_eq!(rendered.filename, "pdf-check.pdf");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&rendered.pdf_base64)
        .unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(bytes, store.rendered_pdf_bytes("pdf-check").unwrap());
}
