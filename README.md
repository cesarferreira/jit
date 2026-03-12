# JIT: JIRA Issue Tool

> A Rust CLI tool to fetch Jira ticket summaries, details, comments, sprint tickets, and create or edit issues via the Jira API.

![demo](assets/ss-3.png)

## Installation

```bash
cargo install jit-cli
```

This installs the `jit` command.

## Usage

### Standard output
```bash
# Using a ticket ID
jit ISSUE-123

# Using a JIRA URL
jit https://your-company.atlassian.net/browse/ISSUE-123
```

Example output:
```
Ticket:   ISSUE-123
Summary:  Fix the login button in Safari
```

### Text output (compact format)
```bash
jit --text ISSUE-123
```

Example output:
```
ISSUE-123: Fix the login button in Safari
```

### JSON output
```bash
# Using a ticket ID
jit --json ISSUE-123

# Using a JIRA URL
jit --json https://your-company.atlassian.net/browse/ISSUE-123

# Include description, metadata, and recent comments
jit --json --full ISSUE-123

# Include linked pull requests
jit --json --include-prs ISSUE-123

# Include comments from a specific date onward
jit --json --include-comments --since 2026-01-01 ISSUE-123
```

Example output:
```json
{"ticket":"ISSUE-123","summary":"Fix the login button in Safari"}
```

### Detailed Information (Table)
View detailed information about a ticket in a well-formatted table:

```bash
# Using a ticket ID
jit --show ISSUE-123

# Using a JIRA URL
jit --show https://your-company.atlassian.net/browse/ISSUE-123

# Include comments (latest 5 by default)
jit --show --include-comments ISSUE-123

# Include all comments and description
jit --show --full --all-comments ISSUE-123

# Include linked pull requests
jit --show --include-prs ISSUE-123

# Include only comments after a date
jit --show --include-comments --since 2026-01-01 ISSUE-123
```

### Detail and Comment Flags
Use these flags with `--show` or `--json` when you need more than key+summary:

```bash
# Include description only
jit --show --include-description ISSUE-123

# Include comments only (latest 5 by default)
jit --show --include-comments ISSUE-123

# Include description + comments + metadata/timestamps
jit --show --full ISSUE-123

# Include pull requests
jit --show --include-prs ISSUE-123

# Limit number of returned comments
jit --json --include-comments --comments-limit 3 ISSUE-123

# Return all comments
jit --json --include-comments --all-comments ISSUE-123

# Filter comments by creation date (inclusive)
jit --json --include-comments --since 2026-01-01 ISSUE-123
```

`--full` is equivalent to combining `--include-description`, `--include-comments`, and `--include-prs` for rich ticket output.

Example output:
```
TICKET DETAILS

ISSUE-123: Fix the login button in Safari

Type:       Bug                  Priority:   Medium
Status:     In Progress          Sprint:     Development Sprint 27
Assignee:   John Doe             Reporter:   Jane Smith
Created:    2023-09-15           Updated:    2023-09-16
Due Date:   2023-09-30

DESCRIPTION

The login button doesn't work properly in Safari browsers.
Steps to reproduce:
1. Open the login page in Safari
2. Click on the login button
3. Nothing happens

Expected: The login form should be submitted.
Actual: Nothing happens when the button is clicked.
```

### Current Sprint Tickets
View your tickets in the current active sprint:

```bash
# Equivalent default behavior (no args)
jit

# View all your tickets in the current sprint
jit --my-tickets

# Show sprint tickets with linked PR IDs
jit --my-tickets --include-prs

# Limit the number of tickets shown
jit --my-tickets --limit 5
```

Example output:
```
Current Sprint: Development Sprint 27

+-----------+----------------------------------+-------------------+
| Key       | Summary                          | Status            |
+-----------+----------------------------------+-------------------+
| PROJ-123  | Implement new login page         | In Review         |
+-----------+----------------------------------+-------------------+
| PROJ-124  | Fix responsiveness on dashboard  | In Progress       |
+-----------+----------------------------------+-------------------+
| PROJ-125  | Update API documentation         | Done              |
+-----------+----------------------------------+-------------------+
```

### Create Backlog Tickets
Create a new issue without sprint assignment so it lands in the backlog on scrum boards:

```bash
# Create a task in the backlog
jit create --project RW --summary "Improve ticket creation flow"

# Create a story with a plain-text description
jit create \
  --project RW \
  --type Story \
  --summary "Support backlog ticket creation" \
  --description $'Add a create command\nCover it with tests'

# Create a bug and assign it to a specific Jira account ID
jit create \
  --project RW \
  --type Bug \
  --assignee 5b10a2844c20165700ede21g \
  --summary "Fix backlog create validation"

# Create a story directly in the current sprint
jit create \
  --project RW \
  --type Story \
  --current-sprint \
  --summary "Deliver current sprint ticket creation"

# Create in the current sprint for a specific board
jit create \
  --project RW \
  --current-sprint \
  --board 123 \
  --summary "Use the board's active sprint"

# Return the created issue as JSON
jit create --project RW --summary "Improve ticket creation flow" --json
```

Example output:
```
Created:  RW-123
Project:  RW
Type:     Task
Assignee: Cesar Ferreira
Summary:  Improve ticket creation flow
Backlog:  Yes (created without sprint assignment)
URL:      https://your-company.atlassian.net/browse/RW-123
```

The command creates the issue through Jira's issue-create API and does not assign it to a sprint unless you pass `--current-sprint`. On Scrum boards, that backlog-by-default behavior is what leaves the issue in the backlog. By default, `jit create` assigns the issue to the current Jira user with `--assignee me`; pass `--assignee <account-id>` to assign someone else, or `--assignee unassigned` to leave it unassigned.

When `--current-sprint` is set, `jit` resolves the active sprint from Jira Software and adds the new issue to it after creation. If you pass `--board <id>`, that board is used directly. Otherwise, `jit` looks at accessible Scrum boards for the project and picks the active sprint with the most recent `startDate`. If your Jira project uses custom workflows or board rules, sprint visibility and backlog behavior still depend on that Jira configuration.

### Edit Existing Tickets
Update an existing issue's summary, description, type, or assignee:

```bash
# Update the summary
jit edit RW-123 --summary "Improve edit flow"

# Update the description
jit edit RW-123 --description $'First line\nSecond line'

# Clear the description
jit edit RW-123 --description ''

# Change issue type and assignee
jit edit RW-123 --type Bug --assignee 5b10a2844c20165700ede21g

# Unassign the issue
jit edit RW-123 --assignee unassigned

# Use the current Jira user as assignee
jit edit RW-123 --assignee me

# Return the update result as JSON
jit edit RW-123 --summary "Improve edit flow" --json
```

Example output:
```
Updated:  RW-123
Fields:   summary
Summary:  Improve edit flow
URL:      https://your-company.atlassian.net/browse/RW-123
```

`jit edit` updates only the fields you pass. Description values are sent as Atlassian Document Format, `--description ''` clears the description, and `--assignee unassigned` clears the assignee. Like `create`, `--assignee me` resolves to the current Jira user before sending the update.


## Configuration

The tool looks for JIRA credentials in the following locations (in order):

1. Custom environment file specified with `--env-file` option
2. `.env` file in the current directory
3. `.env` file in `~/.config/jit/` directory
4. Environment variables set in your shell

When running for the first time, create a `.env` file in your home directory at `~/.config/jit/.env` with:

```
JIRA_BASE_URL=https://your-company.atlassian.net
JIRA_API_TOKEN=your_api_token_here
JIRA_USER_EMAIL=your_email@example.com
```

With this configuration, you can run the tool from any directory on your system.

## Setup

1. Clone the repository
2. Create a `.env` file in the root directory with the following variables:
   ```
   JIRA_BASE_URL=https://your-company.atlassian.net
   JIRA_API_TOKEN=your_api_token_here
   JIRA_USER_EMAIL=your_email@example.com
   ```
3. Get a JIRA API token from [Atlassian's API tokens page](https://id.atlassian.com/manage-profile/security/api-tokens)
4. Run `cargo build --release`

## API Token Creation

1. Go to https://id.atlassian.com/manage-profile/security/api-tokens
2. Click "Create API token"
3. Give it a name like "JIRA Title CLI"
4. Copy the token and save it in your `.env` file 
