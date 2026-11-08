# PDF rendering specification

## Goals

1. **Machine-readable text.** Every visible character is in the PDF text layer as
   the Unicode it shows, so it can be copied, searched and parsed (for example by
   applicant tracking systems).
2. **Layout identical to the HTML.** The PDF is the HTML document as a standards
   browser prints it. Nothing rescales or re-flows it.
3. **Complete content.** Links are clickable, images and fonts render, and the
   result says when something did not.

## Contract

A saved, valid document is rendered to one self-contained HTML file:

- `<html lang>` from the template's `lang`, `<title>` from the document name.
- A first `<style>` with `@page { size; margin }` from the template's `page`
  settings and `print-color-adjust: exact`, followed by the template CSS, which may
  override both.
- Every `asset://<name>` reference replaced by a `data:` URI of the uploaded asset.
  A reference to a missing asset fails the render.

The HTML is printed by headless Chromium (`--print-to-pdf`, no header or footer,
document outline enabled). Chromium runs with a fresh profile in a temporary
directory, an unreachable proxy (`socks5://127.0.0.1:9`, loopback included) and a
host resolver that resolves nothing, so it makes no network requests. No zoom, DPI
or shrink-to-fit setting is applied.

Browser selection: `DOCUMENT_TEMPLATING_MCP_CHROMIUM` when set; otherwise the first
installed Chromium, Chrome or Edge in the platform's standard locations; otherwise
`chromium` on `PATH`. `DOCUMENT_TEMPLATING_MCP_CHROMIUM_NO_SANDBOX=1` adds
`--no-sandbox` for containers without user namespaces.

A render job is limited to 60 seconds. On Windows the browser process can exit
before its child writes the file, so the adapter waits, within the same limit, for
output that ends with `%%EOF` and has stopped growing. A successful conversion must
produce bytes beginning with `%PDF-`; only then does it replace the saved PDF. A
failed conversion keeps the previous PDF and returns the browser path, input and
output paths, exit status and the tail of the browser log.

## Report

After saving, the PDF is compared with its HTML:

- Structure (parsed with `lopdf`): page count and first page size, link
  annotations with URI actions, distinct image XObjects (including those inside
  form XObjects), presence of a structure tree, and the Info title.
- Text (poppler `pdftotext -enc UTF-8`): every word of the HTML body text,
  excluding `style`, `script`, `template`, `noscript` and comments, must appear in
  the extracted text, compared case-insensitively with ligatures expanded. The
  extracted text must contain no U+FFFD or private-use characters. Without
  `pdftotext` the check is `skipped`.
- Links: `http`, `https`, `mailto` and `tel` hrefs in the HTML must appear as PDF
  links, compared with a case-insensitive scheme and host and no trailing slash.
- Images: the number of distinct raster `<img>` sources must not exceed the images
  embedded. SVG images are drawn as vectors and are not counted.

Each failed check adds a plain-language entry to `warnings`, as does exceeding
`page.max_pages`, an untagged PDF or a missing title. Warnings never fail the
render.

## Acceptance criteria

- A document containing accented names, en dashes and curly quotes extracts with
  exactly those characters and no unmapped glyphs.
- A CSS grid row prints on one line, and `page.size` A4 produces 210 × 297 mm pages.
- `https` and `mailto` links in the HTML are PDF link annotations; an uploaded PNG
  referenced as `asset://` is embedded.
- The PDF is tagged and titled with the document name.
- Content beyond `page.max_pages` produces a page-budget warning.
- Missing browsers, failed or timed-out conversions and invalid output return
  actionable errors and never replace the last valid PDF.
- The MCP attachment, `resources/read` and the HTTP download deliver identical bytes.
