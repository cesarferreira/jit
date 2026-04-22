---
name: jit
description: Use the `jit` CLI to interact with Jira issues from the terminal. Trigger when users ask to fetch issue summaries from ticket keys or Jira URLs, show detailed ticket fields, include description/comments/timestamps, include linked GitHub pull requests, filter comments by date, list current sprint tickets assigned to the current user, create backlog tickets, create tickets in the current sprint, edit existing issues or tasks, format output as table/text/JSON, or configure Jira credentials for `jit`.
---

# Jit CLI

## Overview

Use this skill to execute `jit` for common Jira workflows: ticket lookup, detailed issue inspection, current sprint ticket listing, backlog ticket creation, current-sprint ticket creation, ticket/task editing, and structured output for scripts.

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
7. List current sprint tickets assigned to current user:
```bash
jit
jit --my-tickets
jit --my-tickets --include-prs
jit --my-tickets --limit 5
```
8. Create a backlog issue:
```bash
jit create --project RW --summary "Improve ticket creation flow"
jit create --project RW --type Story --summary "Support backlog ticket creation" --description $'Add a create command\nCover it with tests'
jit create --project RW --type Bug --assignee 5b10a2844c20165700ede21g --summary "Fix backlog create validation"
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
jit edit RW-123 --summary "Improve edit flow" --json
```
10. Use a specific config file:
```bash
jit --config-file /path/to/config.toml ISSUE-123
jit --config-file /path/to/config.toml --my-tickets
jit --config-file /path/to/config.toml create --project RW --summary "Improve ticket creation flow"
jit --config-file /path/to/config.toml edit RW-123 --summary "Improve edit flow"
```

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
2. Normalize input:
- If the user provides a Jira URL, pass it directly to `jit`.
- If the user provides a ticket key, pass the key directly.
- If the user wants a new backlog item, gather `--project` and `--summary`; add `--type` and `--description` when provided.
- If the user wants to edit an issue or task, gather the ticket key/URL and whichever of `--summary`, `--description`, `--type`, and `--assignee` they want to change.
- Default `--assignee` to `me` unless the user asks for someone else or wants it left unassigned.
- If the user wants the new issue in the current sprint, add `--current-sprint`; optionally add `--board <id>` when the board is known.
3. Select output mode:
- Default for human-readable summary
- `--text` for one-line output
- `--json` for integrations
- `--show` for expanded details
- `--full` for metadata + description + comments + pull requests
- `--include-prs` to include linked GitHub pull requests
- `--since YYYY-MM-DD` to filter comment history
- `create` to create a new Jira issue without sprint assignment so it lands in the backlog on scrum boards
- `--assignee me` resolves to the current Jira user; other values are treated as Jira account IDs; `unassigned` skips assignee
- `--current-sprint` adds the new issue to an active sprint after creation; if no board is supplied, use the accessible Scrum board whose active sprint has the most recent `startDate`
- `edit` updates only the fields explicitly passed; it works for Jira tasks as well as other issue types, `--description ''` clears the description, and `--assignee unassigned` clears the assignee
4. Execute command and return:
- The command used
- The relevant output lines or parsed fields
- Any actionable errors with a direct fix

## Troubleshooting

1. Missing credentials:
- Symptom: configuration is missing or invalid
- Fix: provide credentials via `config.toml` or `--config-file`
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
- Fix: pass `--board <id>` explicitly, or create without `--current-sprint` if the issue should stay in the backlog
8. Edit command rejected by Jira:
- Symptom: `JIRA API request failed with status ...` during `jit edit`
- Fix: verify the issue key, edited field values, assignee account ID, and whether the Jira project restricts issue type or assignee transitions

## Response Style

1. Prefer concrete commands over abstract guidance.
2. Include full command examples the user can run immediately.
3. Never print or expose `JIRA_API_TOKEN` values in outputs.
4. When `--json` is requested, preserve JSON exactly and avoid additional prose around the payload.
5. For creation requests, be explicit about whether the issue stays in the backlog or is added to the current sprint, mention that assignee defaults to the current Jira user, and note that board/sprint visibility still depends on Jira configuration.
