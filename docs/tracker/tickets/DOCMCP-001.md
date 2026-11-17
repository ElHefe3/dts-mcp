# DOCMCP-001 — Bound template discovery responses

## Outcome and context

A client can discover many templates without automatically receiving every rendered preview, while an editor can still request the detail it needs. The existing Canvas plan reports excessive responses. At revision 3b03619, `list_templates` returns `TemplateGallery` containing `preview_html`; `get_template` and editor state include full template data. Confirm current behavior first.

## Scope and first steps

1. Read `src/server.rs`, `src/model.rs`, `src/store.rs`, `src/web.rs`, `web/src/main.js`, and `tests/mcp_stdio.rs`; trace both MCP and browser/editor consumers before changing shapes.
2. Define a compact default discovery response and an explicit preview/detail path. Preserve a documented way to retrieve full renderer source. Decide and document a practical preview/output limit; avoid silently truncating data.
3. Implement the agreed API changes with compatibility or a clearly documented migration, and update README tool descriptions and the affected client flows.

## Exclusions and decisions

No Canvas renderer or deployment. The original plan proposes 100 summaries under 32 KB: verify that this is feasible with allowed metadata lengths and pagination or explicit limits; do not claim a guarantee without enforcement. Consult the workspace service-alignment document before changing the contract.

## Acceptance criteria

- [ ] Default discovery does not include rendered previews; an explicit path returns previews and full source as documented.
- [ ] A 100-template fixture meets the documented size bound, including a defined response for oversized metadata or previews.
- [ ] Editor/gallery workflow still works, and errors and compatibility are documented.
- [ ] Tests exercise default and expanded responses, limits, and client behavior.

## Verification

Run focused MCP and browser tests, then README's Rust checks. Build the web UI if changed. Use synthetic data; report missing tools or skipped checks. No deployment is implied.

## Evidence and handoff

Not started. Record revision, measured sizes, commands, results, compatibility decisions and remaining limitations here.
