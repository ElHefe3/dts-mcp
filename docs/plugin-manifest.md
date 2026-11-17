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

An entrypoint is either a `builtin` handler shipped with DTS or a `command`:

```json
"entrypoint": {"runtime": "command", "command": "plugin.exe", "args": ["--stdio"]}
```

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

See [plugin development](plugin-development.md) for installation, requests and
the test workflow.
