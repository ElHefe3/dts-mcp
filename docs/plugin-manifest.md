# DTS plugin manifest v1

The manifest describes an extension hosted by DTS. See the [example manifest](../examples/plugins/duplicate-draft/plugin.json) for a document action that would duplicate a draft.

- `id` is the plugin's stable identifier. IDs use 1–64 lowercase ASCII letters, digits or interior hyphens.
- `name` is the display name. It must contain non-whitespace text.
- `version` is the plugin's own release version in numeric `major.minor.patch` format.
- `api_version` is the DTS plugin contract version. The parser currently accepts `1`.
- `contributions` lists the features exposed to DTS. Each entry has a unique `id` within the plugin, a nonblank `label`, and a `kind`. The recognized kinds are `document_action`, `importer`, `exporter`, and `editor`.
- `permissions` lists the document access the plugin requests. The recognized requests are `read_document` and `write_document`. Omitting the list requests no permissions.

Unknown fields and values, repeated contribution IDs, and repeated permission requests are validation errors. Pass the JSON file contents to `PluginManifest::parse` in [src/plugins.rs](../src/plugins.rs). Run `cargo test --locked --lib plugins::tests` to check the example and rejection cases.

This version implements metadata validation only. Contribution names reserve places in the contract. Package discovery, enablement, permission grants, and code execution require the host work tracked in Beads.
