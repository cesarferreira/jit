---
name: jit
description: Use when users ask to look up or search Jira issues, inspect details, list boards or sprints, create or edit tickets, change status, add comments or issue links, use structured Jira output, or configure the jit CLI.
---

# Jit CLI

## Overview

Use this skill to execute `jit` for common Jira workflows: ticket lookup, detailed issue inspection, current sprint ticket listing, backlog ticket creation, current-sprint ticket creation, ticket/task editing, JQL search, transitions, comments, issue links, board/sprint discovery, and structured output for scripts.

## Install or update this skill

Run `jit skill install` to install the skill bundled with the installed CLI on the current machine for all supported agents with existing config directories: Codex, Claude Code, Pi, Cursor, and Antigravity App/IDE. It skips absent config folders and reports a no-op if none are found. Select specific targets with `--agent pi,cursor,antigravity` or repeated flags such as `--agent codex --agent claude`. Explicit `--agent` selections override detection and may create the selected config folders. Names are case insensitive and duplicate selections are installed once. It replaces the jit skill in the selected agents' global directories: `~/.codex/skills/jit/`, `~/.claude/skills/jit/`, `~/.pi/agent/skills/jit/`, `~/.cursor/skills/jit/`, and `~/.gemini/config/skills/jit/` respectively. Antigravity uses `~/.gemini/antigravity/skills/jit/` when only its legacy config directory exists. No network or Jira credentials are required. From a checkout, `cargo run -- skill install` installs that checkout's bundled skill. After upgrading `jit`, rerun `jit skill install` to keep the skill matched to the binary.

## Prerequisites

1. Ensure the command is available:
- Installed CLI: `jit --version`
- From source in this repo: `cargo run -- <args>`
2. Provide Jira credentials through one of:
- `--config-file <path>`
- `config.toml` in the current directory
- `~/.config/jit/config.toml`

Required config values:
```toml
[jira]
base_url = "https://your-company.atlassian.net"
api_token = "your_api_token_here"
user_email = "your_email@example.com"
```

Optional `[jira]` read-field overrides are `sprint_field = "customfield_10020"` and `acceptance_criteria_field = "customfield_10396"`. These are also the defaults; configure the IDs for the target Jira instance when they differ. Refreshing credentials with `jit auth` preserves the overrides.

## Core Commands

1. Fetch a ticket summary (default format):
```bash
jit ISSUE-123
jit https://your-company.atlassian.net/browse/ISSUE-123
```
2. Fetch compact text output:
```bash
jit --text ISSUE-123
```
3. Fetch machine-readable JSON:
```bash
jit --json ISSUE-123
```
4. Show detailed issue fields:
```bash
jit --show ISSUE-123
```
5. Include description/comments/timestamps in detailed output:
```bash
jit --show --include-description ISSUE-123
jit --show --include-comments ISSUE-123
jit --show --include-prs ISSUE-123
jit --show --full ISSUE-123
jit --json --full ISSUE-123
```
6. Filter and limit comments:
```bash
jit --show --include-comments --comments-limit 3 ISSUE-123
jit --show --include-comments --all-comments ISSUE-123
jit --show --include-comments --since 2026-01-01 ISSUE-123
```
If the embedded comment history is incomplete, `jit` fetches its remaining pages before applying date filters and limits.

7. List current sprint tickets assigned to current user:
```bash
jit
jit --my-tickets
jit --my-tickets --include-prs
jit --my-tickets --limit 5
```
8. Create an issue (assigned to the authenticated Jira user in the current sprint by default):
```bash
jit create --project RW --summary "Improve ticket creation flow"
jit create --project RW --backlog --type Story --summary "Support backlog ticket creation" --description $'Add a create command\nCover it with tests'
jit create --project RW --backlog --type Bug --assignee 5b10a2844c20165700ede21g --summary "Fix backlog create validation"
jit create --project RW --backlog --summary "Save for later"
jit create --project RW --current-sprint --summary "Deliver current sprint ticket creation"
jit create --project RW --current-sprint --board 123 --summary "Use the board's active sprint"
jit create --project RW --summary "Improve ticket creation flow" --json
```
9. Edit an existing issue or task:
```bash
jit edit RW-123 --summary "Improve edit flow"
jit edit RW-123 --description $'First line\nSecond line'
jit edit RW-123 --description ''
jit edit RW-123 --type Bug --assignee unassigned
jit edit RW-123 --type Task --summary "Refine task summary"
jit edit RW-123 --assignee me
jit edit RW-123 --current-sprint
jit edit RW-123 --current-sprint --board 123
jit edit https://your-company.atlassian.net/browse/RW-123 --current-sprint --json
jit edit RW-123 --summary "Improve edit flow" --json
```
10. Use a specific config file:
```bash
jit auth
jit --config-file /path/to/config.toml auth
jit --config-file /path/to/config.toml ISSUE-123
jit --config-file /path/to/config.toml --my-tickets
jit --config-file /path/to/config.toml create --project RW --summary "Improve ticket creation flow"
jit --config-file /path/to/config.toml edit RW-123 --summary "Improve edit flow"
```

11. Search issues and discover boards/sprints:
```bash
jit search --jql 'project = RW AND statusCategory != Done' --limit 50 --json
jit --my-tickets --json
jit boards --project RW --json
jit sprints --board 123 --state active,future --json
```
Search and my-tickets JSON use `{ "issues": [], "truncated": false }`. Both follow pagination up to the limit; check `truncated` before claiming the results are complete. Board project filtering is optional. Sprint states are `active`, `future`, `closed`, or a comma-separated combination.
12. Change status, add comments, or link issues:
```bash
jit transitions RW-123 --json
jit transition RW-123 --to 'In Progress' --json
jit comment RW-123 --body 'Verified the fix locally.' --json
jit comment RW-123 --body-file /path/to/comment.txt --json
jit link RW-123 --blocks RW-456 --json
jit link RW-123 --to RW-456 --type 'Relates' --json
```
List available transitions first; `--to` accepts a transition name or ID. The CLI does not supply transition-specific required fields (for example, resolution); Jira validation errors are surfaced. Comments require exactly one of `--body` or `--body-file`; `--body-file -` reads stdin. `--blocks` means SOURCE blocks TARGET. Generic links accept a Jira link-type name or ID; SOURCE displays the type's outward relationship to TARGET.
13. Supply descriptions from files or stdin:
```bash
jit create --project RW --summary 'Improve validation' --description-file /path/to/description.txt
jit edit RW-123 --description-file /path/to/description.txt
printf '%s' 'Updated details' | jit edit RW-123 --description-file -
```
`--description-file` conflicts with inline `--description` for both create and edit.

## Execution Workflow

1. Determine the user intent:
- Single ticket summary
- Detailed ticket inspection
- Ticket description/comments/timestamps
- Linked GitHub pull requests
- Current sprint ticket list
- Backlog ticket creation
- Current sprint ticket creation
- Ticket or task editing
- JQL search, board/sprint discovery, status transitions, comments, or issue links
- Credential setup
2. Normalize input:
- If the user provides a Jira URL, pass it directly to `jit`.
- If the user provides a ticket key, pass the key directly.
- For new tickets, gather `--project` and `--summary`; add `--type` and `--description` when provided. Default to the current sprint. Add `--backlog` only when the user explicitly requests the backlog.
- If the user wants to edit an issue or task, gather the ticket key/URL and whichever of `--summary`, `--description`, `--type`, `--assignee`, and `--current-sprint` they want to change.
- For creation, default `--assignee` to `me` unless the user asks for someone else or wants it left unassigned. For edits, pass `--assignee` only when requested.
- Creation defaults to the current sprint; `--current-sprint` is optional. For existing issues, add `edit --current-sprint` only when a sprint move is requested. Optionally add `--board <id>` when the board is known.
3. Select output mode:
- Default for human-readable summary
- `--text` for one-line output
- `--json` for integrations
- `--show` for expanded details
- `--full` for metadata + description + comments + pull requests
- `--include-prs` to include linked GitHub pull requests
- `--since YYYY-MM-DD` to filter comment history
- `create` assigns new issues to the authenticated Jira user (`me`) in the current active sprint by default. `--backlog` explicitly skips sprint assignment and conflicts with `--current-sprint` and `--board`. Do not silently fall back to backlog when no active sprint is available.
- `--assignee me` resolves to the current Jira user; other values are treated as Jira account IDs; `unassigned` skips assignee
- `--current-sprint` adds the new or existing issue to an active sprint; if no board is supplied, use the accessible Scrum board whose active sprint has the most recent `startDate`
- `edit --current-sprint` moves the existing issue without creating a duplicate; it accepts sprint-only edits and optional `--board <id>`. It derives the project from the ticket key. Resolve the sprint before any changes; combined edits save core fields before moving. A failed move exits unsuccessfully and explicitly reports saved core fields, without rollback or success JSON. Successful JSON includes `sprint`, `sprint_id`, `board`, `board_id`, and `"sprint"` in `updated_fields`.
- `edit` updates only the fields explicitly passed; it works for Jira tasks as well as other issue types, `--description ''` clears the description, and `--assignee unassigned` clears the assignee
4. Handle structured results and failures:
- Noninteractive query, issue, and discovery commands support `--json` (also `jit --json COMMAND ...`); interactive `auth` and skill installation retain their own output formats. With `--json`, success JSON goes to stdout only. Failures exit nonzero and write `{ "error": { "code": "...", "message": "...", "ticket": null, "completed_operations": [] } }` to stderr; ticket is populated when known.
- Inspect `completed_operations` before retrying create/edit. If it contains `created`, use the reported ticket with `jit edit KEY --current-sprint` and the original board when applicable. If it contains `core_fields_updated`, retry only the failed sprint move. Saved operations are not rolled back, and no success JSON is emitted on failure.
5. Execute command and return:
- The command used
- The relevant output lines or parsed fields
- Any actionable errors with a direct fix

## Troubleshooting

1. Missing credentials:
- Symptom: configuration is missing or invalid
- Fix: run `jit auth`, or provide credentials via `config.toml` or `--config-file`
2. URL parsing failure:
- Symptom: `Could not extract ticket ID from URL`
- Fix: ensure URL matches `/browse/PROJECT-123`
3. Jira API failure:
- Symptom: `JIRA API request failed with status ...`
- Fix: verify token validity, Jira base URL, and permission to view the issue
4. No sprint tickets:
- Symptom: `No tickets found in the current sprint.`
- Fix: confirm the user is assigned tickets in an active sprint
5. Invalid `--since` format:
- Symptom: `Invalid --since value ...`
- Fix: pass dates as `YYYY-MM-DD`
6. Create command rejected by Jira:
- Symptom: `JIRA API request failed with status ...` during `jit create`
- Fix: verify the project key, issue type name, assignee account ID, and whether the Jira project requires additional custom fields on create
7. Current sprint resolution failed:
- Symptom: no active sprint found, or the wrong board was selected for `--current-sprint`
- Fix: pass `--board <id>` explicitly or start a sprint in Jira. For creation, use `--backlog` only if the user requests backlog placement; omitting `--current-sprint` still targets the sprint. For edits, omit `--current-sprint` if no sprint move is wanted.
8. Partial creation failure:
- Symptom: the issue was created but could not be added to the sprint
- Fix: inspect the reported ticket and `completed_operations` (`created`), then use `jit edit ISSUE-123 --current-sprint` with the original board when applicable; do not repeat creation and make a duplicate
9. Edit command rejected by Jira:
- Symptom: `JIRA API request failed with status ...` during `jit edit`
- Fix: verify the issue key, edited field values, assignee account ID, and whether the Jira project restricts issue type or assignee transitions

## Response Style

1. Prefer concrete commands over abstract guidance.
2. Include full command examples the user can run immediately.
3. Never print or expose `JIRA_API_TOKEN` values in outputs.
4. When `--json` is requested, preserve JSON exactly and avoid additional prose around the payload.
5. For creation requests, be explicit about whether the issue stays in the backlog or is added to the current sprint, mention that assignee defaults to the current Jira user, and note that board/sprint visibility still depends on Jira configuration.
