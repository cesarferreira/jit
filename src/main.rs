// src/main.rs
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;
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
    #[clap(required_unless_present = "my_tickets")]
    ticket: Option<String>,
    
    /// Output in JSON format
    #[clap(long)]
    json: bool,
    
    /// Output as plain text in format "KEY: Summary"
    #[clap(long)]
    text: bool,
    
    /// Display your current tickets in a table
    #[clap(long)]
    my_tickets: bool,
    
    /// Show detailed information about a ticket in a table format
    #[clap(long)]
    show: bool,
    
    /// Maximum number of tickets to retrieve (default: 10)
    #[clap(long, default_value = "10")]
    limit: u32,
    
    /// Path to a custom .env file
    #[clap(long)]
    env_file: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct JiraIssue {
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
    displayName: String,
}

#[derive(Debug, Deserialize, Default)]
struct JiraPriority {
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct JiraIssueType {
    name: String,
}

#[derive(Debug, Deserialize)]
struct JiraSearchResponse {
    issues: Vec<JiraIssue>,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    
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
    
    if args.my_tickets {
        // Fetch and display current tickets
        let tickets = fetch_my_tickets(&client, &jira_base_url, args.limit)?;
        display_tickets_table(&tickets)?;
    } else if let Some(ticket_input) = args.ticket {
        // Extract ticket ID from URL if needed
        let ticket_id = extract_ticket_id(&ticket_input)?;
        
        // Fetch issue details
        let issue = fetch_jira_issue(&client, &jira_base_url, &ticket_id)?;
        
        // Output the result
        if args.json {
            println!("{}", json!({
                "ticket": issue.key,
                "summary": issue.fields.summary
            }));
        } else if args.text {
            println!("{}: {}", issue.key, issue.fields.summary);
        } else if args.show {
            display_detailed_ticket(&issue)?;
        } else {
            println!("Ticket:   {}", issue.key);
            println!("Summary:  {}", issue.fields.summary);
        }
    } else {
        return Err(anyhow!("Either provide a ticket ID or use --my-tickets"));
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

fn fetch_jira_issue(client: &Client, base_url: &str, issue_key: &str) -> Result<JiraIssue> {
    let url = format!("{}/rest/api/3/issue/{}?fields=summary,status,customfield_10020,description,assignee,reporter,priority,issuetype,created,updated,duedate", base_url, issue_key);
    
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
    let url = format!("{}/rest/api/3/search", base_url);
    
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

fn display_tickets_table(tickets: &[JiraIssue]) -> Result<()> {
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
    let mut table = vec![
        vec!["Key".to_string(), "Summary".to_string(), "Status".to_string()]
    ];
    
    // Add the data rows
    for ticket in tickets {
        let status_text = ticket.fields.status.as_ref().map_or("Unknown", |s| &s.name);
        let summary = truncate_with_ellipsis(&ticket.fields.summary, 58);
        let colored_status = get_colored_status(status_text);
        
        table.push(vec![
            ticket.key.clone(),
            summary,
            colored_status
        ]);
    }
    
    // Calculate column widths
    let mut col_widths = vec![20, 7, 6]; // Set Key column to fixed 20 width
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

/// Extract plain text from a JIRA description in the Atlassian Document Format (ADF)
fn extract_plain_text_from_description(desc: &Value) -> String {
    // If it's not an object with content, return empty string
    if !desc.is_object() || !desc.get("content").is_some() {
        return String::new();
    }

    let mut result = String::new();
    
    // Try to process document content
    if let Some(content) = desc.get("content").and_then(|c| c.as_array()) {
        for item in content {
            // Process nested content
            process_content_node(item, &mut result);
            result.push('\n');
        }
    }
    
    result
}

/// Recursively process content nodes in Atlassian Document Format
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

/// Display detailed information about a JIRA ticket in a table format
fn display_detailed_ticket(issue: &JiraIssue) -> Result<()> {
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
    let assignee = issue.fields.assignee.as_ref().map_or("Unassigned", |a| &a.displayName);
    let reporter = issue.fields.reporter.as_ref().map_or("Unknown", |r| &r.displayName);
    
    // Created and Updated dates
    let created = issue.fields.created.as_ref().map_or("Unknown", |d| d);
    let updated = issue.fields.updated.as_ref().map_or("Unknown", |d| d);
    
    // Due Date
    let due_date = issue.fields.due_date.as_ref().map_or("Not set", |d| d);
    
    // Calculate width needed for label columns
    let left_col_width = 12; // "Due Date: " width
    let val_col_width = 18;  // Width for value columns
    let spacer_width = 2;    // Space between columns
    
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
    
    println!();
    println!("{}", "DESCRIPTION".bold());
    println!();
    
    // Print the description (if available)
    match &issue.fields.description {
        Some(desc) => {
            if desc.is_null() {
                println!("No description provided.");
            } else {
                // First try to extract plain text
                let plain_text = extract_plain_text_from_description(desc);
                if !plain_text.is_empty() {
                    println!("{}", plain_text);
                } else {
                    // Fall back to JSON if text extraction fails
                    println!("{}", serde_json::to_string_pretty(desc).unwrap_or_else(|_| "Cannot display description.".to_string()));
                }
            }
        },
        None => println!("No description provided.")
    }
    
    Ok(())
}
