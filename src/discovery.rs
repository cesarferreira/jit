use anyhow::{Result, bail};
use clap::Args;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::collections::HashSet;

use crate::api::json_response;

#[derive(Args, Debug)]
pub(crate) struct SearchArgs {
    /// Jira Query Language expression
    #[arg(long)]
    pub jql: String,
    #[arg(long)]
    pub json: bool,
    /// Maximum number of issues to return
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,
}

#[derive(Args, Debug)]
pub(crate) struct BoardsArgs {
    /// Filter boards by project key or ID
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct SprintsArgs {
    #[arg(long)]
    pub board: u64,
    /// Comma-separated sprint states: active, future, closed
    #[arg(long, value_parser = parse_states)]
    pub state: Option<String>,
    #[arg(long)]
    pub json: bool,
}

fn parse_states(value: &str) -> std::result::Result<String, String> {
    if value
        .split(',')
        .all(|state| matches!(state, "active" | "future" | "closed"))
    {
        Ok(value.to_owned())
    } else {
        Err("states must be active, future, or closed (comma-separated)".to_owned())
    }
}

pub(crate) fn search_issues(
    client: &Client,
    base: &str,
    jql: &str,
    fields: &[&str],
    limit: u32,
) -> Result<(Vec<Value>, bool)> {
    let mut issues = Vec::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    let mut seen_issues = HashSet::new();
    loop {
        let mut body = json!({"jql": jql, "fields": fields, "maxResults": (limit as usize - issues.len()).min(100)});
        if let Some(token) = &cursor {
            body["nextPageToken"] = json!(token);
        }
        let page = json_response(
            client
                .post(format!("{base}/rest/api/3/search/jql"))
                .json(&body)
                .send()?,
        )?;
        let values = page["issues"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Jira search response is missing issues"))?;
        let next = page["nextPageToken"]
            .as_str()
            .filter(|token| !token.is_empty());
        let has_more = page["isLast"] != true && next.is_some();
        if page["isLast"] == false && next.is_none() {
            bail!("Jira search pagination did not provide a nextPageToken");
        }
        if has_more && (values.is_empty() || !seen_cursors.insert(next.unwrap().to_owned())) {
            bail!("Jira search pagination did not advance");
        }
        let identities: Vec<_> = values
            .iter()
            .filter_map(|issue| issue["id"].as_str().or_else(|| issue["key"].as_str()))
            .collect();
        if !identities.is_empty() && identities.iter().all(|id| seen_issues.contains(*id)) {
            bail!("Jira search pagination repeated issues without advancing");
        }
        seen_issues.extend(identities.into_iter().map(str::to_owned));
        let remaining = limit as usize - issues.len();
        let extra = values.len() > remaining;
        issues.extend(values.iter().take(remaining).cloned());
        if issues.len() >= limit as usize || !has_more {
            return Ok((issues, extra || has_more));
        }
        cursor = next.map(str::to_owned);
    }
}

pub(crate) fn run_search(
    client: &Client,
    base: &str,
    args: &SearchArgs,
    sprint_field: &str,
) -> Result<()> {
    let (issues, truncated) = search_issues(
        client,
        base,
        &args.jql,
        &["summary", "status", "assignee", sprint_field],
        args.limit,
    )?;
    let issues: Vec<_> = issues.iter().map(|issue| {
        let fields = &issue["fields"];
        let sprint = fields[sprint_field].as_array().and_then(|sprints| {
            sprints.iter().find(|sprint| sprint["state"] == "active").or_else(|| sprints.last())
        }).and_then(|sprint| sprint["name"].as_str());
        json!({"ticket": issue["key"], "summary": fields["summary"], "status": fields["status"]["name"], "assignee": fields["assignee"]["displayName"], "sprint": sprint})
    }).collect();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"issues":issues,"truncated":truncated}))?
        );
    } else {
        for issue in &issues {
            println!(
                "{}: {} [{}]",
                text(&issue["ticket"]),
                text(&issue["summary"]),
                text(&issue["status"])
            );
        }
        if truncated {
            eprintln!(
                "Results truncated at {} issues; increase --limit to see more.",
                args.limit
            );
        }
    }
    Ok(())
}

fn agile_values(client: &Client, url: &str, filter: Option<(&str, &str)>) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    let mut start = 0_u64;
    let mut seen_pages = HashSet::new();
    loop {
        let mut request = client
            .get(url)
            .query(&[("startAt", start), ("maxResults", 100)]);
        if let Some((key, value)) = filter {
            request = request.query(&[(key, value)]);
        }
        let page = json_response(request.send()?)?;
        let batch = page["values"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Jira discovery response is missing values"))?;
        if page["startAt"]
            .as_u64()
            .is_some_and(|offset| offset != start)
        {
            bail!("Jira discovery pagination did not advance");
        }
        if !batch.is_empty() && !seen_pages.insert(serde_json::to_string(batch)?) {
            bail!("Jira discovery pagination repeated a page without advancing");
        }
        let next = start + batch.len() as u64;
        let has_more = match page["isLast"].as_bool() {
            Some(is_last) => !is_last,
            None => page["total"]
                .as_u64()
                .map(|total| next < total)
                .unwrap_or_else(|| {
                    page["maxResults"]
                        .as_u64()
                        .is_some_and(|size| size > 0 && batch.len() as u64 >= size)
                }),
        };
        if has_more && batch.is_empty() {
            bail!("Jira discovery pagination did not advance");
        }
        values.extend(batch.iter().cloned());
        if !has_more {
            return Ok(values);
        }
        start = next;
    }
}

pub(crate) fn run_boards(client: &Client, base: &str, args: &BoardsArgs) -> Result<()> {
    let boards = agile_values(
        client,
        &format!("{base}/rest/agile/1.0/board"),
        args.project
            .as_deref()
            .map(|project| ("projectKeyOrId", project)),
    )?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"boards":boards}))?
        );
    } else {
        for board in boards {
            println!(
                "{}: {} [{}]",
                board["id"],
                text(&board["name"]),
                text(&board["type"])
            );
        }
    }
    Ok(())
}

pub(crate) fn run_sprints(client: &Client, base: &str, args: &SprintsArgs) -> Result<()> {
    let sprints = agile_values(
        client,
        &format!("{base}/rest/agile/1.0/board/{}/sprint", args.board),
        args.state.as_deref().map(|state| ("state", state)),
    )?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"sprints":sprints}))?
        );
    } else {
        for sprint in sprints {
            println!(
                "{}: {} [{}]",
                sprint["id"],
                text(&sprint["name"]),
                text(&sprint["state"])
            );
        }
    }
    Ok(())
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
