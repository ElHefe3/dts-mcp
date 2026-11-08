//! Checks a rendered PDF against the HTML it came from: page budget, text layer, links,
//! images, tagging and title. The result tells an agent (or a person) whether the PDF is
//! machine-readable and complete without having to open it.

use std::{
    collections::{BTreeSet, HashSet},
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::{
    model::{PdfReport, TextCheck},
    pdf::{run_with_timeout, which},
};

pub const PDFTOTEXT_ENV: &str = "DOCUMENT_TEMPLATING_MCP_PDFTOTEXT";
const MAX_LISTED: usize = 25;

/// What a faithful PDF of an HTML document must contain.
#[derive(Debug, Default)]
pub struct Expectations {
    pub words: BTreeSet<String>,
    pub links: BTreeSet<String>,
    pub raster_images: usize,
}

pub fn expectations(html: &str) -> Expectations {
    let body = body_of(html);
    let without_blocks = remove_elements(&body, &["style", "script", "template", "noscript"]);
    let without_comments = remove_between(&without_blocks, "<!--", "-->");
    let mut links = BTreeSet::new();
    let mut images = BTreeSet::new();
    let mut text = String::with_capacity(without_comments.len());
    let mut rest = without_comments.as_str();
    while let Some(start) = rest.find('<') {
        text.push_str(&rest[..start]);
        text.push(' ');
        let end = rest[start..]
            .find('>')
            .map_or(rest.len(), |i| start + i + 1);
        let tag = &rest[start..end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<a ") || lower.starts_with("<a\n") || lower.starts_with("<a\t") {
            if let Some(href) = attribute(tag, "href") {
                if ["http:", "https:", "mailto:", "tel:"]
                    .iter()
                    .any(|scheme| href.to_ascii_lowercase().starts_with(scheme))
                {
                    links.insert(href);
                }
            }
        } else if lower.starts_with("<img") {
            if let Some(src) = attribute(tag, "src") {
                if !src.starts_with("data:image/svg") && !src.to_ascii_lowercase().ends_with(".svg")
                {
                    images.insert(src);
                }
            }
        }
        rest = &rest[end..];
    }
    text.push_str(rest);
    Expectations {
        words: words(&decode_entities(&text)),
        links,
        raster_images: images.len(),
    }
}

/// Inspects PDF bytes. `pdf_path` is the saved copy, used by the external text extractor.
pub fn inspect(
    bytes: &[u8],
    pdf_path: &Path,
    expected: &Expectations,
    max_pages: Option<u32>,
) -> PdfReport {
    let mut report = PdfReport {
        max_pages,
        ..PdfReport::default()
    };
    match Document::load_mem(bytes) {
        Ok(document) => structure(&document, &mut report),
        Err(error) => report
            .warnings
            .push(format!("PDF structure could not be parsed: {error}")),
    }
    let found: HashSet<String> = report
        .links
        .iter()
        .map(|link| normalize_link(link))
        .collect();
    report.missing_links = expected
        .links
        .iter()
        .filter(|link| !found.contains(&normalize_link(link)))
        .cloned()
        .collect();
    report.images_expected = expected.raster_images;
    report.text_check = text_check(pdf_path, &expected.words);
    warnings(&mut report);
    report
}

fn structure(document: &Document, report: &mut PdfReport) {
    let pages = document.get_pages();
    report.page_count = pages.len() as u32;
    if let Some(first) = pages.values().next() {
        report.page_size_mm = media_box(document, *first);
    }
    let mut images = HashSet::new();
    for page_id in pages.values() {
        if let Ok(annotations) = document.get_page_annotations(*page_id) {
            for annotation in annotations {
                if let Some(uri) = link_uri(document, annotation) {
                    if !report.links.contains(&uri) {
                        report.links.push(uri);
                    }
                }
            }
        }
        if let Ok((direct, referenced)) = document.get_page_resources(*page_id) {
            let mut seen = HashSet::new();
            if let Some(resources) = direct {
                collect_images(document, resources, &mut images, &mut seen, 0);
            }
            for id in referenced {
                if let Ok(resources) = document.get_dictionary(id) {
                    collect_images(document, resources, &mut images, &mut seen, 0);
                }
            }
        }
    }
    report.images_found = images.len();
    if let Ok(catalog) = document.catalog() {
        report.tagged = catalog.has(b"StructTreeRoot");
    }
    report.title = document
        .trailer
        .get_deref(b"Info", document)
        .and_then(Object::as_dict)
        .and_then(|info| info.get_deref(b"Title", document))
        .ok()
        .and_then(|title| lopdf::decode_text_string(title).ok())
        .filter(|title| !title.trim().is_empty());
}

fn media_box(document: &Document, page_id: ObjectId) -> Option<[f32; 2]> {
    let page = document.get_dictionary(page_id).ok()?;
    let values = page
        .get_deref(b"MediaBox", document)
        .ok()?
        .as_array()
        .ok()?;
    let numbers: Vec<f32> = values.iter().filter_map(|v| v.as_float().ok()).collect();
    let [x0, y0, x1, y1] = numbers.as_slice() else {
        return None;
    };
    let to_mm = |points: f32| (points * 25.4 / 72.0 * 10.0).round() / 10.0;
    Some([to_mm((x1 - x0).abs()), to_mm((y1 - y0).abs())])
}

fn link_uri(document: &Document, annotation: &Dictionary) -> Option<String> {
    if annotation.get(b"Subtype").ok()?.as_name().ok()? != b"Link" {
        return None;
    }
    let action = annotation.get_deref(b"A", document).ok()?.as_dict().ok()?;
    if action.get(b"S").ok()?.as_name().ok()? != b"URI" {
        return None;
    }
    let uri = action.get_deref(b"URI", document).ok()?.as_str().ok()?;
    Some(String::from_utf8_lossy(uri).into_owned())
}

fn collect_images(
    document: &Document,
    resources: &Dictionary,
    images: &mut HashSet<ObjectId>,
    seen: &mut HashSet<ObjectId>,
    depth: usize,
) {
    if depth > 6 {
        return;
    }
    let Ok(xobjects) = resources
        .get_deref(b"XObject", document)
        .and_then(Object::as_dict)
    else {
        return;
    };
    for (_, value) in xobjects.iter() {
        let Ok(id) = value.as_reference() else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else {
            continue;
        };
        match stream.dict.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Image") => {
                images.insert(id);
            }
            Ok(b"Form") => {
                if let Ok(nested) = stream
                    .dict
                    .get_deref(b"Resources", document)
                    .and_then(Object::as_dict)
                {
                    collect_images(document, nested, images, seen, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn text_check(pdf_path: &Path, expected: &BTreeSet<String>) -> TextCheck {
    let Some(text) = extract_text(pdf_path) else {
        return TextCheck {
            status: "skipped".into(),
            ..TextCheck::default()
        };
    };
    compare_text(&text, expected)
}

/// Compares an extracted PDF text layer with the words expected from the HTML.
fn compare_text(text: &str, expected: &BTreeSet<String>) -> TextCheck {
    let unmapped = text
        .chars()
        .filter(|c| *c == '\u{FFFD}' || ('\u{E000}'..='\u{F8FF}').contains(c))
        .count();
    let found = words(text);
    // A word wrapped at a hyphen ("kobus-" / "oberholzer") is joined by the extractor
    // into one longer word, so a word that begins or ends a found word is present.
    let missing: Vec<String> = expected
        .difference(&found)
        .filter(|word| {
            !found.iter().any(|candidate| {
                candidate.len() > word.len()
                    && (candidate.starts_with(word.as_str()) || candidate.ends_with(word.as_str()))
            })
        })
        .cloned()
        .collect();
    TextCheck {
        status: if missing.is_empty() && unmapped == 0 {
            "passed"
        } else {
            "failed"
        }
        .into(),
        missing_words: missing.into_iter().take(MAX_LISTED).collect(),
        unmapped_characters: unmapped,
        extracted_characters: text.chars().filter(|c| !c.is_whitespace()).count(),
    }
}

/// Extracts the PDF text layer with poppler's `pdftotext`, as a PDF reader would.
pub fn extract_text(pdf_path: &Path) -> Option<String> {
    let program = env::var_os(PDFTOTEXT_ENV)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("pdftotext"));
    let program = PathBuf::from(program);
    if !program.is_file() && which(&program).is_none() {
        return None;
    }
    let temp = tempfile::tempdir().ok()?;
    let output = temp.path().join("text.txt");
    let status = run_with_timeout(
        Command::new(&program)
            .args(["-enc", "UTF-8"])
            .arg(pdf_path)
            .arg(&output),
        &temp.path().join("log"),
        Duration::from_secs(30),
    )
    .ok()?;
    status.success().then(|| fs::read_to_string(&output).ok())?
}

fn warnings(report: &mut PdfReport) {
    if let Some(max) = report.max_pages {
        if report.page_count > max {
            report.warnings.push(format!(
                "The PDF has {} pages; the template allows at most {max}. Shorten the content or tighten the layout.",
                report.page_count
            ));
        }
    }
    let check = &report.text_check;
    match check.status.as_str() {
        "skipped" => report
            .warnings
            .push("Text-layer check skipped: pdftotext is not installed.".into()),
        "failed" if !check.missing_words.is_empty() => report.warnings.push(format!(
            "Words from the HTML are missing from the PDF text layer (hidden or clipped content?): {}",
            check.missing_words.join(", ")
        )),
        _ => {}
    }
    if check.unmapped_characters > 0 {
        report.warnings.push(format!(
            "{} characters in the PDF text layer do not map to Unicode, so they will not copy or parse correctly.",
            check.unmapped_characters
        ));
    }
    if !report.missing_links.is_empty() {
        report.warnings.push(format!(
            "Links missing from the PDF: {}",
            report.missing_links.join(", ")
        ));
    }
    if report.images_found < report.images_expected {
        report.warnings.push(format!(
            "{} of {} images did not render; upload them with upload_asset and reference them as asset://<name>.",
            report.images_expected - report.images_found,
            report.images_expected
        ));
    }
    if !report.tagged {
        report.warnings.push(
            "The PDF is not tagged, so screen readers and parsers lose its structure.".into(),
        );
    }
    if report.title.is_none() {
        report
            .warnings
            .push("The PDF has no title metadata.".into());
    }
}

fn normalize_link(link: &str) -> String {
    let link = link.trim();
    let (scheme, rest) = link.split_once(':').unwrap_or(("", link));
    let scheme = scheme.to_ascii_lowercase();
    let rest = match rest.strip_prefix("//") {
        Some(after) => {
            let (host, path) = after.split_at(after.find('/').unwrap_or(after.len()));
            format!("//{}{}", host.to_ascii_lowercase(), path)
        }
        None => rest.to_string(),
    };
    format!("{scheme}:{}", rest.trim_end_matches('/'))
}

/// Lowercased words (letters and digits), with typographic ligatures expanded.
pub fn words(text: &str) -> BTreeSet<String> {
    let expanded = text
        .replace('\u{FB00}', "ff")
        .replace('\u{FB01}', "fi")
        .replace('\u{FB02}', "fl")
        .replace('\u{FB03}', "ffi")
        .replace('\u{FB04}', "ffl")
        .replace(['\u{FB05}', '\u{FB06}'], "st");
    expanded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn body_of(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let start = lower
        .find("<body")
        .and_then(|i| lower[i..].find('>').map(|j| i + j + 1))
        .unwrap_or(0);
    let end = lower
        .rfind("</body>")
        .filter(|end| *end >= start)
        .unwrap_or(html.len());
    html[start..end].to_string()
}

fn remove_elements(html: &str, tags: &[&str]) -> String {
    let mut output = html.to_string();
    for tag in tags {
        output = remove_between(&output, &format!("<{tag}"), &format!("</{tag}>"));
    }
    output
}

fn remove_between(text: &str, open: &str, close: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut position = 0;
    while let Some(start) = lower[position..].find(open).map(|i| position + i) {
        output.push_str(&text[position..start]);
        output.push(' ');
        position = lower[start..]
            .find(close)
            .map_or(text.len(), |i| start + i + close.len());
    }
    output.push_str(&text[position..]);
    output
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name).map(|i| search + i) {
        search = found + name.len();
        let boundary = lower[..found]
            .chars()
            .last()
            .is_some_and(char::is_whitespace);
        let after = lower[search..].trim_start();
        if !boundary || !after.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - after.len() + 1;
        let value = tag[value_start..].trim_start();
        let (value, _) = match value.chars().next()? {
            quote @ ('"' | '\'') => value[1..].split_once(quote)?,
            _ => value
                .split_once(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or((value, "")),
        };
        return Some(decode_entities(value));
    }
    None
}

/// Decodes the entities Handlebars and typical templates produce.
pub fn decode_entities(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            output.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{A0}'),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "middot" => Some('·'),
            "bull" => Some('•'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "euro" => Some('€'),
            "eacute" => Some('é'),
            "egrave" => Some('è'),
            "aacute" => Some('á'),
            "agrave" => Some('à'),
            "auml" => Some('ä'),
            "ouml" => Some('ö'),
            "uuml" => Some('ü'),
            "Auml" => Some('Ä'),
            "Ouml" => Some('Ö'),
            "Uuml" => Some('Ü'),
            "szlig" => Some('ß'),
            "ccedil" => Some('ç'),
            "ntilde" => Some('ñ'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                output.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                output.push('&');
                rest = &rest[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expectations_read_visible_text_links_and_raster_images() {
        let html = r##"<!doctype html><html><head><title>Ignored title</title>
            <style>.x{color:red}</style></head><body>
            <h1>Jos&eacute; Caf&#xE9; &amp; Co</h1><!-- hidden comment -->
            <p>Visit <a class="l" href="https://Example.com/">example</a> or
            <a href='mailto:a@b.co'>mail</a> <a href="#top">top</a></p>
            <img src="data:image/png;base64,AAA"><img src="data:image/png;base64,AAA">
            <img src="data:image/svg+xml;base64,BBB"><script>var secret = 1;</script>
            </body></html>"##;
        let expected = expectations(html);
        assert!(expected.words.contains("café"));
        assert!(expected.words.contains("josé"));
        assert!(expected.words.contains("visit"));
        assert!(!expected.words.contains("ignored"));
        assert!(!expected.words.contains("secret"));
        assert!(!expected.words.contains("hidden"));
        assert_eq!(
            expected.links.iter().cloned().collect::<Vec<_>>(),
            ["https://Example.com/", "mailto:a@b.co"]
        );
        assert_eq!(expected.raster_images, 1);
    }

    #[test]
    fn links_compare_without_case_or_trailing_slash_noise() {
        assert_eq!(
            normalize_link("https://Example.com/"),
            normalize_link("https://example.com")
        );
        assert_ne!(
            normalize_link("https://example.com/A"),
            normalize_link("https://example.com/a")
        );
    }

    #[test]
    fn words_split_by_line_end_hyphens_count_as_present() {
        // pdftotext joins "kobus-" at a line end with "oberholzer" on the next line.
        let expected: BTreeSet<String> = ["kobus", "oberholzer", "absent"]
            .iter()
            .map(|w| w.to_string())
            .collect();
        let check = compare_text("linkedin.com/in/kobusoberholzer \u{FFFD}", &expected);
        assert_eq!(check.missing_words, ["absent"]);
        assert_eq!(check.unmapped_characters, 1);
        assert_eq!(check.status, "failed");
    }

    #[test]
    fn words_expand_ligatures() {
        assert!(words("pro\u{FB01}le").contains("profile"));
    }

    #[test]
    fn warns_about_budget_links_images_and_tags() {
        let mut report = PdfReport {
            page_count: 3,
            max_pages: Some(2),
            missing_links: vec!["https://x.test".into()],
            images_expected: 2,
            images_found: 1,
            text_check: TextCheck {
                status: "failed".into(),
                missing_words: vec!["hidden".into()],
                unmapped_characters: 4,
                extracted_characters: 10,
            },
            ..PdfReport::default()
        };
        warnings(&mut report);
        let all = report.warnings.join("\n");
        for needle in [
            "at most 2",
            "hidden",
            "4 characters",
            "https://x.test",
            "1 of 2",
            "not tagged",
            "no title",
        ] {
            assert!(all.contains(needle), "missing {needle} in {all}");
        }
    }
}
