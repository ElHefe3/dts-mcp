use handlebars::Handlebars;
use serde_json::Value;

use crate::{
    error::{AppError, Result},
    model::{Document, FieldDefinition, FieldType, PageSettings, Template},
};

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
    Ok(format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<style>\n{page}\n\
         html {{ -webkit-print-color-adjust: exact; print-color-adjust: exact; }}\n</style>\n\
         <style>\n{css}\n</style>\n</head>\n<body>\n{body}\n</body>\n</html>\n",
        lang = escape_html(&template.lang),
        title = escape_html(title),
        page = page_css(&template.page),
        css = template.render.css,
    ))
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
    validate_page(&template.page)?;
    validate_lang(&template.lang)?;
    let preview_data = merge_defaults(&template.defaults, &template.preview_data)?;
    validate_document(template, &preview_data, false)?;
    Handlebars::new()
        .render_template(&template.render.html, &template.defaults)
        .map_err(|error| AppError::Invalid(format!("invalid Handlebars template: {error}")))?;
    for section in &template.sections {
        for field in &section.fields {
            validate_field_definition(field)?;
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
    for child in &field.fields {
        validate_field_definition(child)?;
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
