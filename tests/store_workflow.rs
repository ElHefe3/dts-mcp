use document_templating_mcp::{
    error::AppError,
    model::{Document, FieldDefinition, FieldType, Template, TemplateRender, TemplateSection},
    store::Store,
};
use serde_json::json;
use tempfile::TempDir;

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
        preview_data: json!({"recipient": "Example recipient"}),
        render: TemplateRender {
            html: "<h1>{{recipient}}</h1><p>{{message}}</p>".into(),
            css: "h1 { color: navy; }".into(),
        },
    }
}

fn document(id: &str, template_id: &str) -> Document {
    Document {
        schema_version: 1,
        id: id.into(),
        name: format!("Document {id}"),
        template_id: template_id.into(),
        data: json!({}),
    }
}

#[test]
fn templates_persist_and_can_be_reused_after_restart() {
    let directory = TempDir::new().unwrap();
    {
        let store = Store::open(directory.path()).unwrap();
        store.create_template(template()).unwrap();
        store
            .create_document(
                document("first", "letter"),
                Some(&json!({"recipient": "Ada"})),
            )
            .unwrap();
        store
            .create_document(
                document("second", "letter"),
                Some(&json!({"recipient": "Grace"})),
            )
            .unwrap();
    }

    let store = Store::open(directory.path()).unwrap();
    assert_eq!(store.list_templates().unwrap().len(), 1);
    assert_eq!(store.list_documents().unwrap().len(), 2);
    let rendered = store.render_document("first").unwrap();
    assert!(rendered.html.contains("<h1>Ada</h1>"));
    assert!(rendered.html.contains("color: navy"));
    assert!(directory.path().join("renders/first.html").is_file());
}

#[test]
fn updated_templates_apply_to_existing_documents() {
    let directory = TempDir::new().unwrap();
    let store = Store::open(directory.path()).unwrap();
    store.create_template(template()).unwrap();
    store
        .create_document(
            document("first", "letter"),
            Some(&json!({"recipient": "Ada"})),
        )
        .unwrap();
    let mut updated = template();
    updated.version = "2.0.0".into();
    updated.render.html = "<h2>{{recipient}}: {{message}}</h2>".into();
    store.update_template(updated).unwrap();

    let rendered = store.render_document("first").unwrap();
    assert!(rendered.html.contains("<h2>Ada: Hello</h2>"));
}

#[test]
fn incomplete_drafts_fail_only_when_rendered() {
    let directory = TempDir::new().unwrap();
    let store = Store::open(directory.path()).unwrap();
    store.create_template(template()).unwrap();
    store
        .create_document(document("draft", "letter"), None)
        .unwrap();
    let error = store.render_document("draft").unwrap_err();
    assert!(matches!(error, AppError::Invalid(_)));
    assert!(error.to_string().contains("recipient is required"));
}

#[test]
fn referenced_templates_cannot_be_deleted() {
    let directory = TempDir::new().unwrap();
    let store = Store::open(directory.path()).unwrap();
    store.create_template(template()).unwrap();
    store
        .create_document(document("draft", "letter"), None)
        .unwrap();
    assert!(matches!(
        store.delete_template("letter"),
        Err(AppError::InUse(_))
    ));
    store.delete_document("draft").unwrap();
    store.delete_template("letter").unwrap();
    assert!(store.list_templates().unwrap().is_empty());
}

#[test]
fn handlebars_escapes_document_content() {
    let directory = TempDir::new().unwrap();
    let store = Store::open(directory.path()).unwrap();
    store.create_template(template()).unwrap();
    store
        .create_document(
            document("safe", "letter"),
            Some(&json!({"recipient": "<Ada>"})),
        )
        .unwrap();
    let rendered = store.render_document("safe").unwrap();
    assert!(rendered.html.contains("&lt;Ada&gt;"));
    assert!(!rendered.html.contains("<h1><Ada></h1>"));
}
