# Skills for `jit`

This repository uses one primary Codex skill for Jira workflows:

## `jit` skill (Jira CLI usage)

- Skill file: `/Users/cesarferreira/.codex/skills/jit/SKILL.md`
- Purpose: help users run `jit` to query Jira tickets, inspect details, list sprint tickets, create backlog issues, and create tickets directly in the current sprint.
  It also supports editing existing issue summary, description, type, and assignee fields.
- Trigger examples:
  - "Show me the summary for `PROJ-123`"
  - "Get ticket details with comments"
  - "Show linked GitHub PRs for my Jira tickets"
  - "List my current sprint tickets"
  - "Give me JSON for this Jira issue"
  - "Create a backlog ticket in Jira"
  - "Create a Jira ticket in the current sprint"
  - "Edit a Jira ticket"

### Common commands this skill should suggest

```bash
jit PROJ-123
jit --text PROJ-123
jit --json PROJ-123
jit --show --full PROJ-123
jit --show --include-prs PROJ-123
jit --my-tickets --include-prs
jit --show --include-comments --comments-limit 5 PROJ-123
jit --json --include-comments --since 2026-01-01 PROJ-123
jit create --project PROJ --summary "Improve ticket creation flow"
jit create --project PROJ --type Story --summary "Support backlog ticket creation" --description $'Add a create command\nCover it with tests'
jit create --project PROJ --type Bug --assignee 5b10a2844c20165700ede21g --summary "Fix backlog create validation"
jit create --project PROJ --current-sprint --summary "Deliver current sprint ticket creation"
jit create --project PROJ --current-sprint --board 123 --summary "Use the board's active sprint"
jit create --project PROJ --summary "Improve ticket creation flow" --json
jit edit PROJ-123 --summary "Improve edit flow"
jit edit PROJ-123 --description $'First line\nSecond line'
jit edit PROJ-123 --description ''
jit edit PROJ-123 --type Bug --assignee unassigned
jit edit PROJ-123 --summary "Improve edit flow" --json
jit
```

## Maintenance note

When CLI flags change in `src/main.rs`, keep these in sync:

1. `README.md`
2. This `SKILLS.md`
3. `/Users/cesarferreira/.codex/skills/jit/SKILL.md`
