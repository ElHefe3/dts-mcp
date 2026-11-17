use std::collections::HashSet;

use handlebars::Handlebars;
use serde::Serialize;
use serde_json::Value;

use crate::{
    error::{AppError, Result},
    model::{Document, FieldDefinition, FieldType, PageSettings, Template},
};

/// Limits are enforced before persistence and before returning rendered HTML.
pub const MAX_TEMPLATE_BYTES: usize = 256 * 1024;
pub const MAX_DOCUMENT_DATA_BYTES: usize = 256 * 1024;
pub const MAX_TEMPLATE_SECTIONS: usize = 32;
pub const MAX_TEMPLATE_FIELDS: usize = 128;
pub const MAX_LIST_ITEMS: usize = 100;
pub const MAX_RENDERED_HTML_BYTES: usize = 1024 * 1024;

pub fn render(template: &Template, document: &Document) -> Result<String> {
    validate_document(template, &document.data, true)?;
    render_html(template, &document.data, &document.name)
}

pub fn render_preview(template: &Template, data: &Value) -> Result<String> {
    render_html(template, data, &template.name)
}

/// Produces one self-describing HTML file that renders identically on screen and in print:
/// page geometry comes from `@page`, and print keeps backgrounds and colours exactly.
fn render_html(template: &Template, data: &Value, title: &str) -> Result<String> {
    let mut context = data.as_object().cloned().unwrap_or_default();
    context.insert("document".to_string(), data.clone());
    let handlebars = Handlebars::new();
    let body = handlebars
        .render_template(&template.render.html, &Value::Object(context))
        .map_err(|error| AppError::Invalid(format!("template rendering failed: {error}")))?;
    let html = format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<style>\n{page}\n\
         html {{ -webkit-print-color-adjust: exact; print-color-adjust: exact; }}\n</style>\n\
         <style>\n{css}\n</style>\n</head>\n<body>\n{body}\n</body>\n</html>\n",
        lang = escape_html(&template.lang),
        title = escape_html(title),
        page = page_css(&template.page),
        css = template.render.css,
    );
    if html.len() > MAX_RENDERED_HTML_BYTES {
        return Err(AppError::Invalid(format!(
            "rendered HTML is {} bytes; the limit is {MAX_RENDERED_HTML_BYTES}",
            html.len()
        )));
    }
    Ok(html)
}

pub fn page_css(page: &PageSettings) -> String {
    format!("@page {{ size: {}; margin: {}; }}", page.size, page.margin)
}

pub fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn validate_page(page: &PageSettings) -> Result<()> {
    let mut parts = page.size.split_whitespace();
    let size_ok = matches!(
        parts.next().map(str::to_ascii_lowercase).as_deref(),
        Some("a3" | "a4" | "a5" | "letter" | "legal")
    ) && matches!(parts.next(), None | Some("portrait" | "landscape"))
        && parts.next().is_none();
    if !size_ok {
        return Err(AppError::Invalid(format!(
            "page.size must be A3, A4, A5, Letter or Legal, optionally with portrait/landscape; got '{}'",
            page.size
        )));
    }
    let lengths: Vec<&str> = page.margin.split_whitespace().collect();
    let length_ok = |value: &&str| {
        if *value == "0" {
            return true;
        }
        ["mm", "cm", "in", "pt", "px"].iter().any(|unit| {
            value.strip_suffix(unit).is_some_and(|number| {
                !number.is_empty()
                    && number
                        .parse::<f32>()
                        .is_ok_and(|n| (0.0..=200.0).contains(&n))
            })
        })
    };
    if lengths.is_empty() || lengths.len() > 4 || !lengths.iter().all(length_ok) {
        return Err(AppError::Invalid(format!(
            "page.margin must be 1-4 lengths in mm, cm, in, pt or px; got '{}'",
            page.margin
        )));
    }
    if page.max_pages == Some(0) {
        return Err(AppError::Invalid(
            "page.max_pages must be at least 1".into(),
        ));
    }
    Ok(())
}

fn validate_lang(lang: &str) -> Result<()> {
    let valid = (2..=35).contains(&lang.len())
        && lang
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()));
    if valid {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "lang must be a language tag such as 'en' or 'de-DE'; got '{lang}'"
        )))
    }
}

pub fn validate_template(template: &Template) -> Result<()> {
    validate_serialized_size(template, MAX_TEMPLATE_BYTES, "template")?;
    validate_id(&template.id, "template")?;
    if template.schema_version != 1 {
        return Err(AppError::Invalid("schema_version must be 1".into()));
    }
    if template.name.trim().is_empty() {
        return Err(AppError::Invalid("template name is required".into()));
    }
    if !template.defaults.is_object() {
        return Err(AppError::Invalid(
            "template defaults must be a JSON object".into(),
        ));
    }
    if !template.preview_data.is_object() {
        return Err(AppError::Invalid(
            "template preview_data must be a JSON object".into(),
        ));
    }
    if template.render.html.trim().is_empty() {
        return Err(AppError::Invalid("template render.html is required".into()));
    }
    if template.sections.len() > MAX_TEMPLATE_SECTIONS {
        return Err(AppError::Invalid(format!(
            "template has {} sections; the limit is {MAX_TEMPLATE_SECTIONS}",
            template.sections.len()
        )));
    }
    validate_page(&template.page)?;
    validate_lang(&template.lang)?;
    let preview_data = merge_defaults(&template.defaults, &template.preview_data)?;
    validate_document(template, &preview_data, false)?;
    Handlebars::new()
        .render_template(&template.render.html, &template.defaults)
        .map_err(|error| AppError::Invalid(format!("invalid Handlebars template: {error}")))?;
    let mut section_ids = HashSet::new();
    let mut field_paths = HashSet::new();
    let mut field_count = 0;
    for section in &template.sections {
        if section.id.trim().is_empty() {
            return Err(AppError::Invalid("template section id is required".into()));
        }
        if !section_ids.insert(&section.id) {
            return Err(AppError::Invalid(format!(
                "duplicate template section id: {}",
                section.id
            )));
        }
        validate_field_definitions(
            &section.fields,
            "",
            &section.id,
            &mut field_paths,
            &mut field_count,
        )?;
        for field in &section.fields {
            validate_field(&template.defaults, field, false)?;
        }
    }
    Ok(())
}

pub fn validate_document(template: &Template, data: &Value, required: bool) -> Result<()> {
    if !data.is_object() {
        return Err(AppError::Invalid(
            "document data must be a JSON object".into(),
        ));
    }
    validate_serialized_size(data, MAX_DOCUMENT_DATA_BYTES, "document data")?;
    for section in &template.sections {
        for field in &section.fields {
            validate_field(data, field, required)?;
        }
    }
    Ok(())
}

pub fn merge_defaults(defaults: &Value, supplied: &Value) -> Result<Value> {
    if !supplied.is_object() {
        return Err(AppError::Invalid(
            "document data must be a JSON object".into(),
        ));
    }
    let mut merged = defaults.clone();
    merge_object(&mut merged, supplied);
    Ok(merged)
}

fn merge_object(base: &mut Value, overlay: &Value) {
    if let (Some(base), Some(overlay)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay {
            match base.get_mut(key) {
                Some(existing) if existing.is_object() && value.is_object() => {
                    merge_object(existing, value);
                }
                _ => {
                    base.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

pub fn validate_template_id(id: &str) -> Result<()> {
    validate_id(id, "template")
}

pub fn validate_document_id(id: &str) -> Result<()> {
    validate_id(id, "document")
}

fn validate_field_definitions(
    fields: &[FieldDefinition],
    parent_path: &str,
    section_id: &str,
    paths: &mut HashSet<String>,
    field_count: &mut usize,
) -> Result<()> {
    let mut field_ids = HashSet::new();
    for field in fields {
        *field_count += 1;
        if *field_count > MAX_TEMPLATE_FIELDS {
            return Err(AppError::Invalid(format!(
                "template has more than {MAX_TEMPLATE_FIELDS} fields"
            )));
        }
        if !field_ids.insert(&field.id) {
            return Err(AppError::Invalid(format!(
                "duplicate field id '{}' in section '{section_id}'",
                field.id
            )));
        }
        validate_field_definition(field)?;
        let full_path = if parent_path.is_empty() {
            field.path.clone()
        } else {
            format!("{parent_path}.{}", field.path)
        };
        if !paths.insert(full_path.clone()) {
            return Err(AppError::Invalid(format!(
                "duplicate field path: {full_path}"
            )));
        }
        validate_field_definitions(&field.fields, &full_path, section_id, paths, field_count)?;
    }
    Ok(())
}

fn validate_field_definition(field: &FieldDefinition) -> Result<()> {
    if field.id.trim().is_empty() || field.path.trim().is_empty() {
        return Err(AppError::Invalid("field id and path are required".into()));
    }
    if field.path.split('.').any(str::is_empty) {
        return Err(AppError::Invalid(format!(
            "invalid field path: {}",
            field.path
        )));
    }
    if field.field_type == FieldType::ObjectList && field.fields.is_empty() {
        return Err(AppError::Invalid(format!(
            "object_list field {} must define child fields",
            field.path
        )));
    }
    if field.field_type != FieldType::ObjectList && !field.fields.is_empty() {
        return Err(AppError::Invalid(format!(
            "{} may define nested fields only when type is object_list",
            field.path
        )));
    }
    Ok(())
}

fn validate_field(data: &Value, field: &FieldDefinition, required: bool) -> Result<()> {
    let value = get_path(data, &field.path);
    if required && field.required && value.map(is_blank).unwrap_or(true) {
        return Err(AppError::Invalid(format!("{} is required", field.path)));
    }
    let Some(value) = value else {
        return Ok(());
    };
    match field.field_type {
        FieldType::Text | FieldType::Textarea | FieldType::RichText | FieldType::Url
            if !value.is_string() =>
        {
            return Err(AppError::Invalid(format!(
                "{} must be a string",
                field.path
            )));
        }
        FieldType::StringList => {
            let items = value.as_array().ok_or_else(|| {
                AppError::Invalid(format!("{} must be an array of strings", field.path))
            })?;
            validate_list_length(&field.path, items.len())?;
            if items.iter().any(|item| !item.is_string()) {
                return Err(AppError::Invalid(format!(
                    "{} must contain only strings",
                    field.path
                )));
            }
        }
        FieldType::ObjectList => {
            let items = value.as_array().ok_or_else(|| {
                AppError::Invalid(format!("{} must be an array of objects", field.path))
            })?;
            validate_list_length(&field.path, items.len())?;
            for (index, item) in items.iter().enumerate() {
                if !item.is_object() {
                    return Err(AppError::Invalid(format!(
                        "{}.{} must be an object",
                        field.path, index
                    )));
                }
                for child in &field.fields {
                    let mut nested = child.clone();
                    nested.path = format!("{}.{}.{}", field.path, index, child.path);
                    validate_field(data, &nested, required)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_list_length(path: &str, length: usize) -> Result<()> {
    if length > MAX_LIST_ITEMS {
        return Err(AppError::Invalid(format!(
            "{path} has {length} items; the limit is {MAX_LIST_ITEMS}"
        )));
    }
    Ok(())
}

fn validate_serialized_size(value: impl Serialize, max_bytes: usize, label: &str) -> Result<()> {
    let bytes = serde_json::to_vec(&value).map_err(AppError::storage)?;
    if bytes.len() > max_bytes {
        return Err(AppError::Invalid(format!(
            "{label} is {} bytes; the limit is {max_bytes}",
            bytes.len()
        )));
    }
    Ok(())
}

fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(items) => items.is_empty(),
        _ => false,
    }
}

fn validate_id(id: &str, kind: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if !valid {
        return Err(AppError::Invalid(format!(
            "{kind} id must be 1-64 lowercase letters, digits, or hyphens"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::{TemplateRender, TemplateSection};

    fn field(id: &str, path: &str, field_type: FieldType) -> FieldDefinition {
        FieldDefinition {
            id: id.into(),
            path: path.into(),
            label: id.into(),
            field_type,
            required: false,
            fields: vec![],
        }
    }

    fn template() -> Template {
        Template {
            schema_version: 1,
            id: "validation".into(),
            name: "Validation".into(),
            version: "1.0.0".into(),
            description: String::new(),
            sections: vec![TemplateSection {
                id: "content".into(),
                label: "Content".into(),
                description: String::new(),
                fields: vec![field("text", "text", FieldType::Text)],
            }],
            defaults: json!({"text": ""}),
            preview_data: json!({}),
            render: TemplateRender {
                html: "<p>{{text}}</p>".into(),
                css: String::new(),
            },
            page: Default::default(),
            lang: "en".into(),
        }
    }

    #[test]
    fn rejects_duplicate_and_invalid_field_definitions() {
        let mut duplicate_section = template();
        duplicate_section
            .sections
            .push(duplicate_section.sections[0].clone());
        assert!(validate_template(&duplicate_section)
            .unwrap_err()
            .to_string()
            .contains("duplicate template section id: content"));

        let mut duplicate_field = template();
        duplicate_field.sections[0]
            .fields
            .push(field("text", "other", FieldType::Text));
        assert!(validate_template(&duplicate_field)
            .unwrap_err()
            .to_string()
            .contains("duplicate field id 'text' in section 'content'"));

        let mut invalid_nested = template();
        invalid_nested.sections[0].fields[0]
            .fields
            .push(field("child", "child", FieldType::Text));
        assert!(validate_template(&invalid_nested)
            .unwrap_err()
            .to_string()
            .contains("may define nested fields only when type is object_list"));
    }

    #[test]
    fn keeps_drafts_nested_fields_and_handlebars_scopes_compatible() {
        let mut nested = field("positions", "positions", FieldType::ObjectList);
        nested.required = true;
        let mut company = field("company", "company", FieldType::Text);
        company.required = true;
        nested.fields.push(company);
        let mut valid = template();
        valid.sections[0].fields = vec![nested];
        valid.defaults = json!({"positions": []});
        valid.preview_data = json!({});
        valid.render.html =
            "{{#if profile.enabled}}{{#each profile.tags}}{{this}}{{/each}}{{/if}}{{unknown}}"
                .into();
        assert!(validate_template(&valid).is_ok());
        assert!(validate_document(&valid, &json!({"positions": []}), false).is_ok());
        assert!(validate_document(&valid, &json!({"positions": []}), true)
            .unwrap_err()
            .to_string()
            .contains("positions is required"));
    }

    #[test]
    fn bounds_template_document_lists_and_rendered_html() {
        let mut oversized_template = template();
        oversized_template.description = "x".repeat(MAX_TEMPLATE_BYTES);
        assert!(validate_template(&oversized_template)
            .unwrap_err()
            .to_string()
            .contains("template is"));

        let mut list_template = template();
        list_template.sections[0].fields = vec![field("items", "items", FieldType::StringList)];
        list_template.defaults = json!({"items": []});
        list_template.render.html =
            "{{#each items}}{{this}}{{this}}{{this}}{{this}}{{this}}{{this}}{{/each}}".into();
        validate_template(&list_template).unwrap();
        let too_many = json!({"items": vec!["x"; MAX_LIST_ITEMS + 1]});
        assert!(validate_document(&list_template, &too_many, false)
            .unwrap_err()
            .to_string()
            .contains("items has 101 items; the limit is 100"));
        let oversized_data = json!({"items": [], "extra": "x".repeat(MAX_DOCUMENT_DATA_BYTES)});
        assert!(validate_document(&list_template, &oversized_data, false)
            .unwrap_err()
            .to_string()
            .contains("document data is"));

        let data = json!({"items": vec!["x".repeat(2000); MAX_LIST_ITEMS]});
        assert!(validate_document(&list_template, &data, false).is_ok());
        assert!(render_preview(&list_template, &data)
            .unwrap_err()
            .to_string()
            .contains("rendered HTML is"));
    }

    #[test]
    fn escapes_normal_values_and_allows_explicitly_trusted_rich_text() {
        let mut rich = template();
        rich.sections[0].fields = vec![
            field("text", "text", FieldType::Text),
            field("rich", "rich", FieldType::RichText),
        ];
        rich.defaults = json!({"text": "", "rich": ""});
        rich.render.html = "<p>{{text}}</p>{{{rich}}}".into();
        validate_template(&rich).unwrap();
        let html = render_preview(
            &rich,
            &json!({"text": "<Ada>", "rich": "<strong>trusted</strong>"}),
        )
        .unwrap();
        assert!(html.contains("&lt;Ada&gt;"));
        assert!(html.contains("<strong>trusted</strong>"));
    }
}
