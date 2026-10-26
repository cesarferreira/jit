mod support;
use serde_json::{Value, json};
use std::{fs, path::Path};
use support::*;

#[test]
fn description_file_and_stdin_feed_creation_and_editing() {
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 201 Created", r#"{"id":"10","key":"RW-123"}"#),
        ("HTTP/1.1 204 No Content", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let file = Path::new(config.path_str()).with_file_name("description.txt");
    fs::write(&file, "First line\nSecond line").unwrap();
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "create",
        "--project",
        "RW",
        "--summary",
        "File input",
        "--backlog",
        "--assignee",
        "unassigned",
        "--description-file",
        file.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let output = run_jit_with_stdin(
        [
            "--config-file",
            config.path_str(),
            "edit",
            "RW-123",
            "--description-file",
            "-",
            "--json",
        ],
        "Replacement text",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let captured = collect_requests(&requests, 2);
    let created: Value = serde_json::from_str(request_body(&captured[0])).unwrap();
    assert_eq!(
        created["fields"]["description"]["content"][0]["content"][0]["text"],
        "First line"
    );
    assert_eq!(
        created["fields"]["description"]["content"][0]["content"][2]["text"],
        "Second line"
    );
    let edited: Value = serde_json::from_str(request_body(&captured[1])).unwrap();
    assert_eq!(
        edited["fields"]["description"]["content"][0]["content"][0]["text"],
        "Replacement text"
    );
    server.join();
}

#[test]
fn json_argument_errors_are_machine_readable() {
    let output = run_jit([
        "create",
        "--project",
        "RW",
        "--summary",
        "Bad",
        "--description",
        "Inline",
        "--description-file",
        "-",
        "--json",
    ]);
    assert!(!output.status.success());
    assert!(stdout(&output).is_empty());
    let payload: Value = serde_json::from_str(stderr(&output).trim()).expect("JSON argument error");
    assert_eq!(payload["error"]["code"], "invalid_arguments");
    assert_eq!(payload["error"]["completed_operations"], json!([]));
}

#[test]
fn json_partial_creation_failure_reports_created_ticket_for_safe_retry() {
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 200 OK", r#"{"id":42,"name":"Board"}"#),
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":300,"name":"Sprint"}],"isLast":true}"#,
        ),
        ("HTTP/1.1 201 Created", r#"{"id":"10","key":"RW-123"}"#),
        (
            "HTTP/1.1 403 Forbidden",
            r#"{"errorMessages":["Permission denied"]}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "create",
        "--project",
        "RW",
        "--summary",
        "Partial",
        "--assignee",
        "unassigned",
        "--board",
        "42",
        "--json",
    ]);
    assert!(!output.status.success());
    assert!(stdout(&output).is_empty());
    let payload: Value = serde_json::from_str(&stderr(&output)).expect("JSON mutation error");
    assert_eq!(payload["error"]["code"], "partial_failure");
    assert_eq!(payload["error"]["ticket"], "RW-123");
    assert_eq!(payload["error"]["completed_operations"], json!(["created"]));
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Permission denied")
    );
    assert_eq!(collect_requests(&requests, 4).len(), 4);
    server.join();
}

#[test]
fn json_api_errors_report_http_status_and_ticket() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 403 Forbidden",
        r#"{"errorMessages":["Permission denied"]}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "edit",
        "RW-123",
        "--summary",
        "Denied",
        "--json",
    ]);
    assert!(!output.status.success());
    let payload: Value = serde_json::from_str(&stderr(&output)).expect("JSON API error");
    assert_eq!(payload["error"]["code"], "forbidden");
    assert_eq!(payload["error"]["status"], 403);
    assert_eq!(payload["error"]["ticket"], "RW-123");
    assert_eq!(payload["error"]["completed_operations"], json!([]));
    collect_requests(&requests, 1);
    server.join();
}

#[test]
fn configured_custom_fields_are_used_for_issue_reads() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"id":"10","key":"RW-123","fields":{"summary":"Portable fields","customfield_22222":[{"name":"Configured sprint","state":"active"}],"customfield_33333":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Configured criteria"}]}]}}}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let mut content = fs::read_to_string(config.path_str()).unwrap();
    content.push_str(
        "sprint_field = \"customfield_22222\"\nacceptance_criteria_field = \"customfield_33333\"\n",
    );
    fs::write(config.path_str(), content).unwrap();
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "--json",
        "--include-description",
        "RW-123",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["sprint"], "Configured sprint");
    assert_eq!(payload["acceptance_criteria"], "Configured criteria");
    let captured = collect_requests(&requests, 1);
    assert!(captured[0].contains("customfield_22222"));
    assert!(captured[0].contains("customfield_33333"));
    assert!(!captured[0].contains("customfield_10020"));
    server.join();
}

#[test]
fn my_tickets_json_follows_search_cursor_and_reports_truncation() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"issues":[{"id":"10","key":"RW-123","fields":{"summary":"First"}}],"nextPageToken":"page-two","isLast":false}"#,
        ),
        (
            "HTTP/1.1 200 OK",
            r#"{"issues":[{"id":"11","key":"RW-124","fields":{"summary":"Second"}}],"nextPageToken":"page-three","isLast":false}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "--my-tickets",
        "--json",
        "--limit",
        "2",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("JSON ticket list");
    assert_eq!(payload["issues"][0]["ticket"], "RW-123");
    assert_eq!(payload["issues"][1]["ticket"], "RW-124");
    assert_eq!(payload["truncated"], true);
    let captured = collect_requests(&requests, 2);
    let query: Value = serde_json::from_str(request_body(&captured[1])).unwrap();
    assert_eq!(query["nextPageToken"], "page-two");
    server.join();
}

#[test]
fn all_comments_fetches_pages_when_embedded_comments_are_incomplete() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"id":"10","key":"RW-123","fields":{"summary":"Comments","comment":{"comments":[],"total":2}}}"#,
        ),
        (
            "HTTP/1.1 200 OK",
            r#"{"startAt":0,"maxResults":1,"total":2,"comments":[{"body":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"First"}]}]}}]}"#,
        ),
        (
            "HTTP/1.1 200 OK",
            r#"{"startAt":1,"maxResults":1,"total":2,"comments":[{"body":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Second"}]}]}}]}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "--json",
        "--include-comments",
        "--all-comments",
        "RW-123",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["comments"].as_array().unwrap().len(), 2);
    assert_eq!(payload["comments"][1]["body"], "Second");
    let captured = collect_requests(&requests, 3);
    assert!(captured[1].contains("/issue/RW-123/comment?startAt=0"));
    assert!(captured[2].contains("startAt=1"));
    server.join();
}

#[test]
fn edit_partial_failure_reports_core_fields_completed() {
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 200 OK", r#"{"id":42,"name":"Board"}"#),
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":300,"name":"Sprint"}],"isLast":true}"#,
        ),
        ("HTTP/1.1 204 No Content", ""),
        (
            "HTTP/1.1 403 Forbidden",
            r#"{"errorMessages":["Permission denied"]}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "edit",
        "RW-123",
        "--summary",
        "Saved",
        "--current-sprint",
        "--board",
        "42",
        "--json",
    ]);
    assert!(!output.status.success());
    let payload: Value = serde_json::from_str(&stderr(&output)).unwrap();
    assert_eq!(
        payload["error"]["completed_operations"],
        json!(["core_fields_updated"])
    );
    assert_eq!(payload["error"]["ticket"], "RW-123");
    assert_eq!(payload["error"]["status"], 403);
    collect_requests(&requests, 4);
    server.join();
}

#[test]
fn top_level_json_flag_applies_to_subcommand_successes() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"transitions":[{"id":"21","name":"In Progress"}]}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "--json",
        "transitions",
        "RW-123",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let payload: Value =
        serde_json::from_str(&stdout(&output)).expect("JSON inherited by subcommand");
    assert_eq!(payload["transitions"][0]["id"], "21");
    collect_requests(&requests, 1);
    server.join();
}

#[test]
fn auth_token_refresh_preserves_configured_field_ids() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"accountId":"current-account","displayName":"Current user"}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let mut text = fs::read_to_string(config.path_str()).unwrap();
    text.push_str(
        "sprint_field = \"customfield_22222\"\nacceptance_criteria_field = \"customfield_33333\"\n",
    );
    fs::write(config.path_str(), text).unwrap();
    let output = run_jit_with_stdin(
        ["--config-file", config.path_str(), "auth"],
        "y\n\n\nnew-token\n",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = fs::read_to_string(config.path_str()).unwrap();
    let saved: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(
        saved["jira"]["sprint_field"].as_str(),
        Some("customfield_22222")
    );
    assert_eq!(
        saved["jira"]["acceptance_criteria_field"].as_str(),
        Some("customfield_33333")
    );
    collect_requests(&requests, 1);
    server.join();
}

#[test]
fn json_errors_preserve_status_for_auth_checks_and_pr_reads() {
    for auth_check in [true, false] {
        let first = if auth_check {
            r#"{"issues":[],"isLast":true}"#
        } else {
            r#"{"id":"10","key":"RW-123","fields":{"summary":"PR read"}}"#
        };
        let (server, requests) = spawn_sequence_server(vec![
            ("HTTP/1.1 200 OK", first),
            (
                "HTTP/1.1 401 Unauthorized",
                r#"{"errorMessages":["Unauthorized"]}"#,
            ),
        ]);
        let config = TempConfig::new(&server.base_url);
        let mut args = vec!["--config-file", config.path_str(), "--json"];
        if auth_check {
            args.push("--my-tickets");
        } else {
            args.extend(["--include-prs", "RW-123"]);
        }
        let output = run_jit(args);
        assert!(!output.status.success());
        let payload: Value = serde_json::from_str(&stderr(&output)).unwrap();
        assert_eq!(payload["error"]["code"], "unauthorized");
        assert_eq!(payload["error"]["status"], 401);
        assert!(stdout(&output).is_empty());
        collect_requests(&requests, 2);
        server.join();
    }
}
