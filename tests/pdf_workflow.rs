use base64::Engine;
use document_templating_mcp::{
    error::AppError,
    model::{Document, PageSettings, Template, TemplateRender},
    pdf::{ChromiumRenderer, PdfRenderer},
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
            page: Default::default(),
            lang: "en".into(),
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

const PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn fidelity_store(directory: &TempDir, max_pages: Option<u32>) -> Store {
    let store = Store::open(directory.path()).unwrap();
    store
        .put_asset(
            "dot.png",
            &base64::engine::general_purpose::STANDARD
                .decode(PNG_BASE64)
                .unwrap(),
            false,
        )
        .unwrap();
    store
        .create_template(Template {
            schema_version: 1,
            id: "fidelity".into(),
            name: "Fidelity".into(),
            version: "1.0.0".into(),
            description: "PDF fidelity fixture".into(),
            sections: vec![],
            defaults: json!({"name": "José Müller-Ødegård", "dates": "2021 – 2024"}),
            preview_data: json!({}),
            render: TemplateRender {
                html: concat!(
                    "<h1>{{name}}</h1>",
                    "<p class=\"dates\">{{dates}} “quoted” profile office</p>",
                    "<div class=\"grid\"><div>LEFTCELL</div><div>RIGHTCELL</div></div>",
                    "<p><a href=\"https://example.com/portfolio\">Portfolio</a> ",
                    "<a href=\"mailto:jose@example.com\">jose@example.com</a></p>",
                    "<img width=\"40\" height=\"40\" alt=\"Dot\" src=\"asset://dot.png\">",
                    "{{#each filler}}<p>{{this}}</p>{{/each}}"
                )
                .into(),
                css: ".grid { display: grid; grid-template-columns: 1fr 1fr; } body { font-family: sans-serif; }".into(),
            },
            page: PageSettings {
                max_pages,
                ..PageSettings::default()
            },
            lang: "de".into(),
        })
        .unwrap();
    store
}

fn document(id: &str, data: serde_json::Value) -> Document {
    Document {
        schema_version: 1,
        id: id.into(),
        name: "Fidelity check".into(),
        template_id: "fidelity".into(),
        data,
    }
}

/// Covers the three PDF criteria: machine-readable text, layout identical to the HTML
/// engine (CSS grid, A4 from `@page`), and working links and images.
#[test]
fn chromium_pdf_is_machine_readable_and_complete() {
    if !ChromiumRenderer::discover().is_available() {
        eprintln!("skipping PDF integration test: Chromium is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    let store = fidelity_store(&directory, Some(1));
    store
        .create_document(document("pdf-check", json!({})), None)
        .unwrap();

    let rendered = store.render_pdf("pdf-check").unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&rendered.pdf_base64)
        .unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(bytes, store.rendered_pdf_bytes("pdf-check").unwrap());

    let report = &rendered.report;
    assert_eq!(report.page_count, 1, "{report:?}");
    let [width, height] = report.page_size_mm.unwrap();
    assert!((width - 210.0).abs() < 1.0 && (height - 297.0).abs() < 1.0);
    assert!(report.tagged);
    assert_eq!(report.title.as_deref(), Some("Fidelity check"));
    assert!(report
        .links
        .iter()
        .any(|l| l == "https://example.com/portfolio"));
    assert!(report.links.iter().any(|l| l == "mailto:jose@example.com"));
    assert!(report.missing_links.is_empty());
    assert_eq!((report.images_expected, report.images_found), (1, 1));
    if report.text_check.status == "skipped" {
        eprintln!("text-layer check skipped: pdftotext is not installed");
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    } else {
        assert_eq!(report.text_check.status, "passed", "{report:?}");
        assert_eq!(report.text_check.unmapped_characters, 0);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let text = document_templating_mcp::inspect::extract_text(std::path::Path::new(
            &rendered.output_path,
        ))
        .unwrap();
        assert!(text.contains("José Müller-Ødegård"), "{text}");
        assert!(text.contains("2021 – 2024"), "{text}");
        // CSS grid places both cells on one line, as the browser does.
        assert!(
            text.lines()
                .any(|line| line.contains("LEFTCELL") && line.contains("RIGHTCELL"))
                || text.contains("LEFTCELL RIGHTCELL"),
            "{text}"
        );
    }
}

#[test]
fn reports_page_budget_overflow() {
    if !ChromiumRenderer::discover().is_available() {
        eprintln!("skipping PDF integration test: Chromium is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    let store = fidelity_store(&directory, Some(1));
    let filler: Vec<String> = (0..120)
        .map(|i| format!("Paragraph {i} pushes the document onto another page."))
        .collect();
    store
        .create_document(
            document("long", json!({})),
            Some(&json!({"filler": filler})),
        )
        .unwrap();
    let report = store.render_pdf("long").unwrap().report;
    assert!(report.page_count > 1);
    assert!(report.warnings.iter().any(|w| w.contains("at most 1")));
}

#[test]
fn assets_are_inlined_and_protected_while_referenced() {
    let directory = TempDir::new().unwrap();
    let store = fidelity_store(&directory, None);
    store
        .create_document(document("uses-dot", json!({})), None)
        .unwrap();
    let html = store.render_document("uses-dot").unwrap().html;
    assert!(!html.contains("asset://"));
    assert!(html.contains("src=\"data:image/png;base64,"));
    assert!(html.contains("<html lang=\"de\">"));
    assert!(html.contains("<title>Fidelity check</title>"));
    assert!(html.contains("@page { size: A4; margin: 10mm; }"));
    assert!(matches!(
        store.delete_asset("dot.png"),
        Err(AppError::InUse(_))
    ));
    assert!(matches!(
        store.put_asset("dot.png", b"\x89PNG\r\n\x1a\nnew", false),
        Err(AppError::AlreadyExists(_))
    ));
    assert!(store.put_asset("bad.png", b"GIF89a", false).is_err());
    store
        .update_template({
            let mut template = store.get_template("fidelity").unwrap();
            template.render.html = "<img src=\"asset://missing.png\">".into();
            template
        })
        .unwrap();
    assert!(store
        .render_document("uses-dot")
        .unwrap_err()
        .to_string()
        .contains("upload_asset"));
    store.delete_asset("dot.png").unwrap();
    assert!(store.list_assets().unwrap().is_empty());
}
