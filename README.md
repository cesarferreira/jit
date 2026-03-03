# JIT: JIRA Issue Tool

> A Rust CLI tool to fetch Jira ticket summaries, details, comments, and sprint tickets from the Jira API.

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
