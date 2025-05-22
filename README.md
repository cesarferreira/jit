# JIT -> JIRA Issue Tool

A simple Rust CLI tool to extract ticket ID and summary information from JIRA issues using the JIRA API.


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
```

Example output:
```json
{"ticket":"ISSUE-123","summary":"Fix the login button in Safari"}
```


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