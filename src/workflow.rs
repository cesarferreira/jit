use anyhow::{Result, anyhow, bail};
use clap::Args;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::path::PathBuf;

use crate::api::{empty_response, json_response};
use crate::{extract_ticket_id, read_text_input, text_to_adf};

#[derive(Args, Debug)]
pub(crate) struct TransitionsArgs {
    /// Jira issue key or browse URL
    pub ticket: String,
    /// Output available transitions as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct TransitionArgs {
    /// Jira issue key or browse URL
    pub ticket: String,
    /// Available transition ID or name (case insensitive)
    #[arg(long)]
    pub to: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct CommentArgs {
    /// Jira issue key or browse URL
    pub ticket: String,
    /// Plain-text comment body
    #[arg(
        long,
        conflicts_with = "body_file",
        required_unless_present = "body_file"
    )]
    pub body: Option<String>,
    /// Read comment text from a file, or '-' for stdin
    #[arg(long, conflicts_with = "body", required_unless_present = "body")]
    pub body_file: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct LinkArgs {
    /// Jira issue key or browse URL
    pub ticket: String,
    /// Issue blocked by this ticket
    #[arg(long, conflicts_with_all = ["target", "link_type"])]
    pub blocks: Option<String>,
    /// Target issue for a link using the type's outward direction
    #[arg(long = "to", requires = "link_type")]
    pub target: Option<String>,
    /// Link type name or ID, discovered from Jira
    #[arg(long = "type", requires = "target")]
    pub link_type: Option<String>,
    #[arg(long)]
    pub json: bool,
}

fn transitions(client: &Client, base: &str, ticket: &str) -> Result<Vec<Value>> {
    let response = json_response(
        client
            .get(format!("{base}/rest/api/3/issue/{ticket}/transitions"))
            .send()?,
    )?;
    response["transitions"]
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("Jira did not return a transitions list for {ticket}"))
}

pub(crate) fn run_transitions(client: &Client, base: &str, args: &TransitionsArgs) -> Result<()> {
    let ticket = extract_ticket_id(&args.ticket)?;
    let transitions = transitions(client, base, &ticket)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"ticket":ticket,"transitions":transitions}))?
        );
    } else if transitions.is_empty() {
        println!("No transitions available for {ticket}.");
    } else {
        for transition in transitions {
            println!(
                "{}\t{}",
                string(&transition, "id"),
                string(&transition, "name")
            );
        }
    }
    Ok(())
}

pub(crate) fn run_transition(client: &Client, base: &str, args: &TransitionArgs) -> Result<()> {
    let ticket = extract_ticket_id(&args.ticket)?;
    let transitions = transitions(client, base, &ticket)?;
    let ids: Vec<_> = transitions
        .iter()
        .filter(|item| string(item, "id") == args.to)
        .collect();
    let matches = if ids.is_empty() {
        transitions
            .iter()
            .filter(|item| string(item, "name").eq_ignore_ascii_case(&args.to))
            .collect()
    } else {
        ids
    };
    let available = transitions
        .iter()
        .map(|item| format!("{} ({})", string(item, "name"), string(item, "id")))
        .collect::<Vec<_>>()
        .join(", ");
    let transition = match matches.as_slice() {
        [item] => *item,
        [] => bail!(
            "Unknown transition '{}'. Available transitions: {}. Run `jit transitions {ticket}`.",
            args.to,
            if available.is_empty() {
                "none"
            } else {
                &available
            }
        ),
        _ => bail!(
            "Transition '{}' is ambiguous. Use an ID. Available transitions: {available}",
            args.to
        ),
    };
    let id = transition["id"]
        .as_str()
        .ok_or_else(|| anyhow!("Jira transition is missing its ID"))?;
    empty_response(
        client
            .post(format!("{base}/rest/api/3/issue/{ticket}/transitions"))
            .json(&json!({"transition":{"id":id}}))
            .send()?,
    )?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"ticket":ticket,"action":"transition","id":id,"name":transition["name"],"status":"success"})
            )?
        );
    } else {
        println!(
            "Transitioned {ticket}: {} ({id})",
            string(transition, "name")
        );
    }
    Ok(())
}

pub(crate) fn run_comment(client: &Client, base: &str, args: &CommentArgs) -> Result<()> {
    let ticket = extract_ticket_id(&args.ticket)?;
    let body = read_text_input(args.body.as_deref(), args.body_file.as_deref())?
        .filter(|body| !body.trim().is_empty())
        .ok_or_else(|| anyhow!("Comment body must not be empty. Use --body or --body-file."))?;
    let response = json_response(
        client
            .post(format!("{base}/rest/api/3/issue/{ticket}/comment"))
            .json(&json!({"body":text_to_adf(&body)}))
            .send()?,
    )?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"ticket":ticket,"action":"comment","id":response["id"],"status":"success"})
            )?
        );
    } else {
        println!("Added comment {} to {ticket}", string(&response, "id"));
    }
    Ok(())
}

pub(crate) fn run_link(client: &Client, base: &str, args: &LinkArgs) -> Result<()> {
    let (target, kind) = match (&args.blocks, &args.target, &args.link_type) {
        (Some(target), None, None) => (target, None),
        (None, Some(target), Some(kind)) => (target, Some(kind)),
        _ => bail!("Use either --blocks KEY or --to KEY --type NAME."),
    };
    let ticket = extract_ticket_id(&args.ticket)?;
    let target = extract_ticket_id(target)?;
    let response = json_response(
        client
            .get(format!("{base}/rest/api/3/issueLinkType"))
            .send()?,
    )?;
    let types = response["issueLinkTypes"]
        .as_array()
        .ok_or_else(|| anyhow!("Jira did not return issue link types"))?;
    let matches: Vec<_> = types
        .iter()
        .filter(|item| match kind {
            Some(kind) => {
                string(item, "id") == kind || string(item, "name").eq_ignore_ascii_case(kind)
            }
            None => {
                string(item, "outward").eq_ignore_ascii_case("blocks")
                    || string(item, "inward").eq_ignore_ascii_case("blocks")
            }
        })
        .collect();
    let available = types
        .iter()
        .map(|item| format!("{} ({})", string(item, "name"), string(item, "id")))
        .collect::<Vec<_>>()
        .join(", ");
    let link_type = match matches.as_slice() {
        [item] => *item,
        [] => bail!(
            "Unknown link type '{}'. Available link types: {available}",
            kind.map(String::as_str).unwrap_or("blocks")
        ),
        _ => bail!(
            "Link type '{}' is ambiguous. Available link types: {available}",
            kind.map(String::as_str).unwrap_or("blocks")
        ),
    };
    let name = link_type["name"]
        .as_str()
        .ok_or_else(|| anyhow!("Jira link type is missing its name"))?;
    let reverse = kind.is_none() && !string(link_type, "outward").eq_ignore_ascii_case("blocks");
    let (inward, outward) = if reverse {
        (&target, &ticket)
    } else {
        (&ticket, &target)
    };
    empty_response(client.post(format!("{base}/rest/api/3/issueLink")).json(&json!({"type":{"name":name},"inwardIssue":{"key":inward},"outwardIssue":{"key":outward}})).send()?)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"ticket":ticket,"action":"link","target":target,"type":name,"status":"success"})
            )?
        );
    } else {
        let direction = if reverse {
            string(link_type, "inward")
        } else {
            string(link_type, "outward")
        };
        println!("Linked {ticket} {direction} {target}");
    }
    Ok(())
}

fn string<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("")
}
