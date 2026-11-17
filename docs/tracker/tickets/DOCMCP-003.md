# DOCMCP-003 — Validate templates and bound rendering

## Outcome and context

Malformed templates fail with actionable errors before persistence, and large inputs have documented limits. The existing implementation validates IDs, field paths, object-list definitions, defaults and required fields in `src/render.rs`; ordinary Handlebars substitutions are escaped. Audit existing behavior before changing it.

## Scope and first steps

1. Read `src/render.rs`, `src/model.rs`, `src/store.rs`, `src/server.rs` and existing workflow tests. Inventory what is already validated, including nested fields, rich text and preview/render paths.
2. Add narrowly scoped missing validation for duplicate identifiers and invalid definitions; specify any placeholder checks so valid Handlebars helpers, scopes and dynamic fields are not rejected blindly.
3. Propose and enforce documented limits for fields, list items, input and rendered outputs at every relevant transport. Return useful errors rather than truncating content. Preserve draft and rich-text behavior unless an explicit compatibility policy is agreed.

## Exclusions and decisions

No Canvas template, multi-user isolation or deployment. Decide limit values and rich-text trust semantics from actual workflows and compatibility evidence before implementation. Consult service-alignment guidance for contract changes.

## Acceptance criteria

- [ ] Invalid definitions and duplicate IDs fail before create/update persistence with actionable locations; valid nested templates continue to work.
- [ ] Required data errors identify the field path; existing draft behavior is retained.
- [ ] Boundary and over-limit tests cover inputs and preview/render output without truncating valid data.
- [ ] Escaping and intentionally trusted rich-text behavior are documented and tested.
- [ ] Any placeholder diagnostics distinguish warnings from errors and cover supported Handlebars constructs.

## Verification

Run focused validation/store/MCP tests and README's Rust checks. Exercise representative valid and invalid synthetic templates; avoid real CVs or personal data. Record unavailable checks.

## Evidence and handoff

Not started. Record revision, chosen limits, compatibility decisions, commands and outcomes here.
