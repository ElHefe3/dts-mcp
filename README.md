# Document MCP

A local-first Model Context Protocol server for reusable document templates and structured documents. It uses the official Rust MCP SDK (`rmcp`) and supports stdio and authenticated streamable HTTP. See [DEPLOYMENT.md](DEPLOYMENT.md) for the deployed Docker service and SSH client connection.

## Features

- Persist templates independently from documents.
- Reuse one template for any number of documents.
- Define typed fields, defaults, Handlebars HTML, and CSS.
- Merge partial document data over template defaults.
- Save incomplete drafts; enforce required fields when rendering.
- Render standalone HTML and persist it under the data directory.
- Update a template and apply it to subsequent renders of existing documents.
- Prevent deletion of templates that documents still reference.
- Render PDFs with headless Chromium: the layout matches the HTML, text is
  selectable and machine-readable, links are clickable and the PDF is tagged.
- Check every PDF against its HTML (pages, text layer, links, images) and return
  actionable warnings; return PNG page previews for visual checks.
- Store images and fonts as assets and embed them at render time, so documents
  are self-contained and the renderer never needs the network.

## Run

```powershell
$env:DOCUMENT_TEMPLATING_MCP_DATA_DIR = "C:\document-mcp-data"
cargo run
```

If the environment variable is omitted, data is stored in `data/` beneath the process working directory. The directory contains:

```text
templates/<template-id>.json
documents/<document-id>.json
assets/<name>.<ext>
renders/<document-id>.html
renders/<document-id>.pdf
```

Only stderr is used for logging; stdout is reserved for MCP messages.

## MCP client configuration

Build first with `cargo build`, then configure a client to launch the binary. For example, a client configuration using an absolute Windows path can use:

```json
{
  "mcpServers": {
    "documents": {
      "command": "D:\\projects\\job-hunt\\documents\\document-mcp\\target\\debug\\document-templating-mcp.exe",
      "env": {
        "DOCUMENT_TEMPLATING_MCP_DATA_DIR": "C:\\document-mcp-data"
      }
    }
  }
}
```

## Tools

| Tool | Input | Result |
| --- | --- | --- |
| `create_template` | `template` | Saved template |
| `list_templates` | none | Template summaries |
| `get_template` | `id` | Full template |
| `update_template` | `template` | Updated template |
| `delete_template` | `id` | Deletion status |
| `create_document` | `id`, `name`, `template_id`, optional `data` | New document |
| `list_documents` | none | Document summaries |
| `get_document` | `id` | Full document |
| `update_document` | `document` | Updated document |
| `delete_document` | `id` | Deletion status |
| `render_document` | `id` | HTML document attachment, filename, MIME type, resource URI, and output path |
| `render_pdf` | `id` | Binary PDF attachment, metadata and a `report` of checks against the HTML |
| `render_page_images` | `id`, optional `dpi`, `max_pages` | PNG image per page plus the PDF report |
| `upload_asset` | `name`, `data_base64`, optional `replace` | Asset summary with its `asset://` reference |
| `list_assets` | none | Asset summaries |
| `delete_asset` | `name` | Deletion status; refused while referenced |
| `preview_document` | `template_id`, `data` | HTML for unsaved data |
| `open_document_editor` | `id` | Interactive editor (MCP App) |

IDs contain up to 64 lowercase letters, digits, or interior hyphens. Template updates deliberately affect existing documents on their next render. Documents must be moved to another template or deleted before their referenced template can be deleted.

## Template example

Pass this object as the `template` argument to `create_template`:

```json
{
  "schema_version": 1,
  "id": "cover-letter",
  "name": "Cover Letter",
  "version": "1.0.0",
  "description": "A reusable cover letter",
  "sections": [{
    "id": "letter",
    "label": "Letter",
    "fields": [
      {"id": "recipient", "path": "recipient", "label": "Recipient", "type": "text", "required": true},
      {"id": "body", "path": "body", "label": "Body", "type": "textarea"}
    ]
  }],
  "defaults": {"recipient": "", "body": ""},
  "render": {
    "html": "<h1>{{recipient}}</h1><p>{{body}}</p>",
    "css": "body { font-family: sans-serif; }"
  },
  "page": {"size": "A4", "margin": "15mm", "max_pages": 1},
  "lang": "en"
}
```

`page` is optional (default A4, 10 mm margins, no page budget). It becomes a CSS
`@page` rule ahead of the template CSS, so a template may still override it.
`size` is A3, A4, A5, Letter or Legal, optionally followed by `landscape`;
`margin` takes 1-4 lengths in mm, cm, in, pt or px. Exceeding `max_pages` is a
warning in the PDF report, not an error. `lang` sets `<html lang>`.

Field types are `text`, `textarea`, `rich_text`, `url`, `string_list`, and `object_list`. An `object_list` field defines its nested fields in `fields`. Handlebars escapes ordinary `{{value}}` substitutions; use triple braces only for intentionally trusted HTML such as rich text.

Create a document with `create_document`:

```json
{
  "id": "acme-application",
  "name": "Acme application",
  "template_id": "cover-letter",
  "data": {"recipient": "Acme", "body": "I am interested in the role."}
}
```

Then call `render_document` with `{"id":"acme-application"}`.

Rendering returns the actual document as an embedded MCP resource (`text/html`),
alongside the existing structured HTML and server path. The response includes
`filename`, `mime_type`, and `resource_uri`; clients can retrieve the saved render
with `resources/read` using `document://renders/acme-application.html`.
This URI is an MCP resource identifier, not a public browser URL.

The registered Codex SSH bridge automatically writes the returned document into
this project's `output/` directory and adds `local_path`, `file_uri`, and a clickable
file link to the result. Set `DOCUMENT_TEMPLATING_MCP_DOWNLOAD_DIR` to choose another
download directory. Rendering the same document updates its local HTML copy.
Clients should present that attachment or link after rendering instead of showing
only the Docker-internal output path. The save helper also returns a local file.

## Assets

The renderer has no network access, so remote image or font URLs do not load.
Upload images (png, jpg, gif, webp, svg) and fonts (woff2, woff, ttf, otf), up to
5 MB each, with `upload_asset`, then reference them as `asset://<name>` anywhere in
template HTML or CSS or in document data, for example
`<img src="asset://photo.png">` or `src: url(asset://inter-400.woff2)` in an
`@font-face` rule. Rendering replaces each reference with a data URI, so the saved
HTML is self-contained and shows exactly what the PDF shows. Rendering fails with
a clear error if a referenced asset has not been uploaded. An asset cannot be
deleted while a template or document refers to it.

## PDF rendering

Call `render_pdf` with `{"id":"acme-application"}`. The server renders the
document's HTML, prints it with headless Chromium and returns the exact binary
PDF as an `application/pdf` embedded resource, also available through
`resources/read` (`document://renders/acme-application.pdf`) and the
authenticated browser route `/renders/acme-application.pdf`.

Chromium is the engine that displays the HTML, so the PDF keeps the same layout,
including flexbox, grid, web fonts and print CSS. Its text layer maps every glyph
to Unicode, so text copies, searches and parses correctly (accents, dashes and
quotes included); links become clickable annotations; the PDF is tagged and takes
its title from the document name. Page geometry comes only from `@page`: there is
no zoom or DPI scaling, so 1 CSS px in the browser is 1 CSS px on paper.

Each result includes a `report`:

| Field | Meaning |
| --- | --- |
| `page_count`, `max_pages`, `page_size_mm` | Pages produced, the template budget, first page size |
| `text_check` | `passed`, `failed` or `skipped`; words from the HTML missing from the PDF text layer; characters that do not map to Unicode |
| `links`, `missing_links` | Link targets in the PDF, and HTML links that did not make it |
| `images_expected`, `images_found` | Distinct raster `<img>` sources versus images embedded in the PDF |
| `tagged`, `title` | Structure tags and title metadata |
| `warnings` | Plain-language problems to fix; empty means every check passed |

An agent should fix every warning and render again. `render_page_images` returns
PNG images of the printed pages (default 80 dpi, up to 10 pages) to judge layout,
page breaks and overflow.

The container installs Chromium, poppler-utils (`pdftotext` for the text check,
`pdftoppm` for page images) and Liberation and DejaVu fonts. Locally the server
finds an installed Chromium, Chrome or Edge; set `DOCUMENT_TEMPLATING_MCP_CHROMIUM`
to choose one, `DOCUMENT_TEMPLATING_MCP_CHROMIUM_NO_SANDBOX=1` where the browser
sandbox is unavailable (containers), and `DOCUMENT_TEMPLATING_MCP_PDFTOTEXT` /
`DOCUMENT_TEMPLATING_MCP_PDFTOPPM` for poppler tools outside `PATH`. Without
poppler the text check reports `skipped` and page images are unavailable. See the
[rendering specification](docs/pdf-rendering-spec.md) for the contract.
## Test

```powershell
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
```

The integration tests cover persistence across restarts and a real MCP client/server conversation over stdio.
PDF tests run when Chromium, Chrome or Edge is installed; the text-layer
assertions also need poppler's `pdftotext`, which the container image provides:

```powershell
podman build -f Containerfile -t localhost/document-templating-mcp:test .
```

## Scope

The server supports stdio and authenticated HTTP transport. Documents are exported
as HTML and PDF. Multi-user isolation and cross-process database coordination are
not included. File writes are serialized within one server
process; run only one server process against a given data directory.
