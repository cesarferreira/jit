// src/main.rs
use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, FixedOffset};
use clap::{Args, Parser, Subcommand};
use colored::*;
use regex::Regex;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use unicode_width::UnicodeWidthStr;

mod api;
mod discovery;
mod workflow;

const DEFAULT_TICKET_LIMIT: u32 = 50;

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
    /// Configure Jira credentials interactively
    Auth,
    /// Search Jira issues with JQL
    Search(discovery::SearchArgs),
    /// List accessible Jira boards
    Boards(discovery::BoardsArgs),
    /// List sprints on a board
    Sprints(discovery::SprintsArgs),
    /// List available workflow transitions for an issue
    Transitions(workflow::TransitionsArgs),
    /// Move an issue through its workflow
    Transition(workflow::TransitionArgs),
    /// Add a comment to an issue
    Comment(workflow::CommentArgs),
    /// Link related issues
    Link(workflow::LinkArgs),
    /// Create a Jira issue assigned to you in the current sprint by default
    Create(CreateArgs),
    /// Edit an existing Jira ticket's fields or move it into the current sprint
    Edit(EditArgs),
    /// Manage the shared jit agent skill (SKILL.md) installed for coding agents like Codex and Claude Code
    Skill(SkillArgs),
}

#[derive(Args, Debug)]
struct SkillArgs {
    #[command(subcommand)]
    command: SkillCommands,
}

#[derive(Subcommand, Debug)]
enum SkillCommands {
    /// Install the bundled skill for agents with existing config folders, or selected agents
    Install(SkillInstallArgs),
}

#[derive(Args, Debug)]
struct SkillInstallArgs {
    /// Agents: codex, claude, pi, cursor, antigravity. Comma-separated or repeat --agent; defaults to detected config folders
    #[clap(long, value_delimiter = ',')]
    agent: Vec<String>,
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

    /// Maximum number of tickets to retrieve
    #[clap(long, default_value_t = DEFAULT_TICKET_LIMIT, value_parser = clap::value_parser!(u32).range(1..))]
    limit: u32,

    /// Path to a custom config.toml file
    #[clap(long)]
    config_file: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
struct CreateArgs {
    /// Jira project key (e.g., RW)
    #[clap(long)]
    project: String,

    /// Ticket summary
    #[clap(long)]
    summary: String,

    /// Plain-text ticket description
    #[clap(long, conflicts_with = "description_file")]
    description: Option<String>,

    /// Read the plain-text description from a UTF-8 file, or '-' for stdin
    #[clap(long, conflicts_with = "description")]
    description_file: Option<PathBuf>,

    /// Jira issue type name, such as Task, Bug, or Story (default: Task)
    #[clap(long = "type", default_value = "Task")]
    issue_type: String,

    /// Assignee account ID, or `me` to assign to the current Jira user (default: me)
    #[clap(long, default_value = "me")]
    assignee: String,

    /// Add the created ticket to the current active sprint (the default)
    #[clap(long, conflicts_with = "backlog")]
    current_sprint: bool,

    /// Leave the created ticket in the backlog without sprint assignment
    #[clap(long, conflicts_with_all = ["current_sprint", "board"])]
    backlog: bool,

    /// Jira board ID to use for current sprint selection; otherwise the most recently started active sprint is used
    #[clap(long, conflicts_with = "backlog")]
    board: Option<u64>,

    /// Output created issue details in JSON format
    #[clap(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct EditArgs {
    /// JIRA issue key (e.g., RW-1931) or URL (e.g., https://company.atlassian.net/browse/RW-1931)
    ticket: String,

    /// Updated ticket summary
    #[clap(long)]
    summary: Option<String>,

    /// Updated plain-text ticket description; pass an empty string to clear it
    #[clap(long, conflicts_with = "description_file")]
    description: Option<String>,

    /// Read the plain-text description from a UTF-8 file, or '-' for stdin
    #[clap(long, conflicts_with = "description")]
    description_file: Option<PathBuf>,

    /// Updated Jira issue type name, such as Task, Bug, or Story
    #[clap(long = "type")]
    issue_type: Option<String>,

    /// Updated assignee account ID, or `me` / `unassigned`
    #[clap(long)]
    assignee: Option<String>,

    /// Move the existing ticket into the current active sprint
    #[clap(long)]
    current_sprint: bool,

    /// Jira board ID to use for current sprint selection; otherwise the most recently started active sprint is used
    #[clap(long, requires = "current_sprint")]
    board: Option<u64>,

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
    #[serde(rename = "customfield_10396", default)]
    acceptance_criteria: Option<Value>,
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

#[derive(Debug, Deserialize)]
struct AppConfig {
    jira: JiraConfig,
}

#[derive(Debug, Deserialize)]
struct JiraConfig {
    base_url: String,
    api_token: String,
    user_email: String,
    #[serde(default = "default_sprint_field")]
    sprint_field: String,
    #[serde(default = "default_acceptance_criteria_field")]
    acceptance_criteria_field: String,
}

fn default_sprint_field() -> String {
    "customfield_10020".to_owned()
}
fn default_acceptance_criteria_field() -> String {
    "customfield_10396".to_owned()
}

#[derive(Debug)]
struct MutationContext {
    ticket: String,
    completed_operations: Vec<&'static str>,
    message: String,
}
impl std::fmt::Display for MutationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn error_json(error: &anyhow::Error, ticket: Option<&str>, code_override: Option<&str>) -> Value {
    let mutation = error.downcast_ref::<MutationContext>();
    let api_error = error.downcast_ref::<api::ApiError>();
    let completed = mutation
        .map(|context| context.completed_operations.as_slice())
        .unwrap_or(&[]);
    let code = code_override.unwrap_or_else(|| {
        if !completed.is_empty() {
            "partial_failure"
        } else {
            api_error.map_or("command_failed", api::ApiError::code)
        }
    });
    json!({"error": {
        "code": code,
        "message": format!("{error:#}"),
        "ticket": mutation.map(|context| context.ticket.as_str()).or(ticket),
        "completed_operations": completed,
        "status": api_error.map(|error| error.status),
    }})
}

fn main() {
    let mut args = match Cli::try_parse() {
        Ok(args) => args,
        Err(error) => {
            if error.use_stderr() && std::env::args().any(|arg| arg == "--json") {
                eprintln!(
                    "{}",
                    error_json(&anyhow!(error.to_string()), None, Some("invalid_arguments"))
                );
                std::process::exit(2);
            }
            error.exit();
        }
    };
    let query_json = args.query.json;
    let json_flag = match &mut args.command {
        Some(Commands::Create(a)) => &mut a.json,
        Some(Commands::Edit(a)) => &mut a.json,
        Some(Commands::Search(a)) => &mut a.json,
        Some(Commands::Boards(a)) => &mut a.json,
        Some(Commands::Sprints(a)) => &mut a.json,
        Some(Commands::Transitions(a)) => &mut a.json,
        Some(Commands::Transition(a)) => &mut a.json,
        Some(Commands::Comment(a)) => &mut a.json,
        Some(Commands::Link(a)) => &mut a.json,
        _ => &mut args.query.json,
    };
    *json_flag |= query_json;
    let json_output = *json_flag;
    let ticket = match &args.command {
        Some(Commands::Edit(a)) => Some(a.ticket.as_str()),
        Some(Commands::Transitions(a)) => Some(a.ticket.as_str()),
        Some(Commands::Transition(a)) => Some(a.ticket.as_str()),
        Some(Commands::Comment(a)) => Some(a.ticket.as_str()),
        Some(Commands::Link(a)) => Some(a.ticket.as_str()),
        _ => args.query.ticket.as_deref(),
    }
    .and_then(|ticket| extract_ticket_id(ticket).ok());
    if let Err(error) = run(args) {
        if json_output {
            eprintln!("{}", error_json(&error, ticket.as_deref(), None));
        } else {
            eprintln!("Error: {error:#}");
        }
        std::process::exit(1);
    }
}

fn run(args: Cli) -> Result<()> {
    if let Some(since) = args.query.since.as_deref() {
        validate_since_date(since)?;
    }

    if matches!(args.command, Some(Commands::Auth)) {
        return run_auth_command(args.query.config_file.as_deref());
    }

    if let Some(Commands::Skill(skill_args)) = args.command.as_ref() {
        let SkillCommands::Install(install_args) = &skill_args.command;
        return run_skill_install_command(install_args);
    }

    let config = load_configuration(&args.query)?;

    let client = create_jira_client(&config.user_email, &config.api_token)?;

    match args.command {
        Some(Commands::Auth) => unreachable!("auth command is handled before loading config"),
        Some(Commands::Skill(_)) => unreachable!("skill command is handled before loading config"),
        Some(Commands::Create(create_args)) => {
            run_create_issue_command(&client, &config.base_url, &create_args)
        }
        Some(Commands::Edit(edit_args)) => {
            run_edit_issue_command(&client, &config.base_url, &edit_args)
        }
        Some(Commands::Search(a)) => {
            discovery::run_search(&client, &config.base_url, &a, &config.sprint_field)
        }
        Some(Commands::Boards(a)) => discovery::run_boards(&client, &config.base_url, &a),
        Some(Commands::Sprints(a)) => discovery::run_sprints(&client, &config.base_url, &a),
        Some(Commands::Transitions(a)) => workflow::run_transitions(&client, &config.base_url, &a),
        Some(Commands::Transition(a)) => workflow::run_transition(&client, &config.base_url, &a),
        Some(Commands::Comment(a)) => workflow::run_comment(&client, &config.base_url, &a),
        Some(Commands::Link(a)) => workflow::run_link(&client, &config.base_url, &a),
        None => run_query_mode(&client, &config, args.query),
    }
}

fn run_query_mode(client: &Client, config: &JiraConfig, args: QueryArgs) -> Result<()> {
    let jira_base_url = config.base_url.as_str();
    if args.my_tickets || args.ticket.is_none() {
        // Fetch and display current tickets
        let (tickets, truncated) =
            fetch_my_tickets(client, jira_base_url, args.limit, &config.sprint_field)?;
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
        if args.json {
            let issues: Vec<_> = tickets
                .iter()
                .map(|issue| {
                    let prs = pull_requests_by_key
                        .as_ref()
                        .and_then(|by_key| by_key.get(&issue.key))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    build_issue_json(issue, false, false, include_prs, prs, 0, false, None)
                })
                .collect();
            println!("{}", json!({"issues":issues,"truncated":truncated}));
            return Ok(());
        }
        display_tickets_table(&tickets, pull_requests_by_key.as_ref())?;
        if truncated {
            eprintln!(
                "Results truncated at {} issues; increase --limit to see more.",
                args.limit
            );
        }
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
            (&config.sprint_field, &config.acceptance_criteria_field),
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

/// Attempts to load configuration from multiple locations in order:
/// 1. Custom config file passed as an argument
/// 2. Current directory config.toml
/// 3. User config directory ~/.config/jit/config.toml
fn load_configuration(args: &QueryArgs) -> Result<JiraConfig> {
    let config_path = resolve_config_path(args)?;
    read_config_file(&config_path)
}

fn resolve_config_path(args: &QueryArgs) -> Result<PathBuf> {
    if let Some(config_path) = &args.config_file {
        if config_path.exists() {
            return Ok(config_path.clone());
        }

        return Err(anyhow!(
            "Specified config.toml file not found at: {}. Run `jit auth --config-file {}` to create it, or create the file manually.",
            config_path.display(),
            config_path.display()
        ));
    }

    let local_config = PathBuf::from("config.toml");
    if local_config.exists() {
        return Ok(local_config);
    }

    if let Some(user_config) = default_config_path() {
        if user_config.exists() {
            return Ok(user_config);
        }

        return Err(anyhow!(
            "No configuration found. Run `jit auth` to configure Jira credentials, or create `config.toml` in the current directory or at `{}` with:\n[jira]\nbase_url = \"https://your-company.atlassian.net\"\napi_token = \"your_api_token_here\"\nuser_email = \"your_email@example.com\"",
            user_config.display()
        ));
    }

    Err(anyhow!(
        "No configuration found. Run `jit auth` to configure Jira credentials, or create a `config.toml` file with:\n[jira]\nbase_url = \"https://your-company.atlassian.net\"\napi_token = \"your_api_token_here\"\nuser_email = \"your_email@example.com\""
    ))
}

fn default_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|path| config_path_from_home(&path))
}

fn config_path_from_home(home_dir: &Path) -> PathBuf {
    home_dir.join(".config").join("jit").join("config.toml")
}

fn read_config_file(path: &Path) -> Result<JiraConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file at {}", path.display()))?;
    let config: AppConfig = toml::from_str(&contents).with_context(|| {
        format!(
            "Failed to parse config file at {}. Expected:\n[jira]\nbase_url = \"https://your-company.atlassian.net\"\napi_token = \"your_api_token_here\"\nuser_email = \"your_email@example.com\"",
            path.display()
        )
    })?;
    Ok(config.jira)
}

fn run_auth_command(config_file: Option<&Path>) -> Result<()> {
    let config_path = match config_file {
        Some(path) => path.to_path_buf(),
        None => default_config_path().context("Could not determine default config path")?,
    };

    let mut existing_config = None;
    if config_path.exists() {
        match read_config_file(&config_path) {
            Ok(config) => {
                println!("Existing Jira config found at {}", config_path.display());
                println!("Base URL: {}", config.base_url);
                println!("Email:    {}", config.user_email);
                println!("Token:    {}", mask_token(&config.api_token));
                existing_config = Some(config);
            }
            Err(err) => {
                println!(
                    "Existing config at {} could not be read:",
                    config_path.display()
                );
                println!("{}", err);
            }
        }

        if !prompt_yes_no("Overwrite this config? [y/N]: ")? {
            println!("Leaving existing config unchanged.");
            return Ok(());
        }
    }

    let base_url_input = if let Some(config) = existing_config.as_ref() {
        prompt_with_default("Jira company URL", &config.base_url)?
    } else {
        prompt_required("Jira company URL: ")?
    };
    let base_url = normalize_jira_base_url(&base_url_input)?;
    let user_email = if let Some(config) = existing_config.as_ref() {
        prompt_with_default("Jira account email", &config.user_email)?
    } else {
        prompt_required("Jira account email: ")?
    };

    let token_url = "https://id.atlassian.com/manage-profile/security/api-tokens";
    println!("Open this URL to create an Atlassian API token:");
    println!("{}", token_url);
    if let Err(err) = open_url(token_url) {
        println!("Could not open your browser automatically: {}", err);
    }

    let api_token = prompt_required("Paste API token: ")?;
    let client = create_jira_client(&user_email, &api_token)?;
    validate_jira_authentication(&client, &base_url)?;

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create config directory at {}", parent.display())
        })?;
    }

    fs::write(
        &config_path,
        format_config_toml(&JiraConfig {
            base_url,
            api_token,
            user_email,
            sprint_field: existing_config
                .as_ref()
                .map(|config| config.sprint_field.clone())
                .unwrap_or_else(default_sprint_field),
            acceptance_criteria_field: existing_config
                .as_ref()
                .map(|config| config.acceptance_criteria_field.clone())
                .unwrap_or_else(default_acceptance_criteria_field),
        }),
    )
    .with_context(|| format!("Failed to write config file at {}", config_path.display()))?;

    println!("Saved Jira credentials to {}", config_path.display());
    Ok(())
}

const BUNDLED_SKILL: &str = include_str!("../SKILL.md");

struct AgentTarget {
    name: &'static str,
    config_subpaths: &'static [&'static [&'static str]],
}

const AGENT_TARGETS: &[AgentTarget] = &[
    AgentTarget {
        name: "codex",
        config_subpaths: &[&[".codex"]],
    },
    AgentTarget {
        name: "claude",
        config_subpaths: &[&[".claude"]],
    },
    AgentTarget {
        name: "pi",
        config_subpaths: &[&[".pi", "agent"]],
    },
    AgentTarget {
        name: "cursor",
        config_subpaths: &[&[".cursor"]],
    },
    AgentTarget {
        name: "antigravity",
        config_subpaths: &[&[".gemini", "config"], &[".gemini", "antigravity"]],
    },
];

fn existing_agent_config(home: &Path, target: &AgentTarget) -> Option<PathBuf> {
    target
        .config_subpaths
        .iter()
        .map(|segments| home.join(segments.iter().collect::<PathBuf>()))
        .find(|path| path.is_dir())
}

fn run_skill_install_command(args: &SkillInstallArgs) -> Result<()> {
    let home = dirs::home_dir().context("Could not determine home directory")?;

    let mut targets = Vec::new();
    if args.agent.is_empty() {
        targets.extend(
            AGENT_TARGETS
                .iter()
                .filter(|target| existing_agent_config(&home, target).is_some()),
        );
    } else {
        for agent in &args.agent {
            let agent = agent.trim().to_ascii_lowercase();
            let target = AGENT_TARGETS
                .iter()
                .find(|target| target.name == agent)
                .ok_or_else(|| {
                    anyhow!(
                        "Unknown agent '{}'. Supported: {}",
                        agent,
                        AGENT_TARGETS
                            .iter()
                            .map(|target| target.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            if !targets
                .iter()
                .any(|selected: &&AgentTarget| selected.name == target.name)
            {
                targets.push(target);
            }
        }
    }

    if targets.is_empty() {
        println!(
            "No supported agent config folders found. Use --agent <name> to install explicitly."
        );
    }
    for target in targets {
        let config = existing_agent_config(&home, target)
            .unwrap_or_else(|| home.join(target.config_subpaths[0].iter().collect::<PathBuf>()));
        let path = config.join("skills").join("jit").join("SKILL.md");

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create skill directory at {}", parent.display())
            })?;
        }

        fs::write(&path, BUNDLED_SKILL)
            .with_context(|| format!("Failed to write SKILL.md to {}", path.display()))?;

        println!("Installed {} skill at {}", target.name, path.display());
    }

    Ok(())
}

fn prompt_required(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;

    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(anyhow!(
            "{} cannot be empty",
            prompt.trim().trim_end_matches(':')
        ));
    }
    Ok(value)
}

fn prompt_with_default(prompt: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", prompt, default);
    io::stdout().flush()?;

    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();
    if value.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value.to_string())
    }
}

fn prompt_yes_no(prompt: &str) -> Result<bool> {
    print!("{}", prompt);
    io::stdout().flush()?;

    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn normalize_jira_base_url(input: &str) -> Result<String> {
    let base_url = input.trim().trim_end_matches('/').to_string();
    if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        return Err(anyhow!(
            "Jira company URL must start with https:// or http://"
        ));
    }
    Ok(base_url)
}

fn open_url(url: &str) -> Result<()> {
    if std::env::var_os("JIT_AUTH_SKIP_OPEN").is_some() {
        return Ok(());
    }

    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    }
    .context("failed to launch browser command")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("browser command exited with {}", status))
    }
}

fn format_config_toml(config: &JiraConfig) -> String {
    format!(
        "[jira]\nbase_url = {}\napi_token = {}\nuser_email = {}\nsprint_field = {}\nacceptance_criteria_field = {}\n",
        toml_string(&config.base_url),
        toml_string(&config.api_token),
        toml_string(&config.user_email),
        toml_string(&config.sprint_field),
        toml_string(&config.acceptance_criteria_field)
    )
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization should not fail")
}

fn mask_token(token: &str) -> String {
    if token.is_empty() {
        return "<empty>".to_string();
    }

    "*".repeat(token.chars().count().min(8))
}

fn read_text_input(inline: Option<&str>, file: Option<&Path>) -> Result<Option<String>> {
    if inline.is_some() && file.is_some() {
        return Err(anyhow!("Pass inline text or a file, not both"));
    }
    if let Some(path) = file {
        if path == Path::new("-") {
            let mut input = String::new();
            io::stdin()
                .read_to_string(&mut input)
                .context("Failed to read UTF-8 text from stdin")?;
            return Ok(Some(input));
        }
        return fs::read_to_string(path)
            .with_context(|| format!("Failed to read UTF-8 text from {}", path.display()))
            .map(Some);
    }
    Ok(inline.map(str::to_owned))
}

fn run_create_issue_command(client: &Client, jira_base_url: &str, args: &CreateArgs) -> Result<()> {
    let mut resolved_args = args.clone();
    resolved_args.description = read_text_input(
        args.description.as_deref(),
        args.description_file.as_deref(),
    )?;
    let args = &resolved_args;
    let resolved_assignee = resolve_create_assignee(client, jira_base_url, &args.assignee)?;
    let resolved_sprint = resolve_target_sprint(client, jira_base_url, args).context(
        "Could not select the current sprint. Pass --board <id> to select a board or --backlog to create without sprint assignment.",
    )?;
    let created_issue = create_jira_issue(
        client,
        jira_base_url,
        args,
        resolved_assignee.account_id.as_deref(),
    )?;
    if let Some(sprint) = resolved_sprint.as_ref() {
        add_issue_to_sprint(client, jira_base_url, sprint.id, &created_issue.key).with_context(
            || {
                MutationContext {
                    ticket: created_issue.key.clone(),
                    completed_operations: vec!["created"],
                    message: format!("Created {} but failed to add it to sprint {}. Retry with jit edit {} --current-sprint --board {}.", created_issue.key, sprint.name, created_issue.key, sprint.board_id),
                }
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
    let mut resolved_args = args.clone();
    resolved_args.description = read_text_input(
        args.description.as_deref(),
        args.description_file.as_deref(),
    )?;
    let args = &resolved_args;
    let has_core_edits = args.summary.is_some()
        || args.description.is_some()
        || args.issue_type.is_some()
        || args.assignee.is_some();
    if !has_core_edits && !args.current_sprint {
        return Err(anyhow!(
            "No editable fields provided. Pass at least one of --summary, --description, --type, --assignee, or --current-sprint."
        ));
    }

    let ticket_id = extract_ticket_id(&args.ticket)?;
    let resolved_sprint = resolve_current_sprint(
        client,
        jira_base_url,
        ticket_id
            .split_once('-')
            .map_or(ticket_id.as_str(), |(project, _)| project),
        args.current_sprint,
        args.board,
    )?;
    let resolved_assignee = args
        .assignee
        .as_deref()
        .map(|assignee| resolve_create_assignee(client, jira_base_url, assignee))
        .transpose()?;
    if has_core_edits {
        update_jira_issue(
            client,
            jira_base_url,
            &ticket_id,
            args,
            resolved_assignee
                .as_ref()
                .and_then(|assignee| assignee.account_id.as_deref()),
        )?;
    }
    if let Some(sprint) = resolved_sprint.as_ref() {
        add_issue_to_sprint(client, jira_base_url, sprint.id, &ticket_id).with_context(|| {
            let prefix = if has_core_edits {
                "Updated core fields, but "
            } else {
                ""
            };
            MutationContext {
                ticket: ticket_id.clone(),
                completed_operations: if has_core_edits { vec!["core_fields_updated"] } else { vec![] },
                message: format!("{}failed to add {} to sprint {}. Retry with jit edit {} --current-sprint --board {}.", prefix, ticket_id, sprint.name, ticket_id, sprint.board_id),
            }
        })?;
    }

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

    if resolved_sprint.is_some() {
        updated_fields.push("sprint");
    }

    if args.json {
        let mut payload = json!({
            "ticket": ticket_id,
            "updated_fields": updated_fields,
            "url": issue_url,
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(sprint) = resolved_sprint.as_ref() {
                obj.insert("sprint".to_string(), json!(sprint.name));
                obj.insert("sprint_id".to_string(), json!(sprint.id));
                obj.insert("board".to_string(), json!(sprint.board_name));
                obj.insert("board_id".to_string(), json!(sprint.board_id));
            }
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
        if let Some(sprint) = resolved_sprint.as_ref() {
            println!("Board:    {}", sprint.board_name);
            println!("Sprint:   {}", sprint.name);
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
    field_ids: (&str, &str),
) -> Result<JiraIssue> {
    let (sprint_field, acceptance_criteria_field) = field_ids;
    let mut fields = vec!["summary"];

    if include_details {
        fields.extend([
            "status",
            sprint_field,
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
        fields.push(acceptance_criteria_field);
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

    let response = api::checked_response(response)?;

    let mut value: Value = response
        .json()
        .context("Failed to parse JIRA API response")?;
    if include_comments {
        let comment = &value["fields"]["comment"];
        let count = comment["comments"].as_array().map_or(0, Vec::len);
        if comment["total"]
            .as_u64()
            .is_some_and(|total| total > count as u64)
            || comment["startAt"].as_u64().is_some_and(|start| start > 0)
        {
            value["fields"]["comment"]["comments"] =
                json!(fetch_all_comments(client, base_url, issue_key)?);
        }
    }
    decode_issue(value, sprint_field, acceptance_criteria_field)
}

fn fetch_all_comments(client: &Client, base_url: &str, issue_key: &str) -> Result<Vec<Value>> {
    let mut comments = Vec::new();
    let mut start = 0_u64;
    loop {
        let page = api::json_response(
            client
                .get(format!("{base_url}/rest/api/3/issue/{issue_key}/comment"))
                .query(&[("startAt", start), ("maxResults", 100)])
                .send()?,
        )?;
        let values = page["comments"]
            .as_array()
            .context("Jira comment response is missing comments")?;
        let total = page["total"]
            .as_u64()
            .context("Jira comment response is missing total")?;
        if page["startAt"]
            .as_u64()
            .is_some_and(|offset| offset != start)
        {
            return Err(anyhow!("Jira comment pagination did not advance"));
        }
        let next = start + values.len() as u64;
        comments.extend(values.iter().cloned());
        if next >= total {
            break;
        }
        if next == start {
            return Err(anyhow!("Jira comment pagination did not advance"));
        }
        start = next;
    }
    Ok(comments)
}

fn decode_issue(
    mut value: Value,
    sprint_field: &str,
    acceptance_criteria_field: &str,
) -> Result<JiraIssue> {
    if let Some(fields) = value.get_mut("fields").and_then(Value::as_object_mut) {
        for (configured, canonical) in [
            (sprint_field, "customfield_10020"),
            (acceptance_criteria_field, "customfield_10396"),
        ] {
            if configured != canonical {
                fields.remove(canonical);
                if let Some(value) = fields.remove(configured) {
                    fields.insert(canonical.to_owned(), value);
                }
            }
        }
    }
    serde_json::from_value(value).context("Failed to parse JIRA issue response")
}

fn fetch_my_tickets(
    client: &Client,
    base_url: &str,
    limit: u32,
    sprint_field: &str,
) -> Result<(Vec<JiraIssue>, bool)> {
    let (values, truncated) = discovery::search_issues(
        client,
        base_url,
        "assignee = currentUser() AND sprint in openSprints() ORDER BY updated DESC",
        &["summary", "status", "assignee", sprint_field],
        limit,
    )?;
    if values.is_empty() {
        validate_jira_authentication(client, base_url)?;
    }
    let issues = values
        .into_iter()
        .map(|value| decode_issue(value, sprint_field, "customfield_10396"))
        .collect::<Result<Vec<_>>>()?;
    Ok((issues, truncated))
}

fn validate_jira_authentication(client: &Client, base_url: &str) -> Result<()> {
    let url = format!("{}/rest/api/3/myself", base_url);
    let response = client
        .get(&url)
        .send()
        .context("Failed to send request to JIRA API")?;

    let status = response.status();
    api::empty_response(response).with_context(|| {
        if status.as_u16() == 401 || status.as_u16() == 403 {
            format!("Jira authentication failed with status: {status}. Check your Jira email and API token.")
        } else {
            format!("Jira authentication check failed with status: {status}")
        }
    })
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

    let response = api::checked_response(response)?;

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

    api::empty_response(response)
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

    let response = api::checked_response(response)?;

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
    resolve_current_sprint(client, base_url, &args.project, !args.backlog, args.board)
}

fn resolve_current_sprint(
    client: &Client,
    base_url: &str,
    project: &str,
    current_sprint: bool,
    board: Option<u64>,
) -> Result<Option<ResolvedSprint>> {
    if !current_sprint {
        return Ok(None);
    }

    let sprint = if let Some(board_id) = board {
        resolve_active_sprint_for_board(client, base_url, board_id)?
    } else {
        resolve_latest_active_sprint_for_project(client, base_url, project)?
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
            "No active sprint found on accessible Scrum boards for project {}. Start a sprint in Jira or pass --board <id> to select a different board.",
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
            "No active sprint found on board {} ({}). Start a sprint in Jira or pass --board <id> to select a different board.",
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

        let response = api::checked_response(response)?;

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

    let response = api::checked_response(response)?;

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

        let response = api::checked_response(response)?;

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

    api::empty_response(response)
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

    let response = api::checked_response(response).context("Jira dev-status request failed")?;

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
        let mut row = vec![ticket.key.clone(), summary, status_text.to_string()];
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
            let cell_width = cell.width();

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
            let cell_text = if row_idx > 0 && col_idx == 2 {
                get_colored_status(cell)
            } else {
                cell.clone()
            };
            let padding = col_widths[col_idx] - cell.width() - 1;
            print!(" {}{}", cell_text, " ".repeat(padding));
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

// Truncate to terminal display columns, including the ellipsis.
fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    if s.width() <= max_len {
        return s.to_string();
    }
    if max_len <= 3 {
        return ".".repeat(max_len);
    }

    let mut end = 0;
    for (index, ch) in s.char_indices() {
        let next = index + ch.len_utf8();
        if s[..next].width() > max_len - 3 {
            break;
        }
        end = next;
    }
    format!("{}...", &s[..end])
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

/// Render Atlassian Document Format (ADF) into readable text while preserving links.
fn extract_plain_text_from_adf(adf: &Value) -> String {
    let mut result = String::new();
    render_adf_node(adf, &mut result);
    result
}

fn render_adf_node(node: &Value, result: &mut String) {
    let Some(node_type) = node.get("type").and_then(|t| t.as_str()) else {
        if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
            for child in content {
                render_adf_node(child, result);
            }
        }
        return;
    };

    match node_type {
        "doc" => render_adf_blocks(node, result),
        "paragraph" | "heading" | "blockquote" | "codeBlock" => {
            let block = render_adf_inline_content(node);
            append_block(result, &block);
        }
        "bulletList" => render_adf_list(node, result, false),
        "orderedList" => render_adf_list(node, result, true),
        "listItem" => result.push_str(render_adf_list_item(node).trim_end()),
        "rule" => append_block(result, "---"),
        "hardBreak" => result.push('\n'),
        "inlineCard" | "blockCard" | "embedCard" => {
            if let Some(url) = node
                .get("attrs")
                .and_then(|attrs| attrs.get("url"))
                .and_then(|url| url.as_str())
            {
                result.push_str(url);
            }
        }
        "mention" => {
            if let Some(text) = node
                .get("attrs")
                .and_then(|attrs| attrs.get("text"))
                .and_then(|text| text.as_str())
            {
                result.push_str(text);
            }
        }
        "emoji" => {
            if let Some(text) = node
                .get("attrs")
                .and_then(|attrs| attrs.get("text"))
                .and_then(|text| text.as_str())
                .or_else(|| {
                    node.get("attrs")
                        .and_then(|attrs| attrs.get("shortName"))
                        .and_then(|text| text.as_str())
                })
            {
                result.push_str(text);
            }
        }
        "status" => {
            if let Some(text) = node
                .get("attrs")
                .and_then(|attrs| attrs.get("text"))
                .and_then(|text| text.as_str())
            {
                result.push_str(text);
            }
        }
        "date" => {
            if let Some(timestamp) = node
                .get("attrs")
                .and_then(|attrs| attrs.get("timestamp"))
                .and_then(|timestamp| timestamp.as_str())
            {
                result.push_str(timestamp);
            }
        }
        "text" => {
            let text = node
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or_default();
            result.push_str(text);

            if let Some(href) = node
                .get("marks")
                .and_then(|marks| marks.as_array())
                .and_then(|marks| extract_link_href(marks))
            {
                let trimmed_text = text.trim();
                if !href.is_empty() && trimmed_text != href && !trimmed_text.contains(href) {
                    result.push_str(" (");
                    result.push_str(href);
                    result.push(')');
                }
            }
        }
        _ => {
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    render_adf_node(child, result);
                }
            }
        }
    }
}

fn render_adf_blocks(node: &Value, result: &mut String) {
    if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
        for child in content {
            render_adf_node(child, result);
        }
    }
}

fn render_adf_inline_content(node: &Value) -> String {
    let mut block = String::new();
    if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
        for child in content {
            render_adf_node(child, &mut block);
        }
    }
    block
}

fn render_adf_list(node: &Value, result: &mut String, ordered: bool) {
    if let Some(items) = node.get("content").and_then(|c| c.as_array()) {
        for (index, item) in items.iter().enumerate() {
            let rendered_item = render_adf_list_item(item);
            if rendered_item.is_empty() {
                continue;
            }

            let prefix = if ordered {
                format!("{}. ", index + 1)
            } else {
                "- ".to_string()
            };

            let mut lines = rendered_item.lines();
            if let Some(first_line) = lines.next() {
                append_block(result, &format!("{prefix}{first_line}"));
            }

            for line in lines {
                append_block(result, line);
            }
        }
    }
}

fn render_adf_list_item(node: &Value) -> String {
    let mut item = String::new();
    if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
        for child in content {
            match child.get("type").and_then(|t| t.as_str()) {
                Some("paragraph") | Some("heading") | Some("blockquote") | Some("codeBlock") => {
                    let block = render_adf_inline_content(child);
                    if !block.trim().is_empty() {
                        if !item.is_empty() && !item.ends_with('\n') {
                            item.push('\n');
                        }
                        item.push_str(block.trim_end());
                    }
                }
                Some("bulletList") | Some("orderedList") => {
                    let mut nested = String::new();
                    render_adf_node(child, &mut nested);
                    if !nested.trim().is_empty() {
                        if !item.is_empty() && !item.ends_with('\n') {
                            item.push('\n');
                        }
                        item.push_str(nested.trim_end());
                    }
                }
                _ => render_adf_node(child, &mut item),
            }
        }
    }
    item.trim().to_string()
}

fn append_block(result: &mut String, block: &str) {
    let block = block.trim_end();
    if block.is_empty() {
        return;
    }

    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result.push_str(block);
    result.push('\n');
}

fn extract_link_href<'a>(marks: &'a [Value]) -> Option<&'a str> {
    marks.iter().find_map(|mark| {
        (mark.get("type").and_then(|t| t.as_str()) == Some("link"))
            .then(|| {
                mark.get("attrs")
                    .and_then(|attrs| attrs.get("href"))
                    .and_then(|href| href.as_str())
            })
            .flatten()
    })
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

            let acceptance_criteria = issue
                .fields
                .acceptance_criteria
                .as_ref()
                .map(|criteria| {
                    if criteria.is_null() {
                        Value::Null
                    } else {
                        Value::String(adf_value_to_display_text(criteria))
                    }
                })
                .unwrap_or(Value::Null);
            obj.insert("acceptance_criteria".to_string(), acceptance_criteria);
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

        if let Some(criteria) = &issue.fields.acceptance_criteria {
            if !criteria.is_null() {
                println!();
                println!("{}", "ACCEPTANCE CRITERIA".bold());
                println!();
                println!("{}", adf_value_to_display_text(criteria));
            }
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
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    fn cli_parses_skill_install_command() {
        let cli = Cli::try_parse_from(["jit", "skill", "install"])
            .expect("skill install command should parse");

        match cli.command {
            Some(Commands::Skill(args)) => match args.command {
                SkillCommands::Install(install_args) => {
                    assert!(install_args.agent.is_empty());
                }
            },
            _ => panic!("expected skill install command"),
        }
    }

    #[test]
    fn cli_parses_skill_install_with_agent_filter() {
        let cli = Cli::try_parse_from(["jit", "skill", "install", "--agent", "codex"])
            .expect("skill install --agent should parse");

        match cli.command {
            Some(Commands::Skill(args)) => match args.command {
                SkillCommands::Install(install_args) => {
                    assert_eq!(install_args.agent, ["codex"]);
                }
            },
            _ => panic!("expected skill install command"),
        }
    }

    #[test]
    fn cli_parses_config_file_flag() {
        let cli = Cli::try_parse_from(["jit", "--config-file", "/tmp/jit-config.toml", "RW-123"])
            .expect("config file flag should parse");

        assert_eq!(
            cli.query.config_file,
            Some(PathBuf::from("/tmp/jit-config.toml"))
        );
        assert_eq!(cli.query.ticket.as_deref(), Some("RW-123"));
    }

    #[test]
    fn cli_defaults_my_tickets_limit_to_full_sprint_view() {
        let cli =
            Cli::try_parse_from(["jit", "--my-tickets"]).expect("my tickets flag should parse");

        assert_eq!(cli.query.limit, 50);
    }

    #[test]
    fn read_config_file_loads_jira_credentials_from_toml() {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jit-config-{unique_id}.toml"));

        fs::write(
            &path,
            r#"[jira]
base_url = "https://example.atlassian.net"
api_token = "token-123"
user_email = "user@example.com"
"#,
        )
        .expect("temp config should be written");

        let config = read_config_file(&path).expect("config should parse");
        fs::remove_file(&path).expect("temp config should be removed");

        assert_eq!(config.base_url, "https://example.atlassian.net");
        assert_eq!(config.api_token, "token-123");
        assert_eq!(config.user_email, "user@example.com");
    }

    #[test]
    fn config_path_from_home_uses_dot_config_jit_directory() {
        let path = config_path_from_home(Path::new("/Users/example"));

        assert_eq!(
            path,
            PathBuf::from("/Users/example/.config/jit/config.toml")
        );
    }

    #[test]
    fn extract_ticket_id_accepts_plain_ticket_keys() {
        let ticket = extract_ticket_id("RW-123").expect("plain ticket keys should be accepted");

        assert_eq!(ticket, "RW-123");
    }

    #[test]
    fn extract_ticket_id_extracts_ticket_from_browse_url() {
        let ticket = extract_ticket_id("https://example.atlassian.net/browse/RW-123/")
            .expect("browse URLs should parse");

        assert_eq!(ticket, "RW-123");
    }

    #[test]
    fn extract_ticket_id_rejects_invalid_jira_urls() {
        let error = extract_ticket_id("https://example.atlassian.net/issues/RW-123")
            .expect_err("non-browse URLs should be rejected");

        assert!(
            error
                .to_string()
                .contains("Could not extract ticket ID from URL")
        );
    }

    #[test]
    fn validate_since_date_accepts_yyyy_mm_dd_dates() {
        validate_since_date("2026-04-22").expect("valid dates should pass");
    }

    #[test]
    fn validate_since_date_rejects_invalid_date_format() {
        let error =
            validate_since_date("22-04-2026").expect_err("invalid date formats should fail");

        assert!(error.to_string().contains("Use YYYY-MM-DD"));
    }

    #[test]
    fn resolve_config_path_prefers_explicit_config_file() {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jit-config-path-{unique_id}.toml"));
        fs::write(&path, "[jira]\nbase_url = \"https://example.atlassian.net\"\napi_token = \"token\"\nuser_email = \"user@example.com\"\n")
            .expect("temp config should be written");

        let args = QueryArgs {
            ticket: None,
            json: false,
            text: false,
            my_tickets: false,
            show: false,
            include_description: false,
            include_comments: false,
            include_prs: false,
            full: false,
            comments_limit: 5,
            all_comments: false,
            since: None,
            limit: 10,
            config_file: Some(path.clone()),
        };

        let resolved = resolve_config_path(&args).expect("explicit config file should resolve");
        fs::remove_file(&path).expect("temp config should be removed");

        assert_eq!(resolved, path);
    }

    #[test]
    fn resolve_config_path_errors_for_missing_explicit_config_file() {
        let args = QueryArgs {
            ticket: None,
            json: false,
            text: false,
            my_tickets: false,
            show: false,
            include_description: false,
            include_comments: false,
            include_prs: false,
            full: false,
            comments_limit: 5,
            all_comments: false,
            since: None,
            limit: 10,
            config_file: Some(PathBuf::from("/tmp/definitely-missing-jit-config.toml")),
        };

        let error =
            resolve_config_path(&args).expect_err("missing explicit config should return an error");

        assert!(
            error
                .to_string()
                .contains("Specified config.toml file not found")
        );
    }

    #[test]
    fn build_issue_create_payload_uses_adf_for_description() {
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: Some("First line\nSecond line\n\nNew paragraph".to_string()),
            description_file: None,
            issue_type: "Task".to_string(),
            assignee: "me".to_string(),
            current_sprint: false,
            backlog: false,
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
            description_file: None,
            issue_type: Some("Bug".to_string()),
            assignee: Some("account-id-123".to_string()),
            current_sprint: false,
            board: None,
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
    fn build_issue_create_payload_skips_blank_description_and_unassigned_assignee() {
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: Some("   ".to_string()),
            description_file: None,
            issue_type: "Task".to_string(),
            assignee: "unassigned".to_string(),
            current_sprint: false,
            backlog: false,
            board: None,
            json: false,
        };

        let payload = build_issue_create_payload(&args, None);

        assert!(payload["fields"].get("description").is_none());
        assert!(payload["fields"].get("assignee").is_none());
    }

    #[test]
    fn build_issue_update_payload_clears_description_and_assignee() {
        let args = EditArgs {
            ticket: "RW-123".to_string(),
            summary: None,
            description: Some(String::new()),
            description_file: None,
            issue_type: None,
            assignee: Some("unassigned".to_string()),
            current_sprint: false,
            board: None,
            json: false,
        };

        let payload = build_issue_update_payload(&args, None);

        assert_eq!(payload["fields"]["description"], Value::Null);
        assert_eq!(payload["fields"]["assignee"], Value::Null);
    }

    #[test]
    fn adf_value_to_display_text_preserves_inline_links() {
        let value = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [
                    {
                        "type": "text",
                        "text": "See docs",
                        "marks": [{
                            "type": "link",
                            "attrs": { "href": "https://example.com/docs" }
                        }]
                    }
                ]
            }]
        });

        assert_eq!(
            adf_value_to_display_text(&value),
            "See docs (https://example.com/docs)"
        );
    }

    #[test]
    fn adf_value_to_display_text_preserves_smart_links() {
        let value = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Runbook: " },
                    {
                        "type": "inlineCard",
                        "attrs": { "url": "https://example.com/runbook" }
                    }
                ]
            }]
        });

        assert_eq!(
            adf_value_to_display_text(&value),
            "Runbook: https://example.com/runbook"
        );
    }

    #[test]
    fn create_jira_issue_posts_expected_request_for_explicit_assignee() {
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: Some("Description text".to_string()),
            description_file: None,
            issue_type: "Task".to_string(),
            assignee: "account-id-123".to_string(),
            current_sprint: false,
            backlog: false,
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
            description_file: None,
            issue_type: Some("Story".to_string()),
            assignee: Some("account-id-123".to_string()),
            current_sprint: false,
            board: None,
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
    fn resolve_create_assignee_keeps_explicit_account_id() {
        let assignee = resolve_create_assignee(
            &create_jira_client("user@example.com", "token").expect("client"),
            "http://127.0.0.1:9",
            "account-id-123",
        )
        .expect("explicit account ids should be returned as-is");

        assert_eq!(assignee.account_id.as_deref(), Some("account-id-123"));
        assert_eq!(assignee.label, "account-id-123");
    }

    #[test]
    fn fetch_current_user_assignee_uses_fallback_label_when_display_name_missing() {
        let (base_url, requests, handle) = spawn_test_server(
            "HTTP/1.1 200 OK",
            r#"{"accountId":"account-id-999","displayName":""}"#,
        );
        let client = create_jira_client("user@example.com", "token").expect("client");

        let assignee =
            fetch_current_user_assignee(&client, &base_url).expect("current user should resolve");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        assert_eq!(assignee.account_id.as_deref(), Some("account-id-999"));
        assert_eq!(assignee.label, "Current user");
        assert!(request.starts_with("GET /rest/api/3/myself HTTP/1.1"));
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
            description_file: None,
            issue_type: "Task".to_string(),
            assignee: "me".to_string(),
            current_sprint: true,
            backlog: false,
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
    fn resolve_target_sprint_returns_none_when_backlog_requested() {
        let client = create_jira_client("user@example.com", "token").expect("client");
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: None,
            description_file: None,
            issue_type: "Task".to_string(),
            assignee: "me".to_string(),
            current_sprint: false,
            backlog: true,
            board: None,
            json: false,
        };

        let sprint = resolve_target_sprint(&client, "http://127.0.0.1:9", &args)
            .expect("sprint lookup should be skipped");

        assert!(sprint.is_none());
    }

    #[test]
    fn resolve_target_sprint_uses_explicit_board_when_provided() {
        let (base_url, requests, handle) = spawn_sequence_server(vec![
            ("HTTP/1.1 200 OK", r#"{"id":42,"name":"Explicit board"}"#),
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":300,"name":"Board Sprint","startDate":"2026-04-01T09:00:00+00:00"}],"isLast":true,"maxResults":50,"startAt":0}"#,
            ),
        ]);
        let client = create_jira_client("user@example.com", "token").expect("client");
        let args = CreateArgs {
            project: "RW".to_string(),
            summary: "Implement backlog creation".to_string(),
            description: None,
            description_file: None,
            issue_type: "Task".to_string(),
            assignee: "me".to_string(),
            current_sprint: true,
            backlog: false,
            board: Some(42),
            json: false,
        };

        let sprint = resolve_target_sprint(&client, &base_url, &args)
            .expect("sprint should resolve")
            .expect("sprint should be present");
        let requests = collect_requests(requests, 2);
        handle.join().expect("server thread should finish");

        assert_eq!(sprint.id, 300);
        assert_eq!(sprint.board_id, 42);
        assert_eq!(sprint.board_name, "Explicit board");
        assert!(requests[0].starts_with("GET /rest/agile/1.0/board/42 HTTP/1.1"));
        assert!(requests[1].starts_with("GET /rest/agile/1.0/board/42/sprint?state=active"));
    }

    #[test]
    fn fetch_scrum_boards_for_project_reads_all_pages() {
        let (base_url, requests, handle) = spawn_sequence_server(vec![
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":10,"name":"Alpha board"}],"isLast":false,"maxResults":1,"startAt":0}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":20,"name":"Beta board"}],"isLast":true,"maxResults":1,"startAt":1}"#,
            ),
        ]);
        let client = create_jira_client("user@example.com", "token").expect("client");

        let boards = fetch_scrum_boards_for_project(&client, &base_url, "RW")
            .expect("board pagination should succeed");
        let requests = collect_requests(requests, 2);
        handle.join().expect("server thread should finish");

        assert_eq!(boards.len(), 2);
        assert_eq!(boards[0].id, 10);
        assert_eq!(boards[1].id, 20);
        assert!(requests[0].contains("startAt=0&maxResults=50"));
        assert!(requests[1].contains("startAt=1&maxResults=50"));
    }

    #[test]
    fn fetch_active_sprints_for_board_prefers_latest_active_sprint_across_pages() {
        let (base_url, requests, handle) = spawn_sequence_server(vec![
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":100,"name":"Alpha Sprint","startDate":"2026-03-01T09:00:00+00:00"}],"isLast":false,"maxResults":1,"startAt":0}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                r#"{"values":[{"id":200,"name":"Beta Sprint","startDate":"2026-03-05T09:00:00+00:00"}],"isLast":true,"maxResults":1,"startAt":1}"#,
            ),
        ]);
        let client = create_jira_client("user@example.com", "token").expect("client");

        let sprint = fetch_active_sprints_for_board(&client, &base_url, 42)
            .expect("sprint pagination should succeed")
            .expect("a sprint should be returned");
        let requests = collect_requests(requests, 2);
        handle.join().expect("server thread should finish");

        assert_eq!(sprint.id, 200);
        assert_eq!(sprint.name, "Beta Sprint");
        assert!(requests[0].contains("/board/42/sprint?state=active&startAt=0"));
        assert!(requests[1].contains("/board/42/sprint?state=active&startAt=1"));
    }

    #[test]
    fn compare_sprint_identity_prefers_newer_dates_then_ids() {
        assert!(compare_sprint_identity(
            Some("2026-04-10T09:00:00+00:00"),
            10,
            1,
            Some("2026-04-09T09:00:00+00:00"),
            99,
            2,
        ));
        assert!(compare_sprint_identity(
            Some("2026-04-10T09:00:00+00:00"),
            10,
            2,
            Some("2026-04-10T09:00:00+00:00"),
            10,
            1,
        ));
        assert!(!compare_sprint_identity(
            None,
            10,
            1,
            Some("2026-04-10T09:00:00+00:00"),
            5,
            1,
        ));
    }

    #[test]
    fn parse_jira_datetime_accepts_rfc3339_values() {
        let parsed = parse_jira_datetime("2026-04-10T09:00:00+00:00")
            .expect("valid RFC3339 values should parse");

        assert_eq!(parsed.to_rfc3339(), "2026-04-10T09:00:00+00:00");
    }

    #[test]
    fn fetch_jira_issue_requests_expected_detail_fields() {
        let (base_url, requests, handle) = spawn_test_server(
            "HTTP/1.1 200 OK",
            r#"{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation","status":{"name":"In Progress"},"customfield_10020":[{"name":"Sprint 42","state":"active"}],"description":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]}]},"comment":{"comments":[{"author":{"displayName":"Cesar Ferreira"},"body":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Comment body"}]}]},"created":"2026-04-10T09:00:00.000+00:00","updated":"2026-04-10T10:00:00.000+00:00"}]}}}"#,
        );
        let client = create_jira_client("user@example.com", "token").expect("client");

        let issue = fetch_jira_issue(
            &client,
            &base_url,
            "RW-123",
            true,
            true,
            true,
            ("customfield_10020", "customfield_10396"),
        )
        .expect("issue fetch should succeed");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        assert_eq!(issue.key, "RW-123");
        assert!(request.starts_with("GET /rest/api/3/issue/RW-123?fields="));
        assert!(request.contains("assignee,comment,created,customfield_10020,customfield_10396,description,duedate,issuetype,priority,reporter,status,summary,updated"));
    }

    #[test]
    fn fetch_my_tickets_posts_active_sprint_jql() {
        let (base_url, requests, handle) = spawn_test_server(
            "HTTP/1.1 200 OK",
            r#"{"issues":[{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation","status":{"name":"In Progress"},"customfield_10020":[{"name":"Sprint 42","state":"active"}]}}]}"#,
        );
        let client = create_jira_client("user@example.com", "token").expect("client");

        let (issues, _) = fetch_my_tickets(&client, &base_url, 7, "customfield_10020")
            .expect("ticket fetch should succeed");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("http request should contain a body");
        let parsed_body: Value =
            serde_json::from_str(body).expect("request body should be valid json");

        assert_eq!(issues.len(), 1);
        assert!(request.starts_with("POST /rest/api/3/search/jql HTTP/1.1"));
        assert_eq!(
            parsed_body["jql"],
            "assignee = currentUser() AND sprint in openSprints() ORDER BY updated DESC"
        );
        assert_eq!(parsed_body["maxResults"], 7);
    }

    #[test]
    fn fetch_issue_pull_requests_flattens_dev_status_response() {
        let (base_url, requests, handle) = spawn_test_server(
            "HTTP/1.1 200 OK",
            r#"{"detail":[{"pullRequests":[{"id":"1","name":"First PR","status":"OPEN","url":"https://github.com/org/repo/pull/1","lastUpdate":"2026-04-10T09:00:00.000+00:00"}]},{"pullRequests":[{"id":"2","name":"Second PR","status":"MERGED","url":"https://github.com/org/repo/pull/2","lastUpdate":"2026-04-10T10:00:00.000+00:00"}]}]}"#,
        );
        let client = create_jira_client("user@example.com", "token").expect("client");

        let pull_requests = fetch_issue_pull_requests(&client, &base_url, "10001")
            .expect("PR fetch should succeed");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request should be captured");
        handle.join().expect("server thread should finish");

        assert_eq!(pull_requests.len(), 2);
        assert_eq!(pull_requests[0].name.as_deref(), Some("First PR"));
        assert_eq!(pull_requests[1].name.as_deref(), Some("Second PR"));
        assert!(request.starts_with("GET /rest/dev-status/latest/issue/detail?issueId=10001"));
    }

    #[test]
    fn fetch_pull_requests_for_tickets_maps_results_by_ticket_key() {
        let (base_url, requests, handle) = spawn_sequence_server(vec![
            (
                "HTTP/1.1 200 OK",
                r#"{"detail":[{"pullRequests":[{"id":"1","name":"First PR","status":"OPEN","url":"https://github.com/org/repo/pull/1","lastUpdate":"2026-04-10T09:00:00.000+00:00"}]}]}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                r#"{"detail":[{"pullRequests":[{"id":"2","name":"Second PR","status":"MERGED","url":"https://github.com/org/repo/pull/2","lastUpdate":"2026-04-10T10:00:00.000+00:00"}]}]}"#,
            ),
        ]);
        let client = create_jira_client("user@example.com", "token").expect("client");
        let tickets = vec![
            sample_issue_with_summary("10001", "RW-123", "First issue"),
            sample_issue_with_summary("10002", "RW-124", "Second issue"),
        ];

        let mapped = fetch_pull_requests_for_tickets(&client, &base_url, &tickets)
            .expect("PR mapping should succeed");
        let requests = collect_requests(requests, 2);
        handle.join().expect("server thread should finish");

        assert_eq!(mapped["RW-123"].len(), 1);
        assert_eq!(mapped["RW-124"].len(), 1);
        assert!(requests[0].contains("issueId=10001"));
        assert!(requests[1].contains("issueId=10002"));
    }

    #[test]
    fn truncate_with_ellipsis_shortens_long_strings() {
        let truncated = truncate_with_ellipsis("abcdefghijklmnopqrstuvwxyz", 10);

        assert_eq!(truncated, "abcdefg...");
    }

    #[test]
    fn truncation_measures_terminal_columns_instead_of_bytes() {
        assert_eq!(truncate_with_ellipsis("éééé", 4), "éééé");
        assert_eq!(truncate_with_ellipsis("🌵ab", 4), "🌵ab");
        assert_eq!(truncate_with_ellipsis("界界界", 5), "界...");
        assert_eq!(truncate_with_ellipsis("abcdef", 2), "..");
        assert_eq!(truncate_with_ellipsis("abcdef", 0), "");
    }

    #[test]
    fn extract_pr_id_from_url_returns_pull_number() {
        let pr_id = extract_pr_id_from_url("https://github.com/org/repo/pull/123/files");

        assert_eq!(pr_id.as_deref(), Some("#123"));
    }

    #[test]
    fn pull_request_display_id_falls_back_to_pull_number_and_default_label() {
        let from_url = JiraPullRequest {
            id: None,
            name: Some("PR from URL".to_string()),
            status: None,
            url: Some("https://github.com/org/repo/pull/123".to_string()),
            last_update: None,
        };
        let fallback = JiraPullRequest {
            id: None,
            name: Some("No metadata".to_string()),
            status: None,
            url: None,
            last_update: None,
        };

        assert_eq!(pull_request_display_id(&from_url), "#123");
        assert_eq!(pull_request_display_id(&fallback), "PR");
    }

    #[test]
    fn format_pull_request_summary_limits_output_to_three_entries() {
        let prs = vec![
            JiraPullRequest {
                id: Some("#1".to_string()),
                name: None,
                status: None,
                url: None,
                last_update: None,
            },
            JiraPullRequest {
                id: Some("#2".to_string()),
                name: None,
                status: None,
                url: None,
                last_update: None,
            },
            JiraPullRequest {
                id: Some("#3".to_string()),
                name: None,
                status: None,
                url: None,
                last_update: None,
            },
            JiraPullRequest {
                id: Some("#4".to_string()),
                name: None,
                status: None,
                url: None,
                last_update: None,
            },
        ];

        assert_eq!(format_pull_request_summary(&prs), "#1, #2, #3 +1");
    }

    #[test]
    fn format_date_returns_iso_date_or_not_set() {
        assert_eq!(format_date("2026-04-10T09:00:00.000+00:00"), "2026-04-10");
        assert_eq!(format_date(""), "Not set");
    }

    #[test]
    fn text_to_adf_preserves_paragraphs_and_line_breaks() {
        let adf = text_to_adf("First line\nSecond line\n\nNew paragraph");

        assert_eq!(adf["content"][0]["content"][0]["text"], "First line");
        assert_eq!(adf["content"][0]["content"][1]["type"], "hardBreak");
        assert_eq!(adf["content"][0]["content"][2]["text"], "Second line");
        assert_eq!(adf["content"][1]["content"][0]["text"], "New paragraph");
    }

    #[test]
    fn get_filtered_comments_applies_since_and_limit_filters() {
        let issue = sample_issue_with_comments(vec![
            sample_comment("Ada", "2026-04-01T09:00:00.000+00:00", "Old"),
            sample_comment("Grace", "2026-04-10T09:00:00.000+00:00", "Keep me"),
            sample_comment("Linus", "2026-04-12T09:00:00.000+00:00", "Newest"),
        ]);

        let comments = get_filtered_comments(&issue, Some("2026-04-05"), 1, false);

        assert_eq!(comments.len(), 1);
        assert_eq!(
            comments[0]
                .author
                .as_ref()
                .map(|author| author.display_name.as_str()),
            Some("Linus")
        );
    }

    #[test]
    fn build_issue_json_includes_description_comments_and_prs() {
        let issue = JiraIssue {
            id: "10001".to_string(),
            key: "RW-123".to_string(),
            fields: JiraIssueFields {
                summary: "Implement backlog creation".to_string(),
                status: Some(JiraStatus {
                    name: "In Progress".to_string(),
                }),
                sprint: Some(vec![JiraSprint {
                    name: "Sprint 42".to_string(),
                    state: "active".to_string(),
                }]),
                description: Some(text_to_adf("Hello\nWorld")),
                acceptance_criteria: None,
                assignee: Some(JiraUser {
                    display_name: "Cesar Ferreira".to_string(),
                    account_id: Some("account-id-123".to_string()),
                }),
                reporter: Some(JiraUser {
                    display_name: "Ada Lovelace".to_string(),
                    account_id: Some("account-id-999".to_string()),
                }),
                priority: Some(JiraPriority {
                    name: "High".to_string(),
                }),
                issuetype: Some(JiraIssueType {
                    name: "Task".to_string(),
                }),
                created: Some("2026-04-10T09:00:00.000+00:00".to_string()),
                updated: Some("2026-04-10T10:00:00.000+00:00".to_string()),
                due_date: Some("2026-04-15".to_string()),
                comment: Some(JiraCommentContainer {
                    comments: vec![
                        sample_comment("Ada", "2026-04-01T09:00:00.000+00:00", "Old"),
                        sample_comment("Grace", "2026-04-12T09:00:00.000+00:00", "Keep me"),
                    ],
                }),
            },
        };
        let pull_requests = vec![JiraPullRequest {
            id: None,
            name: Some("Implement release workflow".to_string()),
            status: Some("OPEN".to_string()),
            url: Some("https://github.com/org/repo/pull/42".to_string()),
            last_update: Some("2026-04-10T11:00:00.000+00:00".to_string()),
        }];

        let payload = build_issue_json(
            &issue,
            true,
            true,
            true,
            &pull_requests,
            5,
            false,
            Some("2026-04-05"),
        );

        assert_eq!(payload["ticket"], "RW-123");
        assert_eq!(payload["description"], "Hello\nWorld");
        assert_eq!(payload["comments_returned"], 1);
        assert_eq!(payload["comments_limit"], 5);
        assert_eq!(payload["comments_since"], "2026-04-05");
        assert_eq!(payload["comments"][0]["author"], "Grace");
        assert_eq!(payload["pull_requests_count"], 1);
        assert_eq!(payload["pull_requests"][0]["id"], "#42");
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

    fn sample_issue_with_summary(id: &str, key: &str, summary: &str) -> JiraIssue {
        JiraIssue {
            id: id.to_string(),
            key: key.to_string(),
            fields: JiraIssueFields {
                summary: summary.to_string(),
                status: Some(JiraStatus {
                    name: "In Progress".to_string(),
                }),
                sprint: Some(vec![JiraSprint {
                    name: "Sprint 42".to_string(),
                    state: "active".to_string(),
                }]),
                description: None,
                acceptance_criteria: None,
                assignee: None,
                reporter: None,
                priority: None,
                issuetype: None,
                created: None,
                updated: None,
                due_date: None,
                comment: None,
            },
        }
    }

    fn sample_comment(author: &str, created: &str, body: &str) -> JiraComment {
        JiraComment {
            author: Some(JiraUser {
                display_name: author.to_string(),
                account_id: None,
            }),
            body: Some(text_to_adf(body)),
            created: Some(created.to_string()),
            updated: Some(created.to_string()),
        }
    }

    fn sample_issue_with_comments(comments: Vec<JiraComment>) -> JiraIssue {
        JiraIssue {
            id: "10001".to_string(),
            key: "RW-123".to_string(),
            fields: JiraIssueFields {
                summary: "Implement backlog creation".to_string(),
                status: Some(JiraStatus {
                    name: "In Progress".to_string(),
                }),
                sprint: None,
                description: None,
                acceptance_criteria: None,
                assignee: None,
                reporter: None,
                priority: None,
                issuetype: None,
                created: None,
                updated: None,
                due_date: None,
                comment: Some(JiraCommentContainer { comments }),
            },
        }
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
