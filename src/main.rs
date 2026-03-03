// src/main.rs
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use dotenv::dotenv;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use regex::Regex;
use colored::*;

#[derive(Parser)]
#[clap(author, version, about)]
struct Cli {
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

fn main() -> Result<()> {
    let args = Cli::parse();

    if let Some(since) = args.since.as_deref() {
        validate_since_date(since)?;
    }
    
    // Load environment variables from multiple locations
    load_environment_variables(&args);
    
    // Get Jira API credentials from environment
    let jira_base_url = env::var("JIRA_BASE_URL")
        .context("JIRA_BASE_URL not set. Set it in a .env file or as an environment variable")?;
    let jira_api_token = env::var("JIRA_API_TOKEN")
        .context("JIRA_API_TOKEN not set. Set it in a .env file or as an environment variable")?;
    let jira_user_email = env::var("JIRA_USER_EMAIL")
        .context("JIRA_USER_EMAIL not set. Set it in a .env file or as an environment variable")?;
    
    // Create HTTP client for JIRA API
    let client = create_jira_client(&jira_user_email, &jira_api_token)?;
    
    if args.my_tickets || args.ticket.is_none() {
        // Fetch and display current tickets
        let tickets = fetch_my_tickets(&client, &jira_base_url, args.limit)?;
        let include_prs = args.include_prs || args.full;
        let pull_requests_by_key = if include_prs {
            Some(fetch_pull_requests_for_tickets(&client, &jira_base_url, &tickets)?)
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
            &client,
            &jira_base_url,
            &ticket_id,
            include_details,
            include_description,
            include_comments,
        )?;

        let pull_requests = if include_prs {
            fetch_issue_pull_requests(&client, &jira_base_url, &issue.id)?
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
                println!("{}", json!({
                    "ticket": issue.key,
                    "summary": issue.fields.summary
                }));
            }
        } else if args.text {
            println!("{}: {}", issue.key, issue.fields.summary);
        } else if args.show || args.full || args.include_description || args.include_comments || args.include_prs {
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
fn load_environment_variables(args: &Cli) {
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
    if env::var("JIRA_BASE_URL").is_err() || 
       env::var("JIRA_API_TOKEN").is_err() || 
       env::var("JIRA_USER_EMAIL").is_err() {
        
        if let Some(home_dir) = dirs::home_dir() {
            let config_dir = home_dir.join(".config").join("jit");
            let home_env_path = config_dir.join(".env");
            
            if home_env_path.exists() {
                dotenv::from_path(&home_env_path).ok();
            } else {
                // If no config file exists yet, create the directory for future use
                if !config_dir.exists() {
                    if let Ok(_) = std::fs::create_dir_all(&config_dir) {
                        eprintln!("No configuration found. Created directory at: {:?}", config_dir);
                        eprintln!("Please create a .env file in this directory with your JIRA credentials:");
                        eprintln!("  JIRA_BASE_URL=https://your-company.atlassian.net");
                        eprintln!("  JIRA_API_TOKEN=your_api_token_here");
                        eprintln!("  JIRA_USER_EMAIL=your_email@example.com");
                    }
                }
            }
        }
    }
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
        return Err(anyhow!("Invalid --since value '{}'. Use YYYY-MM-DD.", since));
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
    let client = ClientBuilder::new()
        .default_headers(headers)
        .build()?;
    
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
    
    let response = client.get(&url)
        .send()
        .context("Failed to send request to JIRA API")?;
    
    if !response.status().is_success() {
        return Err(anyhow!(
            "JIRA API request failed with status: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }
    
    let issue: JiraIssue = response.json()
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
    
    let response = client.post(&url)
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
    
    let search_result: JiraSearchResponse = response.json()
        .context("Failed to parse JIRA API response")?;
    
    Ok(search_result.issues)
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
    let sprint_name = tickets[0].fields.sprint.as_ref()
        .and_then(|sprints| sprints.iter()
            .find(|s| s.state == "active")
            .or_else(|| sprints.first()))
        .map_or("Unknown Sprint", |s| &s.name);
    
    println!("Current Sprint: {}", sprint_name);
    println!();
    
    // Create a simple table with basic formatting
    let include_prs_column = pull_requests_by_key.is_some();
    let mut header_row = vec!["Key".to_string(), "Summary".to_string(), "Status".to_string()];
    if include_prs_column {
        header_row.push("PRs".to_string());
    }
    let mut table = vec![header_row];
    
    // Add the data rows
    for ticket in tickets {
        let status_text = ticket.fields.status.as_ref().map_or("Unknown", |s| &s.name);
        let summary = truncate_with_ellipsis(&ticket.fields.summary, 58);
        let colored_status = get_colored_status(status_text);
        
        let mut row = vec![
            ticket.key.clone(),
            summary,
            colored_status
        ];
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
                let status_text = tickets[table.iter().position(|r| &r[0] == &row[0]).unwrap_or(0) - 1]
                    .fields.status.as_ref().map_or("Unknown", |s| &s.name);
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
                let status_text = tickets[row_idx - 1].fields.status.as_ref().map_or("Unknown", |s| &s.name);
                format!(" {}{}", cell, " ".repeat(col_widths[col_idx] - status_text.len() - 1))
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
    let number = suffix
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim();
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
        .or_else(|| issue.fields.sprint.as_ref().and_then(|sprints| sprints.first()))
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
    let issue_type = issue.fields.issuetype.as_ref().map_or("Not set", |t| &t.name);
    let priority = issue.fields.priority.as_ref().map_or("Not set", |p| &p.name);
    
    // Status and Sprint
    let status = issue.fields.status.as_ref().map_or("Not set", |s| &s.name);
    let sprint = issue.fields.sprint.as_ref()
        .and_then(|sprints| sprints.iter().find(|s| s.state == "active"))
        .map_or("Not in sprint", |s| &s.name);
    
    // Assignee and Reporter
    let assignee = issue.fields.assignee.as_ref().map_or("Unassigned", |a| &a.display_name);
    let reporter = issue.fields.reporter.as_ref().map_or("Unknown", |r| &r.display_name);
    
    // Created and Updated dates
    let created = issue.fields.created.as_ref().map_or("Unknown", |d| d);
    let updated = issue.fields.updated.as_ref().map_or("Unknown", |d| d);
    
    // Due Date
    let due_date = issue.fields.due_date.as_ref().map_or("Not set", |d| d);
    
    // Calculate width needed for label columns
    let left_col_width = 12; // "Due Date: " width
    let val_col_width = 18;  // Width for value columns
    // Create a custom-drawn table with perfectly aligned columns
    println!("{:<left$} {:<val$} {:<left$} {:<val$}", 
             "Type:".bold(), issue_type,
             "Priority:".bold(), priority,
             left = left_col_width, val = val_col_width);
             
    println!("{:<left$} {:<val$} {:<left$} {:<val$}", 
             "Status:".bold(), get_colored_status(status),
             "Sprint:".bold(), sprint,
             left = left_col_width, val = val_col_width);
             
    println!("{:<left$} {:<val$} {:<left$} {:<val$}", 
             "Assignee:".bold(), assignee,
             "Reporter:".bold(), reporter,
             left = left_col_width, val = val_col_width);
             
    println!("{:<left$} {:<val$} {:<left$} {:<val$}", 
             "Created:".bold(), format_date(created),
             "Updated:".bold(), format_date(updated),
             left = left_col_width, val = val_col_width);
             
    println!("{:<left$} {:<val$}", 
             "Due Date:".bold(), format_date(due_date),
             left = left_col_width, val = val_col_width);
    
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
                    Some(body) if !body.is_null() => println!("{}", adf_value_to_display_text(body)),
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
