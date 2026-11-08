//! Uploaded images and fonts, referenced as `asset://<name>` and inlined as data URIs
//! when a document renders. Inlining makes every rendered HTML file self-contained, so
//! the browser preview, the saved HTML and the PDF show exactly the same pixels without
//! the renderer ever touching the network or the file system.

use std::collections::BTreeSet;

use base64::Engine;

use crate::error::{AppError, Result};

pub const SCHEME: &str = "asset://";
pub const MAX_ASSET_BYTES: usize = 5 * 1024 * 1024;

/// Returns the MIME type for a valid asset name such as `photo.png`.
pub fn validate_name(name: &str) -> Result<&'static str> {
    let invalid = || {
        AppError::Invalid(format!(
            "asset name must be 1-64 lowercase letters, digits or hyphens plus an extension \
             (png, jpg, jpeg, gif, webp, svg, woff2, woff, ttf, otf); got '{name}'"
        ))
    };
    let (stem, extension) = name.rsplit_once('.').ok_or_else(invalid)?;
    let stem_ok = !stem.is_empty()
        && stem.len() <= 64
        && stem
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !stem.starts_with('-')
        && !stem.ends_with('-');
    if !stem_ok {
        return Err(invalid());
    }
    Ok(match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => return Err(invalid()),
    })
}

/// Rejects content that does not match its declared type.
pub fn validate_content(name: &str, mime_type: &str, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Err(AppError::Invalid(format!("asset '{name}' is empty")));
    }
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(AppError::Invalid(format!(
            "asset '{name}' is {} bytes; the limit is {MAX_ASSET_BYTES}",
            bytes.len()
        )));
    }
    let matches = match mime_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        "font/woff2" => bytes.starts_with(b"wOF2"),
        "font/woff" => bytes.starts_with(b"wOFF"),
        "font/ttf" => bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true"),
        "font/otf" => bytes.starts_with(b"OTTO"),
        "image/svg+xml" => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| AppError::Invalid(format!("asset '{name}' is not UTF-8 SVG")))?;
            let lower = text.to_ascii_lowercase();
            if lower.contains("<script") || lower.contains("javascript:") {
                return Err(AppError::Invalid(format!(
                    "asset '{name}' contains script; SVG assets must be static"
                )));
            }
            lower.contains("<svg")
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "asset '{name}' content does not match its {mime_type} extension"
        )))
    }
}

pub fn decode_base64(name: &str, data: &str) -> Result<Vec<u8>> {
    let data = data
        .split_once(";base64,")
        .map_or(data, |(_, payload)| payload)
        .trim();
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| AppError::Invalid(format!("asset '{name}' is not valid base64: {error}")))
}

/// All asset names referenced in a text (template, document JSON or rendered HTML).
pub fn references(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut rest = text;
    while let Some(index) = rest.find(SCHEME) {
        rest = &rest[index + SCHEME.len()..];
        let name = reference_at(rest);
        if !name.is_empty() {
            names.insert(name.to_string());
        }
    }
    names
}

fn reference_at(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'))
        .unwrap_or(text.len());
    text[..end].trim_end_matches('.')
}

/// Replaces every `asset://name` with a data URI. With `strict`, an unknown asset is an
/// error; otherwise the reference is left in place (used for gallery previews).
pub fn inline(
    html: &str,
    strict: bool,
    load: impl Fn(&str) -> Result<(&'static str, Vec<u8>)>,
) -> Result<String> {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(index) = rest.find(SCHEME) {
        output.push_str(&rest[..index]);
        rest = &rest[index + SCHEME.len()..];
        let name = reference_at(rest);
        match load(name) {
            Ok((mime_type, bytes)) => {
                output.push_str("data:");
                output.push_str(mime_type);
                output.push_str(";base64,");
                output.push_str(&base64::engine::general_purpose::STANDARD.encode(bytes));
                rest = &rest[name.len()..];
            }
            Err(error) if strict => {
                return Err(match error {
                    AppError::NotFound(_) => AppError::Invalid(format!(
                        "asset '{name}' is referenced but not uploaded; call upload_asset first"
                    )),
                    other => other,
                })
            }
            Err(_) => output.push_str(SCHEME),
        }
    }
    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    #[test]
    fn names_map_to_mime_types() {
        assert_eq!(validate_name("photo.png").unwrap(), "image/png");
        assert_eq!(validate_name("inter-400.woff2").unwrap(), "font/woff2");
        for bad in [
            "Photo.png",
            "../x.png",
            "photo",
            "photo.exe",
            "-a.png",
            ".png",
        ] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn content_must_match_extension() {
        assert!(validate_content("a.png", "image/png", PNG).is_ok());
        assert!(validate_content("a.png", "image/png", b"GIF89a").is_err());
        assert!(validate_content("a.svg", "image/svg+xml", b"<svg></svg>").is_ok());
        assert!(validate_content(
            "a.svg",
            "image/svg+xml",
            b"<svg><script>alert(1)</script></svg>"
        )
        .is_err());
    }

    #[test]
    fn inlines_references_in_attributes_and_css() {
        let html =
            r#"<img src="asset://photo.png"><style>x{background:url(asset://photo.png)}</style>"#;
        let output = inline(html, true, |name| {
            assert_eq!(name, "photo.png");
            Ok(("image/png", PNG.to_vec()))
        })
        .unwrap();
        assert!(!output.contains(SCHEME));
        assert_eq!(output.matches("data:image/png;base64,").count(), 2);
        assert_eq!(references(html).len(), 1);
    }

    #[test]
    fn missing_assets_fail_only_in_strict_mode() {
        let html = "<img src=\"asset://gone.png\">";
        let missing = |_: &str| Err(AppError::NotFound("gone".into()));
        assert!(inline(html, true, missing)
            .unwrap_err()
            .to_string()
            .contains("upload_asset"));
        assert_eq!(inline(html, false, missing).unwrap(), html);
    }

    #[test]
    fn accepts_data_url_prefixes() {
        assert_eq!(
            decode_base64("a", "data:image/png;base64,AAEC").unwrap(),
            [0, 1, 2]
        );
    }
}
