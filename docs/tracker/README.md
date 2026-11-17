# DTS MCP ticket tracker

Beads is the status authority. Run `bd ready`, `bd show <id>`, and `bd update <id> --claim` from the repository root. The Markdown packets are migration snapshots for reference; update issue descriptions, acceptance and evidence in Beads rather than maintaining parallel status in these files. The Dolt database is local unless separately synchronized; do not assume a Git push backs it up.

## Start here

1. Read the [workspace guide](../../../../AGENTS.md), the [repository guide](../../AGENTS.md), README Run/Test sections, and the selected ticket. Before changing a service contract, integration or deployment, also read the workspace service-alignment document.
2. Run `bd ready` and claim one issue with `bd update <id> --claim` before starting.
3. Confirm the current checkout, branch, local changes and test baseline. Use a ticket-type branch; preserve existing edits and private data. Do not assume a historical bug still reproduces.
4. Follow the ticket's first steps and acceptance criteria. Record commands, results, revision and limitations in its evidence section. Record new work separately; do not silently expand scope.
5. Review the diff and run relevant checks. Close the issue only after every criterion is proven and the change is merged, consistent with the workspace guide. Otherwise record the exact blocker and next action. Do not deploy without the appropriate authorization.

## Migrated issue mapping

The IDs below map the original plan to Beads. Run `bd ready` for current status and dependencies; these are not a second status table. Priorities were imported from the historical plan and should be reassessed against current needs.

| Original ID | Beads ID | Reference packet |
| --- | --- | --- |
| DOCMCP-001 | `docmcp-yu1` | [Bounded discovery responses](tickets/DOCMCP-001.md) |
| DOCMCP-002 | `docmcp-oxo` | [Document contracts](tickets/DOCMCP-002.md) |
| DOCMCP-003 | `docmcp-dte` | [Template validation](tickets/DOCMCP-003.md) |

The source plan is `../../../cv-studio/docs/tickets/business-model-canvas.md`. These packets translate its MCP items; it also lists BMC-001–004, whose ownership and release scope are not established here. DTS-001 is the separate CV Studio dead-code audit in that repository; it is not an MCP ticket and remains in progress there. The workspace roadmap's cross-service job-application goals are not assumed to be authorized implementation tasks for this service.

## Creating a ticket

Use `bd create` and [TEMPLATE.md](TEMPLATE.md) to write a self-contained issue; Beads assigns its ID. State the user-visible outcome, repository boundary, first files/commands, concrete acceptance checks, relevant tests, exclusions, dependencies, and evidence. Make proposed behavior and unresolved decisions explicit. Never put personal documents, credentials, raw protected data, or rendered PDFs in tickets.
