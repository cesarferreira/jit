// src/main.rs
use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, FixedOffset};
use clap::{Args, Parser, Subcommand};
use colored::*;
use dotenv::dotenv;
use regex::Regex;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[command(flatten)]
    query: QueryArgs,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new Jira issue without sprint assignment so it lands in the backlog on scrum boards
    Create(CreateArgs),
    /// Edit an existing Jira ticket's core fields, including Task issues
    Edit(EditArgs),
}

#[derive(Args, Debug)]
struct QueryArgs {
    /// JIRA issue key (e.g., RW-1931) or URL (e.g., https://company.atlassian.net/browse/RW-1931)
    ticket: Option<String>,

    /// Output in JSON format
    #[clap(long)]
    json: bool,

    /// Output as plain text in format "KEY: Summary"
    #[clap(long)]
    text: bool,

    /// Display your current tickets in a table (default when no ticket is provided)
    #[clap(long)]
    my_tickets: bool,

    /// Show detailed information about a ticket in a table format
    #[clap(long)]
    show: bool,

    /// Include ticket description in detailed output
    #[clap(long)]
    include_description: bool,

    /// Include ticket comments in detailed output
    #[clap(long)]
    include_comments: bool,

    /// Include linked GitHub pull requests
    #[clap(long)]
    include_prs: bool,

    /// Include description, comments, pull requests, and metadata in detailed output
    #[clap(long)]
    full: bool,

    /// Maximum number of comments to show (default: 5)
    #[clap(long, default_value = "5")]
    comments_limit: usize,

    /// Show all comments (overrides --comments-limit)
    #[clap(long)]
    all_comments: bool,

    /// Only include comments created on or after YYYY-MM-DD
    #[clap(long)]
    since: Option<String>,

    /// Maximum number of tickets to retrieve (default: 10)
    #[clap(long, default_value = "10")]
    limit: u32,

    /// Path to a custom .env file
    #[clap(long)]
    env_file: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct CreateArgs {
    /// Jira project key (e.g., RW)
    #[clap(long)]
    project: String,

    /// Ticket summary
    #[clap(long)]
    summary: String,

    /// Plain-text ticket description
    #[clap(long)]
    description: Option<String>,

    /// Jira issue type name, such as Task, Bug, or Story (default: Task)
    #[clap(long = "type", default_value = "Task")]
    issue_type: String,

    /// Assignee account ID, or `me` to assign to the current Jira user (default: me)
    #[clap(long, default_value = "me")]
    assignee: String,

    /// Add the created ticket to the current active sprint instead of leaving it in the backlog
    #[clap(long)]
    current_sprint: bool,

    /// Jira board ID to use for current sprint selection; otherwise the most recently started active sprint is used
    #[clap(long, requires = "current_sprint")]
    board: Option<u64>,

    /// Output created issue details in JSON format
    #[clap(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct EditArgs {
    /// JIRA issue key (e.g., RW-1931) or URL (e.g., https://company.atlassian.net/browse/RW-1931)
    ticket: String,

    /// Updated ticket summary
    #[clap(long)]
    summary: Option<String>,

    /// Updated plain-text ticket description; pass an empty string to clear it
    #[clap(long)]
    description: Option<String>,

    /// Updated Jira issue type name, such as Task, Bug, or Story
    #[clap(long = "type")]
    issue_type: Option<String>,

    /// Updated assignee account ID, or `me` / `unassigned`
    #[clap(long)]
    assignee: Option<String>,

    /// Output updated issue details in JSON format
    #[clap(long)]
    json: bool,
}

#[derive(Debug, Deserialize)]
struct JiraIssue {
    id: String,
    key: String,
    fields: JiraIssueFields,
}

#[derive(Debug, Deserialize)]
struct JiraIssueFields {
    summary: String,
    #[serde(default)]
    status: Option<JiraStatus>,
    #[serde(rename = "customfield_10020", default)]
    sprint: Option<Vec<JiraSprint>>,
    #[serde(default)]
    description: Option<Value>,
    #[serde(default)]
    assignee: Option<JiraUser>,
    #[serde(default)]
    reporter: Option<JiraUser>,
    #[serde(default)]
    priority: Option<JiraPriority>,
    #[serde(default)]
    issuetype: Option<JiraIssueType>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(rename = "duedate", default)]
    due_date: Option<String>,
    #[serde(default)]
    comment: Option<JiraCommentContainer>,
}

#[derive(Debug, Deserialize, Default)]
struct JiraStatus {
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct JiraSprint {
    name: String,
    state: String,
}

#[derive(Debug, Deserialize, Default)]
struct JiraUser {
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "accountId", default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct JiraPriority {
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct JiraIssueType {
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct JiraCommentContainer {
    #[serde(default)]
    comments: Vec<JiraComment>,
}

#[derive(Debug, Deserialize, Default)]
struct JiraComment {
    #[serde(default)]
    author: Option<JiraUser>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    updated: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct JiraDevStatusResponse {
    #[serde(default)]
    detail: Vec<JiraDevStatusDetail>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct JiraDevStatusDetail {
    #[serde(rename = "pullRequests", default)]
    pull_requests: Vec<JiraPullRequest>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct JiraPullRequest {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(rename = "lastUpdate", default)]
    last_update: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JiraSearchResponse {
    issues: Vec<JiraIssue>,
}

#[derive(Debug, Deserialize)]
struct JiraCreatedIssue {
    id: String,
    key: String,
}

#[derive(Debug, Deserialize)]
struct JiraBoardPage {
    #[serde(default)]
    values: Vec<JiraBoard>,
    #[serde(rename = "isLast", default)]
    is_last: bool,
    #[serde(rename = "maxResults", default)]
    max_results: usize,
    #[serde(rename = "startAt", default)]
    start_at: usize,
}

#[derive(Debug, Deserialize, Clone)]
struct JiraBoard {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct JiraSprintPage {
    #[serde(default)]
    values: Vec<JiraAgileSprint>,
    #[serde(rename = "isLast", default)]
    is_last: bool,
    #[serde(rename = "maxResults", default)]
    max_results: usize,
    #[serde(rename = "startAt", default)]
    start_at: usize,
}

#[derive(Debug, Deserialize, Clone)]
struct JiraAgileSprint {
    id: u64,
    name: String,
    #[serde(rename = "startDate", default)]
    start_date: Option<String>,
}

fn main() -> Result<()> {
    let args = Cli::parse();

    if let Some(since) = args.query.since.as_deref() {
        validate_since_date(since)?;
    }

    // Load environment variables from multiple locations
    load_environment_variables(&args.query);

    // Get Jira API credentials from environment
    let jira_base_url = env::var("JIRA_BASE_URL")
        .context("JIRA_BASE_URL not set. Set it in a .env file or as an environment variable")?;
    let jira_api_token = env::var("JIRA_API_TOKEN")
        .context("JIRA_API_TOKEN not set. Set it in a .env file or as an environment variable")?;
    let jira_user_email = env::var("JIRA_USER_EMAIL")
        .context("JIRA_USER_EMAIL not set. Set it in a .env file or as an environment variable")?;

    // Create HTTP client for JIRA API
    let client = create_jira_client(&jira_user_email, &jira_api_token)?;

    match args.command {
        Some(Commands::Create(create_args)) => {
            run_create_issue_command(&client, &jira_base_url, &create_args)
        }
        Some(Commands::Edit(edit_args)) => {
            run_edit_issue_command(&client, &jira_base_url, &edit_args)
        }
        None => run_query_mode(&client, &jira_base_url, args.query),
    }
}

fn run_query_mode(client: &Client, jira_base_url: &str, args: QueryArgs) -> Result<()> {
    if args.my_tickets || args.ticket.is_none() {
        // Fetch and display current tickets
        let tickets = fetch_my_tickets(client, jira_base_url, args.limit)?;
        let include_prs = args.include_prs || args.full;
        let pull_requests_by_key = if include_prs {
            Some(fetch_pull_requests_for_tickets(
                client,
                jira_base_url,
                &tickets,
            )?)
        } else {
            None
        };
        display_tickets_table(&tickets, pull_requests_by_key.as_ref())?;
    } else if let Some(ticket_input) = args.ticket {
        // Extract ticket ID from URL if needed
        let ticket_id = extract_ticket_id(&ticket_input)?;

        let include_description = args.show || args.full || args.include_description;
        let include_comments = args.full || args.include_comments;
        let include_prs = args.full || args.include_prs;
        let include_details = args.show
            || args.full
            || args.include_description
            || args.include_comments
            || args.include_prs;

        // Fetch issue details based on requested output mode.
        let issue = fetch_jira_issue(
            client,
            jira_base_url,
            &ticket_id,
            include_details,
            include_description,
            include_comments,
        )?;

        let pull_requests = if include_prs {
            fetch_issue_pull_requests(client, jira_base_url, &issue.id)?
        } else {
            Vec::new()
        };

        // Output the result
        if args.json {
            if include_details || include_description || include_comments {
                let payload = build_issue_json(
                    &issue,
                    include_description,
                    include_comments,
                    include_prs,
                    &pull_requests,
                    args.comments_limit,
                    args.all_comments,
                    args.since.as_deref(),
                );
                println!("{}", payload);
            } else {
                println!(
                    "{}",
                    json!({
                        "ticket": issue.key,
                        "summary": issue.fields.summary
                    })
                );
            }
        } else if args.text {
            println!("{}: {}", issue.key, issue.fields.summary);
        } else if args.show
            || args.full
            || args.include_description
            || args.include_comments
            || args.include_prs
        {
            display_detailed_ticket(
                &issue,
                include_description,
                include_comments,
                include_prs,
                &pull_requests,
                args.comments_limit,
                args.all_comments,
                args.since.as_deref(),
            )?;
        } else {
            println!("Ticket:   {}", issue.key);
            println!("Summary:  {}", issue.fields.summary);
        }
    }

    Ok(())
}

/// Attempts to load environment variables from multiple locations in order:
/// 1. Custom env file passed as an argument
/// 2. Current directory .env
/// 3. User's home directory ~/.config/jit/.env
fn load_environment_variables(args: &QueryArgs) {
    // First try user-specified env file if provided
    if let Some(env_path) = &args.env_file {
        if env_path.exists() {
            dotenv::from_path(env_path).ok();
            return;
        } else {
            eprintln!("Warning: Specified .env file not found at: {:?}", env_path);
        }
    }

    // Try the current directory
    dotenv().ok();

    // If the vars aren't set yet, try in the home directory
    if env::var("JIRA_BASE_URL").is_err()
        || env::var("JIRA_API_TOKEN").is_err()
        || env::var("JIRA_USER_EMAIL").is_err()
    {
        if let Some(home_dir) = dirs::home_dir() {
            let config_dir = home_dir.join(".config").join("jit");
            let home_env_path = config_dir.join(".env");

            if home_env_path.exists() {
                dotenv::from_path(&home_env_path).ok();
            } else {
                // If no config file exists yet, create the directory for future use
                if !config_dir.exists() {
                    if let Ok(_) = std::fs::create_dir_all(&config_dir) {
                        eprintln!(
                            "No configuration found. Created directory at: {:?}",
                            config_dir
                        );
                        eprintln!(
                            "Please create a .env file in this directory with your JIRA credentials:"
                        );
                        eprintln!("  JIRA_BASE_URL=https://your-company.atlassian.net");
                        eprintln!("  JIRA_API_TOKEN=your_api_token_here");
                        eprintln!("  JIRA_USER_EMAIL=your_email@example.com");
                    }
                }
            }
        }
    }
}

fn run_create_issue_command(client: &Client, jira_base_url: &str, args: &CreateArgs) -> Result<()> {
    let resolved_assignee = resolve_create_assignee(client, jira_base_url, &args.assignee)?;
    let resolved_sprint = resolve_target_sprint(client, jira_base_url, args)?;
    let created_issue = create_jira_issue(
        client,
        jira_base_url,
        args,
        resolved_assignee.account_id.as_deref(),
    )?;
    if let Some(sprint) = resolved_sprint.as_ref() {
        add_issue_to_sprint(client, jira_base_url, sprint.id, &created_issue.key).with_context(
            || {
                format!(
                    "Created {} but failed to add it to sprint {}",
                    created_issue.key, sprint.name
                )
            },
        )?;
    }
    let issue_url = format!("{}/browse/{}", jira_base_url, created_issue.key);

    if args.json {
        let mut payload = json!({
            "id": created_issue.id,
            "ticket": created_issue.key,
            "project": args.project,
            "summary": args.summary,
            "issue_type": args.issue_type,
            "assignee": resolved_assignee.label,
            "url": issue_url,
            "backlog": resolved_sprint.is_none(),
        });
        if let Some(sprint) = resolved_sprint {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("sprint".to_string(), json!(sprint.name));
                obj.insert("board".to_string(), json!(sprint.board_name));
            }
        }
        println!("{}", payload);
    } else {
        println!("Created:  {}", created_issue.key);
        println!("Project:  {}", args.project);
        println!("Type:     {}", args.issue_type);
        println!("Assignee: {}", resolved_assignee.label);
        println!("Summary:  {}", args.summary);
        if let Some(sprint) = resolved_sprint {
            println!("Board:    {}", sprint.board_name);
            println!("Sprint:   {}", sprint.name);
            println!("Backlog:  No (added to current sprint)");
        } else {
            println!("Backlog:  Yes (created without sprint assignment)");
        }
        println!("URL:      {}", issue_url);
    }

    Ok(())
}

fn run_edit_issue_command(client: &Client, jira_base_url: &str, args: &EditArgs) -> Result<()> {
    if args.summary.is_none()
        && args.description.is_none()
        && args.issue_type.is_none()
        && args.assignee.is_none()
    {
        return Err(anyhow!(
            "No editable fields provided. Pass at least one of --summary, --description, --type, or --assignee."
        ));
    }

    let ticket_id = extract_ticket_id(&args.ticket)?;
    let resolved_assignee = args
        .assignee
        .as_deref()
        .map(|assignee| resolve_create_assignee(client, jira_base_url, assignee))
        .transpose()?;
    update_jira_issue(
        client,
        jira_base_url,
        &ticket_id,
        args,
        resolved_assignee
            .as_ref()
            .and_then(|assignee| assignee.account_id.as_deref()),
    )?;

    let issue_url = format!("{}/browse/{}", jira_base_url, ticket_id);
    let mut updated_fields = Vec::new();
    if args.summary.is_some() {
        updated_fields.push("summary");
    }
    if args.description.is_some() {
        updated_fields.push("description");
    }
    if args.issue_type.is_some() {
        updated_fields.push("issue_type");
    }
    if args.assignee.is_some() {
        updated_fields.push("assignee");
    }

    if args.json {
        let mut payload = json!({
            "ticket": ticket_id,
            "updated_fields": updated_fields,
            "url": issue_url,
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(summary) = args.summary.as_ref() {
                obj.insert("summary".to_string(), json!(summary));
            }
            if let Some(description) = args.description.as_ref() {
                obj.insert(
                    "description".to_string(),
                    if description.trim().is_empty() {
                        Value::Null
                    } else {
                        json!(description)
                    },
                );
            }
            if let Some(issue_type) = args.issue_type.as_ref() {
                obj.insert("issue_type".to_string(), json!(issue_type));
            }
            if let Some(assignee) = resolved_assignee.as_ref() {
                obj.insert("assignee".to_string(), json!(assignee.label));
            }
        }

        println!("{}", payload);
    } else {
        println!("Updated:  {}", ticket_id);
        println!("Fields:   {}", updated_fields.join(", "));
        if let Some(summary) = args.summary.as_ref() {
            println!("Summary:  {}", summary);
        }
        if let Some(issue_type) = args.issue_type.as_ref() {
            println!("Type:     {}", issue_type);
        }
        if let Some(assignee) = resolved_assignee.as_ref() {
            println!("Assignee: {}", assignee.label);
        }
        if let Some(description) = args.description.as_ref() {
            println!(
                "Description: {}",
                if description.trim().is_empty() {
                    "Cleared"
                } else {
                    "Updated"
                }
            );
        }
        println!("URL:      {}", issue_url);
    }

    Ok(())
}

fn extract_ticket_id(input: &str) -> Result<String> {
    // If input starts with http/https, it's a URL
    if input.starts_with("http://") || input.starts_with("https://") {
        // Use regex to extract the ticket ID from the URL
        let re = Regex::new(r"/browse/([A-Z]+-\d+)(?:/|$)")?;
        if let Some(captures) = re.captures(input) {
            if let Some(ticket_match) = captures.get(1) {
                return Ok(ticket_match.as_str().to_string());
            }
        }
        Err(anyhow!("Could not extract ticket ID from URL: {}", input))
    } else {
        // Input is already a ticket ID
        Ok(input.to_string())
    }
}

fn validate_since_date(since: &str) -> Result<()> {
    let re = Regex::new(r"^\d{4}-\d{2}-\d{2}$")?;
    if !re.is_match(since) {
        return Err(anyhow!(
            "Invalid --since value '{}'. Use YYYY-MM-DD.",
            since
        ));
    }
    Ok(())
}

fn create_jira_client(email: &str, api_token: &str) -> Result<Client> {
    // Create Basic Auth header
    let auth = format!("{}:{}", email, api_token);
    let encoded_auth = STANDARD.encode(auth);
    let auth_header = format!("Basic {}", encoded_auth);

    // Setup headers
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&auth_header)?);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    // Build client
    let client = ClientBuilder::new().default_headers(headers).build()?;

    Ok(client)
}

fn fetch_jira_issue(
    client: &Client,
    base_url: &str,
    issue_key: &str,
    include_details: bool,
    include_description: bool,
    include_comments: bool,
) -> Result<JiraIssue> {
    let mut fields = vec!["summary"];

    if include_details {
        fields.extend([
            "status",
            "customfield_10020",
            "assignee",
            "reporter",
            "priority",
            "issuetype",
            "created",
            "updated",
            "duedate",
        ]);
    }

    if include_description {
        fields.push("description");
    }

    if include_comments {
        fields.push("comment");
    }

    fields.sort_unstable();
    fields.dedup();

    let url = format!(
        "{}/rest/api/3/issue/{}?fields={}",
        base_url,
        issue_key,
        fields.join(",")
    );

    let response = client
        .get(&url)
        .send()
        .context("Failed to send request to JIRA API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "JIRA API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    let issue: JiraIssue = response
        .json()
        .context("Failed to parse JIRA API response")?;

    Ok(issue)
}

fn fetch_my_tickets(client: &Client, base_url: &str, limit: u32) -> Result<Vec<JiraIssue>> {
    // Jira removed /rest/api/3/search for JQL queries in favor of /search/jql.
    let url = format!("{}/rest/api/3/search/jql", base_url);

    // JQL query to find issues assigned to the current user in the active sprint
    let query = json!({
        "jql": "assignee = currentUser() AND sprint in openSprints() ORDER BY updated DESC",
        "maxResults": limit,
        "fields": ["summary", "status", "customfield_10020"]
    });

    let response = client
        .post(&url)
        .json(&query)
        .send()
        .context("Failed to send request to JIRA API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "JIRA API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    let search_result: JiraSearchResponse = response
        .json()
        .context("Failed to parse JIRA API response")?;

    Ok(search_result.issues)
}

fn create_jira_issue(
    client: &Client,
    base_url: &str,
    args: &CreateArgs,
    assignee_id: Option<&str>,
) -> Result<JiraCreatedIssue> {
    let url = format!("{}/rest/api/3/issue", base_url);
    let payload = build_issue_create_payload(args, assignee_id);

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .context("Failed to send request to JIRA API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "JIRA API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    response.json().context("Failed to parse JIRA API response")
}

fn update_jira_issue(
    client: &Client,
    base_url: &str,
    issue_key: &str,
    args: &EditArgs,
    assignee_id: Option<&str>,
) -> Result<()> {
    let url = format!("{}/rest/api/3/issue/{}", base_url, issue_key);
    let payload = build_issue_update_payload(args, assignee_id);

    let response = client
        .put(&url)
        .json(&payload)
        .send()
        .context("Failed to send request to JIRA API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "JIRA API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    Ok(())
}

fn resolve_create_assignee(
    client: &Client,
    base_url: &str,
    requested: &str,
) -> Result<ResolvedAssignee> {
    match requested.trim() {
        "" | "me" | "self" | "current" => fetch_current_user_assignee(client, base_url),
        "unassigned" => Ok(ResolvedAssignee {
            account_id: None,
            label: "Unassigned".to_string(),
        }),
        account_id => Ok(ResolvedAssignee {
            account_id: Some(account_id.to_string()),
            label: account_id.to_string(),
        }),
    }
}

fn fetch_current_user_assignee(client: &Client, base_url: &str) -> Result<ResolvedAssignee> {
    let url = format!("{}/rest/api/3/myself", base_url);
    let response = client
        .get(&url)
        .send()
        .context("Failed to send request to JIRA API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "JIRA API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    let current_user: JiraUser = response
        .json()
        .context("Failed to parse JIRA API response")?;
    let account_id = current_user
        .account_id
        .context("Current Jira user response did not include accountId")?;
    let label = if current_user.display_name.trim().is_empty() {
        "Current user".to_string()
    } else {
        current_user.display_name
    };

    Ok(ResolvedAssignee {
        account_id: Some(account_id),
        label,
    })
}

fn resolve_target_sprint(
    client: &Client,
    base_url: &str,
    args: &CreateArgs,
) -> Result<Option<ResolvedSprint>> {
    if !args.current_sprint {
        return Ok(None);
    }

    let sprint = if let Some(board_id) = args.board {
        resolve_active_sprint_for_board(client, base_url, board_id)?
    } else {
        resolve_latest_active_sprint_for_project(client, base_url, &args.project)?
    };

    Ok(Some(sprint))
}

fn resolve_latest_active_sprint_for_project(
    client: &Client,
    base_url: &str,
    project_key: &str,
) -> Result<ResolvedSprint> {
    let boards = fetch_scrum_boards_for_project(client, base_url, project_key)?;
    if boards.is_empty() {
        return Err(anyhow!(
            "No Scrum boards found for project {}. Pass --board <id> if the sprint lives on a different board.",
            project_key
        ));
    }

    let mut best: Option<ResolvedSprint> = None;
    for board in boards {
        let sprint = match fetch_active_sprints_for_board(client, base_url, board.id)? {
            Some(sprint) => ResolvedSprint {
                id: sprint.id,
                name: sprint.name,
                board_id: board.id,
                board_name: board.name,
                start_date: sprint.start_date,
            },
            None => continue,
        };

        if is_better_sprint_candidate(&sprint, best.as_ref()) {
            best = Some(sprint);
        }
    }

    best.ok_or_else(|| {
        anyhow!(
            "No active sprint found on accessible Scrum boards for project {}. Create without --current-sprint or pass --board <id>.",
            project_key
        )
    })
}

fn resolve_active_sprint_for_board(
    client: &Client,
    base_url: &str,
    board_id: u64,
) -> Result<ResolvedSprint> {
    let board = fetch_board(client, base_url, board_id)?;
    let sprint = fetch_active_sprints_for_board(client, base_url, board_id)?.ok_or_else(|| {
        anyhow!(
            "No active sprint found on board {} ({}).",
            board.id,
            board.name
        )
    })?;

    Ok(ResolvedSprint {
        id: sprint.id,
        name: sprint.name,
        board_id: board.id,
        board_name: board.name,
        start_date: sprint.start_date,
    })
}

fn fetch_scrum_boards_for_project(
    client: &Client,
    base_url: &str,
    project_key: &str,
) -> Result<Vec<JiraBoard>> {
    let mut start_at = 0;
    let mut boards = Vec::new();

    loop {
        let url = format!(
            "{}/rest/agile/1.0/board?projectKeyOrId={}&type=scrum&startAt={}&maxResults=50",
            base_url, project_key, start_at
        );
        let response = client
            .get(&url)
            .send()
            .context("Failed to send request to Jira Agile API")?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Jira Agile API request failed with status: {} - {}",
                response.status(),
                response.text().unwrap_or_default()
            ));
        }

        let page: JiraBoardPage = response
            .json()
            .context("Failed to parse Jira Agile API response")?;
        let page_size = page.values.len();
        boards.extend(page.values);

        if page.is_last || page_size == 0 {
            break;
        }

        start_at = page.start_at + page.max_results.max(page_size);
    }

    Ok(boards)
}

fn fetch_board(client: &Client, base_url: &str, board_id: u64) -> Result<JiraBoard> {
    let url = format!("{}/rest/agile/1.0/board/{}", base_url, board_id);
    let response = client
        .get(&url)
        .send()
        .context("Failed to send request to Jira Agile API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Jira Agile API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    response
        .json()
        .context("Failed to parse Jira Agile API response")
}

fn fetch_active_sprints_for_board(
    client: &Client,
    base_url: &str,
    board_id: u64,
) -> Result<Option<JiraAgileSprint>> {
    let mut start_at = 0;
    let mut best: Option<JiraAgileSprint> = None;

    loop {
        let url = format!(
            "{}/rest/agile/1.0/board/{}/sprint?state=active&startAt={}&maxResults=50",
            base_url, board_id, start_at
        );
        let response = client
            .get(&url)
            .send()
            .context("Failed to send request to Jira Agile API")?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Jira Agile API request failed with status: {} - {}",
                response.status(),
                response.text().unwrap_or_default()
            ));
        }

        let page: JiraSprintPage = response
            .json()
            .context("Failed to parse Jira Agile API response")?;
        let page_size = page.values.len();
        for sprint in page.values {
            if is_better_active_sprint(&sprint, best.as_ref()) {
                best = Some(sprint);
            }
        }

        if page.is_last || page_size == 0 {
            break;
        }

        start_at = page.start_at + page.max_results.max(page_size);
    }

    Ok(best)
}

fn add_issue_to_sprint(
    client: &Client,
    base_url: &str,
    sprint_id: u64,
    issue_key: &str,
) -> Result<()> {
    let url = format!("{}/rest/agile/1.0/sprint/{}/issue", base_url, sprint_id);
    let payload = json!({ "issues": [issue_key] });
    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .context("Failed to send request to Jira Agile API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Jira Agile API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    Ok(())
}

fn is_better_sprint_candidate(
    candidate: &ResolvedSprint,
    current: Option<&ResolvedSprint>,
) -> bool {
    match current {
        None => true,
        Some(current) => compare_sprint_identity(
            candidate.start_date.as_deref(),
            candidate.id,
            candidate.board_id,
            current.start_date.as_deref(),
            current.id,
            current.board_id,
        ),
    }
}

fn is_better_active_sprint(candidate: &JiraAgileSprint, current: Option<&JiraAgileSprint>) -> bool {
    match current {
        None => true,
        Some(current) => compare_sprint_identity(
            candidate.start_date.as_deref(),
            candidate.id,
            0,
            current.start_date.as_deref(),
            current.id,
            0,
        ),
    }
}

fn compare_sprint_identity(
    candidate_date: Option<&str>,
    candidate_sprint_id: u64,
    candidate_board_id: u64,
    current_date: Option<&str>,
    current_sprint_id: u64,
    current_board_id: u64,
) -> bool {
    let candidate_date = candidate_date.and_then(parse_jira_datetime);
    let current_date = current_date.and_then(parse_jira_datetime);

    match (candidate_date, current_date) {
        (Some(candidate), Some(current)) => {
            candidate > current
                || (candidate == current
                    && (candidate_sprint_id, candidate_board_id)
                        > (current_sprint_id, current_board_id))
        }
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => {
            (candidate_sprint_id, candidate_board_id) > (current_sprint_id, current_board_id)
        }
    }
}

fn parse_jira_datetime(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn build_issue_create_payload(args: &CreateArgs, assignee_id: Option<&str>) -> Value {
    let mut fields = serde_json::Map::from_iter([
        ("project".to_string(), json!({ "key": args.project })),
        ("summary".to_string(), json!(args.summary)),
        ("issuetype".to_string(), json!({ "name": args.issue_type })),
    ]);

    if let Some(assignee_id) = assignee_id {
        fields.insert("assignee".to_string(), json!({ "id": assignee_id }));
    }

    if let Some(description) = args
        .description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        fields.insert("description".to_string(), text_to_adf(description));
    }

    json!({ "fields": fields })
}

fn build_issue_update_payload(args: &EditArgs, assignee_id: Option<&str>) -> Value {
    let mut fields = serde_json::Map::new();

    if let Some(summary) = args.summary.as_deref() {
        fields.insert("summary".to_string(), json!(summary));
    }

    if let Some(issue_type) = args.issue_type.as_deref() {
        fields.insert("issuetype".to_string(), json!({ "name": issue_type }));
    }

    if args.assignee.is_some() {
        let assignee_value = assignee_id
            .map(|account_id| json!({ "accountId": account_id }))
            .unwrap_or(Value::Null);
        fields.insert("assignee".to_string(), assignee_value);
    }

    if let Some(description) = args.description.as_deref() {
        fields.insert(
            "description".to_string(),
            if description.trim().is_empty() {
                Value::Null
            } else {
                text_to_adf(description)
            },
        );
    }

    json!({ "fields": fields })
}

struct ResolvedAssignee {
    account_id: Option<String>,
    label: String,
}

#[derive(Clone)]
struct ResolvedSprint {
    id: u64,
    name: String,
    board_id: u64,
    board_name: String,
    start_date: Option<String>,
}

fn fetch_issue_pull_requests(
    client: &Client,
    base_url: &str,
    issue_id: &str,
) -> Result<Vec<JiraPullRequest>> {
    let url = format!(
        "{}/rest/dev-status/latest/issue/detail?issueId={}&applicationType=GitHub&dataType=pullrequest",
        base_url, issue_id
    );

    let response = client
        .get(&url)
        .send()
        .context("Failed to send request to Jira dev-status API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Jira dev-status request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    let dev_status: JiraDevStatusResponse = response
        .json()
        .context("Failed to parse Jira dev-status response")?;

    Ok(dev_status
        .detail
        .into_iter()
        .flat_map(|detail| detail.pull_requests.into_iter())
        .collect())
}

fn fetch_pull_requests_for_tickets(
    client: &Client,
    base_url: &str,
    tickets: &[JiraIssue],
) -> Result<HashMap<String, Vec<JiraPullRequest>>> {
    let mut by_key = HashMap::new();

    for ticket in tickets {
        let prs = fetch_issue_pull_requests(client, base_url, &ticket.id)
            .with_context(|| format!("Failed to fetch pull requests for {}", ticket.key))?;
        by_key.insert(ticket.key.clone(), prs);
    }

    Ok(by_key)
}

fn display_tickets_table(
    tickets: &[JiraIssue],
    pull_requests_by_key: Option<&HashMap<String, Vec<JiraPullRequest>>>,
) -> Result<()> {
    if tickets.is_empty() {
        println!("No tickets found in the current sprint.");
        return Ok(());
    }

    // Get sprint name from the first ticket
    let sprint_name = tickets[0]
        .fields
        .sprint
        .as_ref()
        .and_then(|sprints| {
            sprints
                .iter()
                .find(|s| s.state == "active")
                .or_else(|| sprints.first())
        })
        .map_or("Unknown Sprint", |s| &s.name);

    println!("Current Sprint: {}", sprint_name);
    println!();

    // Create a simple table with basic formatting
    let include_prs_column = pull_requests_by_key.is_some();
    let mut header_row = vec![
        "Key".to_string(),
        "Summary".to_string(),
        "Status".to_string(),
    ];
    if include_prs_column {
        header_row.push("PRs".to_string());
    }
    let mut table = vec![header_row];

    // Add the data rows
    for ticket in tickets {
        let status_text = ticket.fields.status.as_ref().map_or("Unknown", |s| &s.name);
        let summary = truncate_with_ellipsis(&ticket.fields.summary, 58);
        let colored_status = get_colored_status(status_text);

        let mut row = vec![ticket.key.clone(), summary, colored_status];
        if let Some(pr_map) = pull_requests_by_key {
            let prs = pr_map
                .get(&ticket.key)
                .map(|entries| format_pull_request_summary(entries))
                .unwrap_or_else(|| "-".to_string());
            row.push(prs);
        }
        table.push(row);
    }

    // Calculate column widths
    let mut col_widths = vec![20, 7, 6]; // Set Key column to fixed 20 width
    if include_prs_column {
        col_widths.push(5);
    }
    for row in &table {
        for (i, cell) in row.iter().enumerate() {
            // For status column with color codes, use the length of the plain text
            let cell_width = if i == 2 && row[0] != "Key" {
                // This is a status cell, get the original text length
                let status_text = tickets
                    [table.iter().position(|r| &r[0] == &row[0]).unwrap_or(0) - 1]
                    .fields
                    .status
                    .as_ref()
                    .map_or("Unknown", |s| &s.name);
                status_text.len()
            } else {
                cell.len()
            };

            if i < col_widths.len() {
                col_widths[i] = col_widths[i].max(cell_width + 2);
            }
        }
    }

    // Print top border
    print!("┌");
    for (i, width) in col_widths.iter().enumerate() {
        print!("{}", "─".repeat(*width));
        if i < col_widths.len() - 1 {
            print!("┬");
        }
    }
    println!("┐");

    // Print header row
    for (row_idx, row) in table.iter().enumerate() {
        print!("│");
        for (col_idx, cell) in row.iter().enumerate() {
            let cell_text = if row_idx == 0 {
                format!(" {:<width$}", cell, width = col_widths[col_idx] - 1)
            } else if col_idx == 2 {
                // Add proper spacing for colored status
                let status_text = tickets[row_idx - 1]
                    .fields
                    .status
                    .as_ref()
                    .map_or("Unknown", |s| &s.name);
                format!(
                    " {}{}",
                    cell,
                    " ".repeat(col_widths[col_idx] - status_text.len() - 1)
                )
            } else {
                format!(" {:<width$}", cell, width = col_widths[col_idx] - 1)
            };

            print!("{}", cell_text);
            print!("│");
        }
        println!();

        // Print row separator
        if row_idx < table.len() - 1 {
            print!("├");
            for (i, width) in col_widths.iter().enumerate() {
                print!("{}", "─".repeat(*width));
                if i < col_widths.len() - 1 {
                    print!("┼");
                }
            }
            println!("┤");
        }
    }

    // Print bottom border
    print!("└");
    for (i, width) in col_widths.iter().enumerate() {
        print!("{}", "─".repeat(*width));
        if i < col_widths.len() - 1 {
            print!("┴");
        }
    }
    println!("┘");

    Ok(())
}

// Truncate a string to max_len and add ellipsis if needed
fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }

    let mut result = s.chars().take(max_len - 3).collect::<String>();
    result.push_str("...");
    result
}

fn extract_pr_id_from_url(url: &str) -> Option<String> {
    let marker = "/pull/";
    let start = url.find(marker)?;
    let suffix = &url[start + marker.len()..];
    let number = suffix.split(['/', '?', '#']).next().unwrap_or("").trim();
    if number.is_empty() {
        None
    } else {
        Some(format!("#{}", number))
    }
}

fn pull_request_display_id(pr: &JiraPullRequest) -> String {
    if let Some(id) = pr.id.as_deref().filter(|id| !id.trim().is_empty()) {
        return id.to_string();
    }
    if let Some(url) = pr.url.as_deref() {
        if let Some(id) = extract_pr_id_from_url(url) {
            return id;
        }
    }
    "PR".to_string()
}

fn format_pull_request_summary(pull_requests: &[JiraPullRequest]) -> String {
    if pull_requests.is_empty() {
        return "-".to_string();
    }

    let mut ids: Vec<String> = pull_requests.iter().map(pull_request_display_id).collect();
    if ids.len() > 3 {
        let remaining = ids.len() - 3;
        ids.truncate(3);
        format!("{} +{}", ids.join(", "), remaining)
    } else {
        ids.join(", ")
    }
}

/// Returns color-coded status text based on the status name
fn get_colored_status(status: &str) -> String {
    match status.to_lowercase().as_str() {
        s if s.contains("done") => status.bright_green().bold().to_string(),
        s if s.contains("complete") => status.bright_green().bold().to_string(),
        s if s.contains("resolved") => status.bright_green().bold().to_string(),

        s if s.contains("progress") => status.bright_yellow().bold().to_string(),
        s if s.contains("review") => status.yellow().bold().to_string(),
        s if s.contains("implement") => status.bright_yellow().bold().to_string(),
        s if s.contains("testing") => status.bright_yellow().bold().to_string(),

        s if s.contains("todo") => status.bright_blue().to_string(),
        s if s.contains("backlog") => status.blue().to_string(),
        s if s.contains("selected") => status.cyan().to_string(),
        s if s.contains("open") => status.bright_blue().to_string(),

        s if s.contains("block") => status.bright_red().bold().to_string(),
        s if s.contains("impediment") => status.bright_red().bold().to_string(),
        s if s.contains("cancel") => status.red().bold().to_string(),
        s if s.contains("won't") => status.red().bold().to_string(),
        s if s.contains("wont") => status.red().bold().to_string(),

        _ => status.white().to_string(),
    }
}

/// Format a date string from JIRA's format to a more readable format
fn format_date(date_str: &str) -> String {
    if date_str.is_empty() {
        return "Not set".to_string();
    }

    // JIRA dates look like "2023-09-15T14:53:37.123+0000"
    // We'll just extract the date part for simplicity
    if let Some(date_part) = date_str.split('T').next() {
        return date_part.to_string();
    }

    date_str.to_string()
}

/// Extract plain text from Atlassian Document Format (ADF).
fn extract_plain_text_from_adf(adf: &Value) -> String {
    // If it's not an object with content, return empty string
    if !adf.is_object() || adf.get("content").is_none() {
        return String::new();
    }

    let mut result = String::new();

    // Try to process document content
    if let Some(content) = adf.get("content").and_then(|c| c.as_array()) {
        for item in content {
            // Process nested content
            process_content_node(item, &mut result);
            result.push('\n');
        }
    }

    result
}

/// Recursively process content nodes in Atlassian Document Format.
fn process_content_node(node: &Value, result: &mut String) {
    // Process text nodes
    if let Some(text) = node.get("text").and_then(|t| t.as_str()) {
        result.push_str(text);
    }

    // Recursively process nested content
    if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
        for item in content {
            process_content_node(item, result);

            // Add a newline if this is a paragraph or list item
            if let Some(node_type) = node.get("type").and_then(|t| t.as_str()) {
                if node_type == "paragraph" || node_type == "listItem" {
                    result.push('\n');
                }
            }
        }
    }
}

fn adf_value_to_display_text(value: &Value) -> String {
    let plain_text = extract_plain_text_from_adf(value);
    if plain_text.trim().is_empty() {
        serde_json::to_string_pretty(value)
            .unwrap_or_else(|_| "Cannot display content.".to_string())
    } else {
        plain_text.trim().to_string()
    }
}

fn text_to_adf(text: &str) -> Value {
    let content: Vec<Value> = text
        .split("\n\n")
        .filter_map(|paragraph| {
            let nodes: Vec<Value> = paragraph
                .lines()
                .enumerate()
                .flat_map(|(index, line)| {
                    let mut items = Vec::new();
                    if index > 0 {
                        items.push(json!({ "type": "hardBreak" }));
                    }
                    if !line.is_empty() {
                        items.push(json!({
                            "type": "text",
                            "text": line,
                        }));
                    }
                    items
                })
                .collect();

            if nodes.is_empty() {
                None
            } else {
                Some(json!({
                    "type": "paragraph",
                    "content": nodes,
                }))
            }
        })
        .collect();

    json!({
        "type": "doc",
        "version": 1,
        "content": content,
    })
}

fn comment_created_date(comment: &JiraComment) -> Option<&str> {
    comment
        .created
        .as_deref()
        .and_then(|timestamp| timestamp.split('T').next())
}

fn get_filtered_comments<'a>(
    issue: &'a JiraIssue,
    since: Option<&str>,
    comments_limit: usize,
    all_comments: bool,
) -> Vec<&'a JiraComment> {
    let mut comments: Vec<&JiraComment> = issue
        .fields
        .comment
        .as_ref()
        .map(|container| container.comments.iter().collect())
        .unwrap_or_default();

    if let Some(since_date) = since {
        comments.retain(|comment| {
            comment_created_date(comment)
                .map(|date| date >= since_date)
                .unwrap_or(false)
        });
    }

    if !all_comments && comments.len() > comments_limit {
        comments = comments.into_iter().rev().take(comments_limit).collect();
        comments.reverse();
    }

    comments
}

fn build_issue_json(
    issue: &JiraIssue,
    include_description: bool,
    include_comments: bool,
    include_prs: bool,
    pull_requests: &[JiraPullRequest],
    comments_limit: usize,
    all_comments: bool,
    since: Option<&str>,
) -> Value {
    let sprint_name = issue
        .fields
        .sprint
        .as_ref()
        .and_then(|sprints| sprints.iter().find(|s| s.state == "active"))
        .or_else(|| {
            issue
                .fields
                .sprint
                .as_ref()
                .and_then(|sprints| sprints.first())
        })
        .map(|sprint| sprint.name.clone());

    let mut payload = json!({
        "ticket": issue.key,
        "summary": issue.fields.summary,
        "status": issue.fields.status.as_ref().map(|s| s.name.clone()),
        "issue_type": issue.fields.issuetype.as_ref().map(|t| t.name.clone()),
        "priority": issue.fields.priority.as_ref().map(|p| p.name.clone()),
        "assignee": issue.fields.assignee.as_ref().map(|a| a.display_name.clone()),
        "reporter": issue.fields.reporter.as_ref().map(|r| r.display_name.clone()),
        "sprint": sprint_name,
        "created": issue.fields.created.clone(),
        "updated": issue.fields.updated.clone(),
        "due_date": issue.fields.due_date.clone()
    });

    if let Some(obj) = payload.as_object_mut() {
        if include_description {
            let description = issue
                .fields
                .description
                .as_ref()
                .map(|desc| {
                    if desc.is_null() {
                        Value::Null
                    } else {
                        Value::String(adf_value_to_display_text(desc))
                    }
                })
                .unwrap_or(Value::Null);
            obj.insert("description".to_string(), description);
        }

        if include_comments {
            let comments = get_filtered_comments(issue, since, comments_limit, all_comments);
            let comments_payload: Vec<Value> = comments
                .iter()
                .map(|comment| {
                    let body_value = comment
                        .body
                        .as_ref()
                        .map(|body| {
                            if body.is_null() {
                                Value::Null
                            } else {
                                Value::String(adf_value_to_display_text(body))
                            }
                        })
                        .unwrap_or(Value::Null);

                    json!({
                        "author": comment.author.as_ref().map(|a| a.display_name.clone()),
                        "created": comment.created.clone(),
                        "updated": comment.updated.clone(),
                        "body": body_value
                    })
                })
                .collect();

            obj.insert("comments".to_string(), Value::Array(comments_payload));
            obj.insert("comments_returned".to_string(), json!(comments.len()));
            if !all_comments {
                obj.insert("comments_limit".to_string(), json!(comments_limit));
            }
            if let Some(since_date) = since {
                obj.insert("comments_since".to_string(), json!(since_date));
            }
        }

        if include_prs {
            let pull_requests_payload: Vec<Value> = pull_requests
                .iter()
                .map(|pr| {
                    json!({
                        "id": pull_request_display_id(pr),
                        "title": pr.name,
                        "status": pr.status,
                        "url": pr.url,
                        "last_update": pr.last_update
                    })
                })
                .collect();

            obj.insert(
                "pull_requests".to_string(),
                Value::Array(pull_requests_payload),
            );
            obj.insert(
                "pull_requests_count".to_string(),
                json!(pull_requests.len()),
            );
        }
    }

    payload
}

/// Display detailed information about a JIRA ticket in a table format.
fn display_detailed_ticket(
    issue: &JiraIssue,
    include_description: bool,
    include_comments: bool,
    include_prs: bool,
    pull_requests: &[JiraPullRequest],
    comments_limit: usize,
    all_comments: bool,
    since: Option<&str>,
) -> Result<()> {
    println!("{}", "TICKET DETAILS".bold());
    println!();

    // Print the ticket key and summary as headers
    println!("{}: {}", issue.key.bold(), issue.fields.summary.bold());
    println!();

    // Type and Priority
    let issue_type = issue
        .fields
        .issuetype
        .as_ref()
        .map_or("Not set", |t| &t.name);
    let priority = issue
        .fields
        .priority
        .as_ref()
        .map_or("Not set", |p| &p.name);

    // Status and Sprint
    let status = issue.fields.status.as_ref().map_or("Not set", |s| &s.name);
    let sprint = issue
        .fields
        .sprint
        .as_ref()
        .and_then(|sprints| sprints.iter().find(|s| s.state == "active"))
        .map_or("Not in sprint", |s| &s.name);

    // Assignee and Reporter
    let assignee = issue
        .fields
        .assignee
        .as_ref()
        .map_or("Unassigned", |a| &a.display_name);
    let reporter = issue
        .fields
        .reporter
        .as_ref()
        .map_or("Unknown", |r| &r.display_name);

    // Created and Updated dates
    let created = issue.fields.created.as_ref().map_or("Unknown", |d| d);
    let updated = issue.fields.updated.as_ref().map_or("Unknown", |d| d);

    // Due Date
    let due_date = issue.fields.due_date.as_ref().map_or("Not set", |d| d);

    // Calculate width needed for label columns
    let left_col_width = 12; // "Due Date: " width
    let val_col_width = 18; // Width for value columns
    // Create a custom-drawn table with perfectly aligned columns
    println!(
        "{:<left$} {:<val$} {:<left$} {:<val$}",
        "Type:".bold(),
        issue_type,
        "Priority:".bold(),
        priority,
        left = left_col_width,
        val = val_col_width
    );

    println!(
        "{:<left$} {:<val$} {:<left$} {:<val$}",
        "Status:".bold(),
        get_colored_status(status),
        "Sprint:".bold(),
        sprint,
        left = left_col_width,
        val = val_col_width
    );

    println!(
        "{:<left$} {:<val$} {:<left$} {:<val$}",
        "Assignee:".bold(),
        assignee,
        "Reporter:".bold(),
        reporter,
        left = left_col_width,
        val = val_col_width
    );

    println!(
        "{:<left$} {:<val$} {:<left$} {:<val$}",
        "Created:".bold(),
        format_date(created),
        "Updated:".bold(),
        format_date(updated),
        left = left_col_width,
        val = val_col_width
    );

    println!(
        "{:<left$} {:<val$}",
        "Due Date:".bold(),
        format_date(due_date),
        left = left_col_width,
        val = val_col_width
    );

    if include_description {
        println!();
        println!("{}", "DESCRIPTION".bold());
        println!();

        // Print the description (if available)
        match &issue.fields.description {
            Some(desc) => {
                if desc.is_null() {
                    println!("No description provided.");
                } else {
                    println!("{}", adf_value_to_display_text(desc));
                }
            }
            None => println!("No description provided."),
        }
    }

    if include_comments {
        println!();
        println!("{}", "COMMENTS".bold());
        println!();

        let comments = get_filtered_comments(issue, since, comments_limit, all_comments);
        if comments.is_empty() {
            if since.is_some() {
                println!("No comments found for the provided filters.");
            } else {
                println!("No comments found.");
            }
        } else {
            for (index, comment) in comments.iter().enumerate() {
                let author = comment
                    .author
                    .as_ref()
                    .map_or("Unknown", |a| a.display_name.as_str());
                let created = comment.created.as_deref().unwrap_or("Unknown");
                let updated = comment.updated.as_deref().unwrap_or("Unknown");

                println!(
                    "#{} {} | created: {} | updated: {}",
                    index + 1,
                    author.bold(),
                    created,
                    updated
                );

                match &comment.body {
                    Some(body) if !body.is_null() => {
                        println!("{}", adf_value_to_display_text(body))
                    }
                    _ => println!("(No comment body)"),
                }

                if index < comments.len() - 1 {
                    println!();
                }
            }
        }
    }

    if include_prs {
        println!();
        println!("{}", "PULL REQUESTS".bold());
        println!();

        if pull_requests.is_empty() {
            println!("No pull requests found.");
        } else {
            for (index, pr) in pull_requests.iter().enumerate() {
                let pr_id = pull_request_display_id(pr);
                let title = pr.name.as_deref().unwrap_or("Untitled PR");
                let status = pr.status.as_deref().unwrap_or("Unknown");
                let last_update = pr.last_update.as_deref().unwrap_or("Unknown");
                let url = pr.url.as_deref().unwrap_or("No URL");

                println!(
                    "#{} {} [{}] | updated: {}",
                    index + 1,
                    pr_id.bold(),
                    status,
                    last_update
                );
                println!("{}", title);
                println!("{}", url);

                if index < pull_requests.len() - 1 {
                    println!();
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn cli_parses_create_command() {
        let cli = Cli::try_parse_from([
            "jit",
            "create",
            "--project",
            "RW",
            "--summary",
            "Implement backlog creation",
            "--description",
            "First line\nSecond line",
            "--type",
            "Story",
            "--assignee",
            "account-id-123",
            "--current-sprint",
            "--board",
            "456",
            "--json",
        ])
        .expect("create command should parse");

        match cli.command {
            Some(Commands::Create(args)) => {
                assert_eq!(args.project, "RW");
                assert_eq!(args.summary, "Implement backlog creation");
                assert_eq!(args.description.as_deref(), Some("First line\nSecond line"));
                assert_eq!(args.issue_type, "Story");
                assert_eq!(args.assignee, "account-id-123");
                assert!(args.current_sprint);
                assert_eq!(args.board, Some(456));
                assert!(args.json);
            }
            _ => panic!("expected create command"),
        }
    }

    #[test]
    fn cli_parses_edit_command() {
        let cli = Cli::try_parse_from([
            "jit",
            "edit",
            "RW-123",
            "--summary",
            "Updated summary",
            "--description",
            "First line\nSecond line",
            "--type",
            "Bug",
            "--assignee",
            "unassigned",
            "--json",
        ])
        .expect("edit command should parse");

        match cli.command {
            Some(Commands::Edit(args)) => {
                assert_eq!(args.ticket, "RW-123");
                assert_eq!(args.summary.as_deref(), Some("Updated summary"));
                assert_eq!(args.description.as_deref(), Some("First line\nSecond line"));
                assert_eq!(args.issue_type.as_deref(), Some("Bug"));
                assert_eq!(args.assignee.as_deref(), Some("unassigned"));
                assert!(args.json);
            }
            _ => panic!("expected edit command"),
        }
    }

    #[test]
    fn build_issue_create_payload_uses_adf_for_description() {
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: Some("First line\nSecond line\n\nNew paragraph".to_string()),
            issue_type: "Task".to_string(),
            assignee: "me".to_string(),
            current_sprint: false,
            board: None,
            json: false,
        };

        let payload = build_issue_create_payload(&args, Some("account-id-123"));

        assert_eq!(payload["fields"]["project"]["key"], "RW");
        assert_eq!(payload["fields"]["summary"], "Implement backlog creation");
        assert_eq!(payload["fields"]["issuetype"]["name"], "Task");
        assert_eq!(payload["fields"]["assignee"]["id"], "account-id-123");
        assert_eq!(payload["fields"]["description"]["type"], "doc");
        assert_eq!(payload["fields"]["description"]["version"], 1);
        assert_eq!(
            payload["fields"]["description"]["content"][0]["type"],
            "paragraph"
        );
        assert_eq!(
            payload["fields"]["description"]["content"][0]["content"][0]["text"],
            "First line"
        );
        assert_eq!(
            payload["fields"]["description"]["content"][0]["content"][1]["type"],
            "hardBreak"
        );
        assert_eq!(
            payload["fields"]["description"]["content"][0]["content"][2]["text"],
            "Second line"
        );
        assert_eq!(
            payload["fields"]["description"]["content"][1]["content"][0]["text"],
            "New paragraph"
        );
    }

    #[test]
    fn build_issue_update_payload_supports_core_editable_fields() {
        let args = EditArgs {
            ticket: "RW-123".to_string(),
            summary: Some("Updated summary".to_string()),
            description: Some("First line\nSecond line".to_string()),
            issue_type: Some("Bug".to_string()),
            assignee: Some("account-id-123".to_string()),
            json: false,
        };

        let payload = build_issue_update_payload(&args, Some("account-id-123"));

        assert_eq!(payload["fields"]["summary"], "Updated summary");
        assert_eq!(payload["fields"]["issuetype"]["name"], "Bug");
        assert_eq!(payload["fields"]["assignee"]["accountId"], "account-id-123");
        assert_eq!(payload["fields"]["description"]["type"], "doc");
        assert_eq!(payload["fields"]["description"]["version"], 1);
        assert_eq!(
            payload["fields"]["description"]["content"][0]["content"][0]["text"],
            "First line"
        );
        assert_eq!(
            payload["fields"]["description"]["content"][0]["content"][1]["type"],
            "hardBreak"
        );
        assert_eq!(
            payload["fields"]["description"]["content"][0]["content"][2]["text"],
            "Second line"
        );
    }

    #[test]
    fn build_issue_update_payload_clears_description_and_assignee() {
        let args = EditArgs {
            ticket: "RW-123".to_string(),
            summary: None,
            description: Some(String::new()),
            issue_type: None,
            assignee: Some("unassigned".to_string()),
            json: false,
        };

        let payload = build_issue_update_payload(&args, None);

        assert_eq!(payload["fields"]["description"], Value::Null);
        assert_eq!(payload["fields"]["assignee"], Value::Null);
    }

    #[test]
    fn create_jira_issue_posts_expected_request_for_explicit_assignee() {
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: Some("Description text".to_string()),
            issue_type: "Task".to_string(),
            assignee: "account-id-123".to_string(),
            current_sprint: false,
            board: None,
            json: false,
        };
        let expected_payload = build_issue_create_payload(&args, Some("account-id-123"));
        let (base_url, requests, handle) =
            spawn_test_server("HTTP/1.1 201 Created", r#"{"id":"10001","key":"RW-123"}"#);
        let client = create_jira_client("user@example.com", "token").expect("client");

        let created_issue = create_jira_issue(&client, &base_url, &args, Some("account-id-123"))
            .expect("issue creation should succeed");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        assert_eq!(created_issue.id, "10001");
        assert_eq!(created_issue.key, "RW-123");
        assert!(request.starts_with("POST /rest/api/3/issue HTTP/1.1"));
        assert!(
            request.contains("\r\ncontent-type: application/json\r\n")
                || request.contains("\r\nContent-Type: application/json\r\n")
        );

        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("http request should contain a body");
        let parsed_body: Value =
            serde_json::from_str(body).expect("request body should be valid json");
        assert_eq!(parsed_body, expected_payload);
    }

    #[test]
    fn update_jira_issue_puts_expected_request() {
        let args = EditArgs {
            ticket: "RW-123".to_string(),
            summary: Some("Updated summary".to_string()),
            description: Some("Description text".to_string()),
            issue_type: Some("Story".to_string()),
            assignee: Some("account-id-123".to_string()),
            json: false,
        };
        let expected_payload = build_issue_update_payload(&args, Some("account-id-123"));
        let (base_url, requests, handle) = spawn_test_server("HTTP/1.1 204 No Content", "");
        let client = create_jira_client("user@example.com", "token").expect("client");

        update_jira_issue(&client, &base_url, "RW-123", &args, Some("account-id-123"))
            .expect("issue update should succeed");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        assert!(request.starts_with("PUT /rest/api/3/issue/RW-123 HTTP/1.1"));
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("http request should contain a body");
        let parsed_body: Value =
            serde_json::from_str(body).expect("request body should be valid json");
        assert_eq!(parsed_body, expected_payload);
    }

    #[test]
    fn resolve_create_assignee_uses_current_user_by_default() {
        let (base_url, requests, handle) = spawn_test_server(
            "HTTP/1.1 200 OK",
            r#"{"accountId":"account-id-999","displayName":"Cesar Ferreira"}"#,
        );
        let client = create_jira_client("user@example.com", "token").expect("client");

        let assignee = resolve_create_assignee(&client, &base_url, "me")
            .expect("default assignee should resolve");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        assert_eq!(assignee.account_id.as_deref(), Some("account-id-999"));
        assert_eq!(assignee.label, "Cesar Ferreira");
        assert!(request.starts_with("GET /rest/api/3/myself HTTP/1.1"));
    }

    #[test]
    fn resolve_create_assignee_accepts_unassigned_without_network_call() {
        let assignee = resolve_create_assignee(
            &create_jira_client("user@example.com", "token").expect("client"),
            "http://127.0.0.1:9",
            "unassigned",
        )
        .expect("unassigned should not require network");

        assert_eq!(assignee.account_id, None);
        assert_eq!(assignee.label, "Unassigned");
    }

    #[test]
    fn resolve_target_sprint_uses_latest_active_sprint_across_project_boards() {
        let (base_url, requests, handle) = spawn_sequence_server(vec![
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":10,"name":"Alpha board"},{"id":20,"name":"Beta board"}],"isLast":true,"maxResults":50,"startAt":0}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":100,"name":"Alpha Sprint","startDate":"2026-03-01T09:00:00.000+00:00"}],"isLast":true,"maxResults":50,"startAt":0}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":200,"name":"Beta Sprint","startDate":"2026-03-05T09:00:00.000+00:00"}],"isLast":true,"maxResults":50,"startAt":0}"#,
            ),
        ]);
        let client = create_jira_client("user@example.com", "token").expect("client");
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: None,
            issue_type: "Task".to_string(),
            assignee: "me".to_string(),
            current_sprint: true,
            board: None,
            json: false,
        };

        let sprint = resolve_target_sprint(&client, &base_url, &args)
            .expect("current sprint should resolve")
            .expect("current sprint should be present");
        let requests = collect_requests(requests, 3);
        handle.join().expect("server thread should finish");

        assert_eq!(sprint.id, 200);
        assert_eq!(sprint.name, "Beta Sprint");
        assert_eq!(sprint.board_id, 20);
        assert_eq!(sprint.board_name, "Beta board");
        assert!(requests[0].starts_with("GET /rest/agile/1.0/board?projectKeyOrId=RW&type=scrum"));
        assert!(requests[1].starts_with("GET /rest/agile/1.0/board/10/sprint?state=active"));
        assert!(requests[2].starts_with("GET /rest/agile/1.0/board/20/sprint?state=active"));
    }

    #[test]
    fn add_issue_to_sprint_posts_issue_key() {
        let (base_url, requests, handle) = spawn_test_server("HTTP/1.1 204 No Content", "");
        let client = create_jira_client("user@example.com", "token").expect("client");

        add_issue_to_sprint(&client, &base_url, 200, "RW-123")
            .expect("adding issue to sprint should succeed");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        assert!(request.starts_with("POST /rest/agile/1.0/sprint/200/issue HTTP/1.1"));
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("http request should contain a body");
        let parsed_body: Value =
            serde_json::from_str(body).expect("request body should be valid json");
        assert_eq!(parsed_body, json!({ "issues": ["RW-123"] }));
    }

    fn spawn_test_server(
        status_line: &str,
        response_body: &'static str,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("read local addr");
        let (tx, rx) = mpsc::channel();
        let status_line = status_line.to_string();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set timeout");

            let request = read_http_request(&mut stream);
            tx.send(request).expect("send request");

            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });

        (format!("http://{}", addr), rx, handle)
    }

    fn spawn_sequence_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("read local addr");
        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            for (status_line, response_body) in responses {
                let (mut stream, _) = listener.accept().expect("accept connection");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("set timeout");

                let request = read_http_request(&mut stream);
                tx.send(request).expect("send request");

                let response = format!(
                    "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        (format!("http://{}", addr), rx, handle)
    }

    fn collect_requests(receiver: mpsc::Receiver<String>, expected: usize) -> Vec<String> {
        (0..expected)
            .map(|_| {
                receiver
                    .recv_timeout(Duration::from_secs(2))
                    .expect("request should be captured")
            })
            .collect()
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut temp = [0_u8; 4096];
        let mut header_end = None;
        let mut content_length = 0_usize;

        loop {
            let read = stream.read(&mut temp).expect("read request");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&temp[..read]);

            if header_end.is_none() {
                header_end = buffer
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|index| index + 4);

                if let Some(end) = header_end {
                    let headers = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                    content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .map(str::trim)
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                }
            }

            if let Some(end) = header_end {
                if buffer.len() >= end + content_length {
                    break;
                }
            }
        }

        String::from_utf8(buffer).expect("request should be utf8")
    }
}
