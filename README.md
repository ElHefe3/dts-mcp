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

## Run

```powershell
$env:DOCUMENT_TEMPLATING_MCP_DATA_DIR = "C:\document-mcp-data"
cargo run
```

If the environment variable is omitted, data is stored in `data/` beneath the process working directory. The directory contains:

```text
templates/<template-id>.json
documents/<document-id>.json
renders/<document-id>.html
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
| `render_pdf` | `id` | Binary PDF attachment, filename, MIME type, resource URI, and output path |

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
  }
}
```

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

## PDF rendering

Call `render_pdf` with `{"id":"acme-application"}` to generate a real PDF using
wkhtmltopdf 0.12.6. Like `cv-studio`, the renderer enables local
file access, print media, internal and external links, PDF outlines, and 300-DPI
output. The result is an `application/pdf` embedded resource containing the exact
binary PDF, not a raster screenshot or renamed HTML file. Text remains selectable
and supported links remain clickable. The PDF is also available through the
authenticated browser route `/renders/acme-application.pdf`.

The container installs wkhtmltopdf and Liberation fonts. For local PDF rendering,
set `DOCUMENT_TEMPLATING_MCP_WKHTMLTOPDF`, provide a project-local renderer, or
install wkhtmltopdf on `PATH`. The SSH bridge saves the returned binary attachment
to `output/` and adds a local clickable path. See the [PDF rendering guide](docs/pdf-rendering.html)
for adapter diagrams, renderer discovery, and troubleshooting, and the
[rendering specification](docs/pdf-rendering-spec.md) for the contract.

## Test

```powershell
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
```

The integration tests cover persistence across restarts and a real MCP client/server conversation over stdio.

## Scope

The server supports stdio and authenticated HTTP transport. Documents are exported
as HTML and PDF. Multi-user isolation and cross-process database coordination are
not included. File writes are serialized within one server
process; run only one server process against a given data directory.
