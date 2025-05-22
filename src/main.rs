// src/main.rs
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;
use std::env;
use dotenv::dotenv;
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[derive(Parser)]
#[clap(author, version, about)]
struct Cli {
    /// JIRA issue key (e.g., RW-1931)
    ticket: String,
    
    /// Output in JSON format
    #[clap(long)]
    json: bool,
}

#[derive(Debug, Deserialize)]
struct JiraIssue {
    key: String,
    fields: JiraIssueFields,
}

#[derive(Debug, Deserialize)]
struct JiraIssueFields {
    summary: String,
}

fn main() -> Result<()> {
    // Load environment variables from .env file
    dotenv().ok();
    
    let args = Cli::parse();
    
    // Get Jira API credentials from environment
    let jira_base_url = env::var("JIRA_BASE_URL")
        .context("JIRA_BASE_URL not set in .env file")?;
    let jira_api_token = env::var("JIRA_API_TOKEN")
        .context("JIRA_API_TOKEN not set in .env file")?;
    let jira_user_email = env::var("JIRA_USER_EMAIL")
        .context("JIRA_USER_EMAIL not set in .env file")?;
    
    // Create HTTP client for JIRA API
    let client = create_jira_client(&jira_user_email, &jira_api_token)?;
    
    // Fetch issue details
    let issue = fetch_jira_issue(&client, &jira_base_url, &args.ticket)?;
    
    // Output the result
    if args.json {
        println!("{}", json!({
            "ticket": issue.key,
            "summary": issue.fields.summary
        }));
    } else {
        println!("Ticket:   {}", issue.key);
        println!("Summary:  {}", issue.fields.summary);
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

fn fetch_jira_issue(client: &Client, base_url: &str, issue_key: &str) -> Result<JiraIssue> {
    let url = format!("{}/rest/api/3/issue/{}", base_url, issue_key);
    
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
