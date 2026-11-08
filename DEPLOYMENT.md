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

`render_pdf` uses the same wkhtmltopdf 0.12.6 pipeline and render arguments as
`cv-studio`: local-file access, print media, internal/external
links, 300-DPI output, and outlines. The runtime image includes wkhtmltopdf and
Liberation fonts. The tool returns an `application/pdf` embedded resource, and
`/renders/{id}.pdf` provides the authenticated browser download.

The SSH bridge decodes the PDF resource and saves exact binary bytes to `output/`.
The Professional CV template uses a print-safe table layout and real contact links.
Its stored template lives in the Docker volume; the reference copy is
`examples/professional-cv.json`.

Live Scout24 validation passed: one A4 page, correct embedded photo, no column
overlap, selectable name/CV text, and clickable GitHub, LinkedIn, and email links.
The MCP attachment, resource read-back, and downloaded local PDF were byte-identical.

Verification commands:

```powershell
& C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe -m unittest discover -s clients -p test_document_files.py -v
& C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe clients\verify_render_delivery.py
& C:\Users\kobus\Documents\projects\mcp-server-setup\.venv\Scripts\python.exe clients\verify_pdf_delivery.py
```

