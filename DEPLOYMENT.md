# Document templating MCP deployment

Host: `lab-compute` / `lab-pta-03` / `192.168.1.115`.

Server directory: `/home/kreef/projects/document-templating-mcp`.
Docker Compose service: `documents` (container `document-templating-mcp-documents-1`).
Local server endpoint: `http://127.0.0.1:8093/mcp`.
Document storage: Docker named volume `document-templating-mcp_document-data`.

The service requires a bearer token stored in a mode-0600 `.env` on the server.
It binds only to loopback. The client bridge opens an encrypted SSH tunnel and
loads the token over SSH at connection time. No token is written locally.

The host was selected because it already runs Docker and the shared MCP gateway,
and had approximately 6 GiB of available RAM before deployment. The existing
Tailscale routes require sudo to change, so the connection uses SSH instead.

## Operations

```powershell
ssh lab-compute 'cd /home/kreef/projects/document-templating-mcp && docker compose ps'
ssh lab-compute 'cd /home/kreef/projects/document-templating-mcp && docker compose logs --tail=60 documents'
ssh lab-compute 'cd /home/kreef/projects/document-templating-mcp && docker compose up -d --build'
```

`restart: unless-stopped` provides automatic restart. Keep the Docker volume when
recreating the service. Include that volume and the server `.env` in protected
backups; do not use `docker compose down -v` unless deleting all documents is intended.

## Client connection

The bridge uses the existing Python environment at
`C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe`.
It requires the configured `lab-compute` SSH alias and its existing SSH key.

Codex is registered globally under the name `documents`, launching
`clients/ssh_bridge.py` from the original project. Start a new Codex session or
reload MCP connections to load this registration. The launcher has been tested
with a real stdio MCP client: all 14 tools and both UI resources were discovered.
Windows OpenSSH requires `PROGRAMDATA` even when launched with MCP's reduced
environment; the bridge supplies it without embedding credentials.

For a standalone validation including persistence across a service restart:

```powershell
& C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe .\clients\ssh_bridge.py --smoke
```

For browser access, keep this command running and open `http://localhost:18093`:

```powershell
ssh -N -L 127.0.0.1:18093:127.0.0.1:8093 lab-compute
```

The browser login requires the server token. Retrieve it privately over SSH; do
not paste it into a chat or commit it to the repository.

## Build repairs

- Compose explicitly selects `Containerfile` and supports a configurable host port.
- The container frontend stage copies `vite.config.js`.
- Vite uses `web/` as its root, matching the Rust embedded HTML paths.
- `server.rs` explicitly imports `std::future::Future` for Rust edition 2021.
- `Cargo.lock` is regenerated to include the HTTP transport dependencies.
- The authorized HTTP test uses `localhost`, accepted by the SDK's host validation.

Verified live: health check, unauthenticated requests rejected with 401, template
creation, document creation, HTML rendering, persistence after container restart,
and cleanup of temporary verification records. The deployment does not currently
have a new scheduled backup job.

Codex registration follows the official [MCP command documentation](https://learn.chatgpt.com/docs/developer-commands#codex-mcp).

Final validation: all 9 Rust integration tests passed with cargo test --locked.

## Document delivery

`render_document` now returns an embedded HTML document resource, in addition to
the structured render result. The SSH bridge automatically saves exact UTF-8
bytes into the local project `output/` directory and returns a clickable path.
Reload the MCP connection after changing the bridge; existing running bridge
processes keep the old code until restarted. No extra SSH download is required.

Verified after deployment: all 9 Rust integration tests and 4 bridge tests pass.
The live registered launcher returned the ticket attachment, wrote an identical
UTF-8 local file, and reopened it through the advertised MCP resource template.

## PDF delivery

Since 9 November 2026 `render_pdf` prints with headless Chromium (Debian's
`chromium` package) instead of wkhtmltopdf. The Debian wkhtmltopdf build was
compiled against unpatched Qt: it ignored print media, link and outline flags,
garbled non-ASCII text extraction and could not lay out CSS grid. Chromium runs
inside the read-only container with `--no-sandbox`
(`DOCUMENT_TEMPLATING_MCP_CHROMIUM_NO_SANDBOX=1`), a fresh profile in the 512 MB
`/tmp` tmpfs and no network access; Compose runs an init process to reap its
helper processes. poppler-utils provides `pdftotext` for the text-layer check and
`pdftoppm` for `render_page_images`. The image is about 800 MB.

Every `render_pdf` returns a `report` (pages, text layer, links, images, tags,
title, warnings). `/renders/{id}.pdf` provides the authenticated browser download,
and the SSH bridge saves the exact PDF bytes to `output/`.

Chromium prints CSS at its true size, whereas wkhtmltopdf shrank the 1000 px CV
layout to about 72%. The Professional CV template (stored in the Docker volume,
reference copy `examples/professional-cv.json`, version 1.1.0) therefore carries
`@media print` sizing, A4 with 12 mm margins and `max_pages: 1`. The version
before this change was saved locally before the update.

Live validation on 9 November 2026: all six stored documents, including the three
Professional CV documents, rendered on one A4 page with the text check passed and
no warnings. The Scout24 CV kept its embedded photo and clickable GitHub, LinkedIn
and email links, with a tagged PDF titled after the document.

Rollback: before deployment the data volume was archived to
`~/backups/document-templating-mcp/data-20261109T051241.tgz` and the previous
image tagged `localhost/document-templating-mcp:pre-chromium-20261109T051241`.
To roll back, retag that image as `:latest`, restore the previous `compose.yaml`
from the same backup folder and run `docker compose up -d` without `--build`.
Verification commands:

```powershell
& C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe -m unittest discover -s clients -p test_document_files.py -v
& C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe clients\verify_render_delivery.py
& C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe clients\verify_pdf_delivery.py
```

