# DOCMCP-002 — Verify document contracts and update safety

## Outcome and context

Clients can list, retrieve and update structured documents reliably, with a clear policy for concurrent edits. The original plan reports a list-wrapper error and an update schema mismatch. At revision 3b03619, `list_documents` already returns `DocumentList` and `Document.data` is `serde_json::Value`; commit f136677 addressed the list wrapper. Do not redo or mark the whole ticket complete without testing the remaining contract.

## Scope and first steps

1. Inspect `src/server.rs`, `src/model.rs`, `src/store.rs`, `tests/mcp_stdio.rs` and the editor client. Inspect actual advertised schemas and reproduce any remaining mismatch.
2. Add missing tests for empty, single and multiple list results, nested JSON round trips, invalid input, and the advertised update schema. Repair only verified defects.
3. Determine and document a backward-compatible update/concurrency contract. The original plan requests a revision for conflict detection; it is a behavioral and persisted-data change, so specify migration and a conflict test before implementing it.

## Exclusions and decisions

Do not promise multi-process coordination: README currently supports one server process per data directory. No deployment or unrelated storage rewrite. Read the workspace service-alignment document before modifying contracts.

## Acceptance criteria

- [ ] MCP list results validate for zero, one and multiple documents.
- [ ] Advertised create/update inputs accept structured JSON; nested arrays and objects survive create, retrieve and update.
- [ ] Invalid field data returns an actionable field path.
- [ ] Concurrent-edit handling is implemented and tested or separately scoped with an explicit decision; no unsupported guarantee is claimed.
- [ ] Contract and migration notes describe actual behavior.

## Verification

Run focused stdio and store tests, then README's Rust checks. Use temporary fixtures and document any skipped browser or transport checks.

## Evidence and handoff

Partial prior work: commit f136677 addresses the list wrapper; prior Rust suite passed on 2026-11-17. Remaining criteria have not been proven as a ticket. Record exact revision and new test results here.
