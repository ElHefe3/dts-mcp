# Developing DTS plugins

## Package and lifecycle

Create a directory under the DTS data directory:

```text
plugins/acme-tools/plugin.json
plugins/acme-tools/plugin.exe
```

Restart DTS or refresh the browser workspace. `GET /api/plugins` lists installed
packages and their health. Enable or disable one with authenticated
`PUT /api/plugins/<id>` and `{"enabled": true|false}`. Enablement persists in
`plugins/state.json`; disabled plugins contribute no actions, importers,
exporters or editor affordances. Removing a package directory uninstalls it.

The service installs four example manifests on first startup: `duplicate-draft`,
`text-io`, `inline-cv`, and `cv-template-pack`. They show a document action,
plain-text import/export, an editor contribution, and declarative template-pack,
validation and render-hook contracts respectively.

## Command protocol

For each invocation DTS writes one JSON object to stdin:

```json
{"operation":"document_action","input":{"document":{"id":"draft", "data":{}}}}
```

`operation` is `document_action`, `import`, or `export`. A document appears only
when `read_document` was declared; an operation that creates a document requires
`write_document`. Return one JSON object only. A document action returns
`{"document": { ... }}`; an exporter returns `{"message":"plain text"}`; an
importer returns `{"data": { ... }}`. Keep responses below 256 KiB and finish
inside five seconds.

Command plugins are local trusted software. The host constrains the data and
write operations it brokers, but cannot turn an arbitrary local executable into
an OS sandbox. Do not place credentials, DTS tokens or personal source documents
inside a plugin package.

## Workspace integration

Use `GET /api/plugin-contributions` to inspect enabled contributions. DTS shows
enabled `document_action` contributions in a document editor and invokes them at
`POST /api/documents/<document-id>/plugins/<plugin-id>/<contribution-id>`.
Import through `POST /api/plugin-import`; export through
`GET /api/documents/<id>/plugin-export/<plugin>/<contribution>`.

An editor contribution declares an alternative editor experience. The initial
inline-CV package is structured-field based, not a template-layout editor.

## Verification

```powershell
cargo fmt --check
cargo test --locked plugins::tests
cargo test --locked
```

For a manual check, create a synthetic draft, use **Duplicate draft** in the
browser editor, then disable `duplicate-draft` and confirm it disappears after
refresh.
