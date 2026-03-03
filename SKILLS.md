# Skills for `jit`

This repository uses one primary Codex skill for Jira workflows:

## `jit` skill (Jira CLI usage)

- Skill file: `/Users/cesarferreira/.codex/skills/jit/SKILL.md`
- Purpose: help users run `jit` to query Jira tickets, inspect details, and list sprint tickets.
- Trigger examples:
  - "Show me the summary for `PROJ-123`"
  - "Get ticket details with comments"
  - "Show linked GitHub PRs for my Jira tickets"
  - "List my current sprint tickets"
  - "Give me JSON for this Jira issue"

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
jit
```

## Maintenance note

When CLI flags change in `src/main.rs`, keep these in sync:

1. `README.md`
2. This `SKILLS.md`
3. `/Users/cesarferreira/.codex/skills/jit/SKILL.md`
