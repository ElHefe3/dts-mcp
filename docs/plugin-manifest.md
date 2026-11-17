# DTS plugin manifest v1

DTS extensions are local packages hosted by the document service. A package is a
directory below `<data-dir>/plugins/<plugin-id>/` containing `plugin.json` and,
for a command plugin, its executable. DTS owns document persistence, rendering,
authentication and the workspace UI; a plugin declares contributions rather than
receiving those capabilities directly.

```json
{
  "id": "duplicate-draft",
  "name": "Duplicate Draft",
  "version": "1.0.0",
  "api_version": 1,
  "contributions": [{"id": "duplicate", "label": "Duplicate draft", "kind": "document_action"}],
  "permissions": ["read_document", "write_document"],
  "entrypoint": {"runtime": "builtin", "handler": "duplicate_draft"}
}
```

`id` is 1–64 lowercase ASCII letters, digits or interior hyphens. `version` is
numeric `major.minor.patch`; `api_version` is currently `1`. DTS rejects unknown
fields, unknown enum values and repeated contribution IDs or permissions.

Contribution kinds are `document_action`, `importer`, `exporter`, `editor`,
`template_pack`, `validation_rule`, and `render_hook`. The first four are exposed
through the authenticated workspace API. The final three are declarative v1
extension points; they never run unattended integrations.

An entrypoint is a `builtin` handler shipped with DTS, a one-shot `command`, or
a DTS-managed persistent local HTTP process. Existing `api_version: 1` command
packages remain supported unchanged.

```json
"entrypoint": {"runtime": "command", "command": "plugin.exe", "args": ["--stdio"]}
```

An HTTP package declares its package-relative launcher and one or more
`http_operation` contributions:

```json
{
  "id": "application-tracker",
  "name": "Application Tracker",
  "version": "1.0.0",
  "api_version": 1,
  "contributions": [
    {"id": "list-applications", "label": "List applications", "kind": "http_operation"}
  ],
  "permissions": [],
  "entrypoint": {"runtime": "http", "command": "application-tracker.cmd", "health_path": "/health"}
}
```

DTS allocates a loopback address and an opaque per-process bearer token, then
starts the package with `DTS_PLUGIN_BIND`, `DTS_PLUGIN_TOKEN` and a package-local
`DTS_PLUGIN_DATA_DIR`. The package
must bind only to that address, return HTTP 200 from its health path within five
seconds, require `Authorization: Bearer <DTS_PLUGIN_TOKEN>` on every request,
and accept `POST /operations/<declared-contribution-id>` with a JSON body.
It returns one JSON response, limited to 256 KiB. DTS stops the process on
disable and shutdown, restarts it on the next operation after a crash, and never
reveals its address or token to browser or MCP clients.

Browser clients call `POST /api/plugins/<plugin-id>/operations/<contribution-id>`
through DTS's authenticated local API. MCP clients call `call_plugin_operation`.
Both surfaces can invoke only enabled, declared `http_operation` contributions.
The host does not expose a general proxy or a plugin-provided UI in v1.

Commands are package-relative only. DTS sends one JSON request on stdin and
expects one JSON response on stdout, limits the response to 256 KiB, kills a
request after five seconds, and never supplies data unless the matching
permission is declared. A `document_action` response must contain a `document`;
DTS assigns the new ID and creates it, so a plugin cannot overwrite the draft it
read. Importers and exporters are similarly limited to their declared access.

Executable plugins are disabled until a local operator enables them through the
authenticated plugin API. This is a trust acknowledgement, not an OS sandbox:
only install packages you trust. Built-in packages are enabled by default.
Malformed and duplicate-ID packages are listed as invalid and their
contributions do not appear in the workspace.

HTTP plugins are trusted local software, not a sandbox. The loopback address,
ephemeral token, package-relative launch requirement and contribution allowlist
protect DTS clients from direct access, but do not prevent a locally installed
executable from accessing the machine. Plugin data belongs in the plugin's
host-provided data directory rather than the DTS document store. A package must
not store DTS credentials in its manifest, logs or source tree.

See [plugin development](plugin-development.md) for installation, requests and
the test workflow.
