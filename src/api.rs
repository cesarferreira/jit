use anyhow::{Context, Result};
use reqwest::blocking::Response;
use serde_json::Value;
use std::fmt;

#[derive(Debug)]
pub(crate) struct ApiError {
    pub status: u16,
    pub body: String,
}

impl ApiError {
    pub fn code(&self) -> &'static str {
        match self.status {
            400 | 422 => "invalid_request",
            401 => "unauthorized",
            403 => "forbidden",
            404 => "not_found",
            409 => "conflict",
            429 => "rate_limited",
            500..=599 => "server_error",
            _ => "api_error",
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JIRA API request failed with status: {} - {}",
            self.status, self.body
        )
    }
}
impl std::error::Error for ApiError {}

pub(crate) fn checked_response(response: Response) -> Result<Response> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(ApiError {
            status: response.status().as_u16(),
            body: response.text().unwrap_or_default(),
        }
        .into())
    }
}

pub(crate) fn json_response(response: Response) -> Result<Value> {
    checked_response(response)?
        .json()
        .context("Failed to parse JIRA API response")
}

pub(crate) fn empty_response(response: Response) -> Result<()> {
    checked_response(response)?;
    Ok(())
}
