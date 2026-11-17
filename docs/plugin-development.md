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

## DTS-managed HTTP plugins

Use an HTTP plugin for a stateful workflow such as application tracking. DTS
starts the package only after the operator enables it. It supplies the private
loopback bind address in `DTS_PLUGIN_BIND` and a fresh bearer token in
`DTS_PLUGIN_TOKEN`; neither is persisted in `plugin.json`. The plugin must be
ready at `/health` within five seconds and must authenticate every request.

Declare each public operation as `http_operation`. DTS brokers it at
`POST /api/plugins/<plugin-id>/operations/<contribution-id>` for the browser
and through the `call_plugin_operation` MCP tool. Each receives a JSON input and
returns JSON; requests and results are limited to 256 KiB. Do not add a catch-all
proxy or let the browser connect to the plugin port. A stopped, crashed or
unready plugin returns an actionable DTS error and does not take down the host.

The host owns process lifecycle and permission boundaries. The plugin owns its
own records and any configured external service credentials. v1 deliberately
does not embed a plugin web application: use contributed operations and a DTS
workspace view until a separately reviewed UI-extension contract exists.

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
