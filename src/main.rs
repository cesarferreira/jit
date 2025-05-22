// src/main.rs
use anyhow::{Context, Result};
use clap::Parser;
use headless_chrome::{Browser, LaunchOptionsBuilder};
use regex::Regex;

#[derive(Parser)]
struct Cli {
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    // 1. Launch Chrome in headless/background mode
    let browser = Browser::new(
        LaunchOptionsBuilder::default()
            .headless(true)
            .build()
            .context("launching Chrome")?,
    )?;

    // 2. Grab first tab and navigate
    let tab = browser.wait_for_initial_tab()?;
    tab.navigate_to(&args.url)?;
    tab.wait_until_navigated()?;

    // 3. Read <title> from DOM
    let title = tab.get_title()?;

    // 4. Regex-parse “[KEY-123] Summary”
    let re = Regex::new(r"\[(?P<key>[A-Z]+-\d+)]\s+(?P<summary>.+)")?;
    let caps = re.captures(&title).context("title not in expected format")?;
    println!("Ticket:   {}", &caps["key"]);
    println!("Summary:  {}", &caps["summary"]);
    Ok(())
}
