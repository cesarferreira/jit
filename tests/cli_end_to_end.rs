use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

static TEMP_CONFIG_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[test]
fn default_query_without_ticket_lists_current_sprint_table() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"issues":[{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation","status":{"name":"In Progress"},"customfield_10020":[{"name":"Sprint 42","state":"active"}]}}]}"#,
    )]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit(["--config-file", config.path_str()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = stdout(&output);
    assert!(stdout.contains("Current Sprint: Sprint 42"));
    assert!(stdout.contains("RW-123"));
    assert!(stdout.contains("Implement backlog creation"));

    let captured = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("search request should be captured");
    assert!(captured.starts_with("POST /rest/api/3/search/jql HTTP/1.1"));

    server.join();
}

#[test]
fn my_tickets_reports_auth_error_when_empty_search_masks_expired_token() {
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 200 OK", r#"{"issues":[],"isLast":true}"#),
        (
            "HTTP/1.1 401 Unauthorized",
            r#"{"errorMessages":["Unauthorized"]}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit(["--config-file", config.path_str(), "--my-tickets"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Jira authentication failed"),
        "stderr was: {}",
        stderr(&output)
    );
    assert!(
        !stdout(&output).contains("No tickets found in the current sprint."),
        "stdout was: {}",
        stdout(&output)
    );

    let captured = collect_requests(&requests, 2);
    assert!(captured[0].starts_with("POST /rest/api/3/search/jql HTTP/1.1"));
    assert!(captured[1].starts_with("GET /rest/api/3/myself HTTP/1.1"));

    server.join();
}

#[test]
fn auth_wizard_writes_config_after_validating_credentials() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"accountId":"account-id-123","displayName":"Cesar Ferreira"}"#,
    )]);
    let config = TempConfig::empty();

    let output = run_jit_with_stdin(
        ["--config-file", config.path_str(), "auth"],
        &format!("{}\nuser@example.com\ntoken-123\n", server.base_url),
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = stdout(&output);
    assert!(stdout.contains("Open this URL to create an Atlassian API token:"));
    assert!(stdout.contains("Saved Jira credentials to"));

    let contents = fs::read_to_string(&config.path).expect("config should be written");
    assert!(contents.contains("[jira]"));
    assert!(contents.contains(&format!("base_url = \"{}\"", server.base_url)));
    assert!(contents.contains("api_token = \"token-123\""));
    assert!(contents.contains("user_email = \"user@example.com\""));

    let captured = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("auth validation request should be captured");
    assert!(captured.starts_with("GET /rest/api/3/myself HTTP/1.1"));

    server.join();
}

#[test]
fn auth_wizard_rejects_invalid_token_without_writing_config() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 401 Unauthorized",
        r#"{"errorMessages":["Unauthorized"]}"#,
    )]);
    let config = TempConfig::empty();

    let output = run_jit_with_stdin(
        ["--config-file", config.path_str(), "auth"],
        &format!("{}\nuser@example.com\nbad-token\n", server.base_url),
    );

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Jira authentication failed"),
        "stderr was: {}",
        stderr(&output)
    );
    assert!(
        !config.path.exists(),
        "config should not be written when auth fails"
    );

    let captured = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("auth validation request should be captured");
    assert!(captured.starts_with("GET /rest/api/3/myself HTTP/1.1"));

    server.join();
}

#[test]
fn auth_wizard_uses_existing_url_and_email_when_prompts_are_empty() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"accountId":"account-id-123","displayName":"Cesar Ferreira"}"#,
    )]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit_with_stdin(
        ["--config-file", config.path_str(), "auth"],
        "y\n\n\nnew-token\n",
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(&format!("Jira company URL [{}]:", server.base_url)),
        "stdout was: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("Jira account email [user@example.com]:"),
        "stdout was: {}",
        stdout(&output)
    );

    let contents = fs::read_to_string(&config.path).expect("config should be written");
    assert!(contents.contains(&format!("base_url = \"{}\"", server.base_url)));
    assert!(contents.contains("api_token = \"new-token\""));
    assert!(contents.contains("user_email = \"user@example.com\""));

    let captured = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("auth validation request should be captured");
    assert!(captured.starts_with("GET /rest/api/3/myself HTTP/1.1"));

    server.join();
}

#[test]
fn missing_explicit_config_suggests_auth_wizard() {
    let config = TempConfig::empty();

    let output = run_jit(["--config-file", config.path_str(), "--my-tickets"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Run `jit auth --config-file"),
        "stderr was: {}",
        stderr(&output)
    );
}

#[test]
fn default_ticket_query_prints_summary_lines() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation"}}"#,
    )]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit(["--config-file", config.path_str(), "RW-123"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = stdout(&output);
    assert!(stdout.contains("Ticket:   RW-123"));
    assert!(stdout.contains("Summary:  Implement backlog creation"));

    let captured = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("issue request should be captured");
    assert!(captured.starts_with("GET /rest/api/3/issue/RW-123?fields=summary HTTP/1.1"));

    server.join();
}

#[test]
fn text_ticket_query_prints_one_line_summary() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation"}}"#,
    )]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit(["--config-file", config.path_str(), "--text", "RW-123"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "RW-123: Implement backlog creation");

    let captured = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("issue request should be captured");
    assert!(captured.starts_with("GET /rest/api/3/issue/RW-123?fields=summary HTTP/1.1"));

    server.join();
}

#[test]
fn show_full_ticket_query_prints_details_description_comments_and_prs() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation","status":{"name":"In Progress"},"customfield_10020":[{"name":"Sprint 42","state":"active"}],"description":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"},{"type":"hardBreak"},{"type":"text","text":"World"}]}]},"assignee":{"displayName":"Cesar Ferreira","accountId":"account-id-123"},"reporter":{"displayName":"Ada Lovelace","accountId":"account-id-999"},"priority":{"name":"High"},"issuetype":{"name":"Task"},"created":"2026-04-10T09:00:00.000+00:00","updated":"2026-04-10T10:00:00.000+00:00","duedate":"2026-04-15","comment":{"comments":[{"author":{"displayName":"Grace Hopper","accountId":"account-id-555"},"body":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Latest comment"}]}]},"created":"2026-04-12T09:00:00.000+00:00","updated":"2026-04-12T10:00:00.000+00:00"}]}}}"#,
        ),
        (
            "HTTP/1.1 200 OK",
            r##"{"detail":[{"pullRequests":[{"id":"#42","name":"Implement release workflow","status":"OPEN","url":"https://github.com/org/repo/pull/42","lastUpdate":"2026-04-10T11:00:00.000+00:00"}]}]}"##,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit([
        "--config-file",
        config.path_str(),
        "--show",
        "--full",
        "RW-123",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = stdout(&output);
    assert!(stdout.contains("TICKET DETAILS"));
    assert!(stdout.contains("RW-123: Implement backlog creation"));
    assert!(stdout.contains("DESCRIPTION"));
    assert!(stdout.contains("Hello\nWorld"));
    assert!(stdout.contains("COMMENTS"));
    assert!(stdout.contains("#1 Grace Hopper | created: 2026-04-12T09:00:00.000+00:00 | updated: 2026-04-12T10:00:00.000+00:00"));
    assert!(stdout.contains("Latest comment"));
    assert!(stdout.contains("PULL REQUESTS"));
    assert!(stdout.contains("#1 #42 [OPEN] | updated: 2026-04-10T11:00:00.000+00:00"));
    assert!(stdout.contains("https://github.com/org/repo/pull/42"));

    let first = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("issue request should be captured");
    let second = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("PR request should be captured");
    assert!(first.starts_with("GET /rest/api/3/issue/RW-123?fields="));
    assert!(first.contains("comment"));
    assert!(first.contains("description"));
    assert!(second.starts_with(
        "GET /rest/dev-status/latest/issue/detail?issueId=10001&applicationType=GitHub&dataType=pullrequest HTTP/1.1"
    ));

    server.join();
}

#[test]
fn create_current_sprint_json_runs_full_creation_flow() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"accountId":"account-id-999","displayName":"Cesar Ferreira"}"#,
        ),
        ("HTTP/1.1 200 OK", r#"{"id":42,"name":"Explicit board"}"#),
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":300,"name":"Board Sprint","startDate":"2026-04-01T09:00:00+00:00"}],"isLast":true,"maxResults":50,"startAt":0}"#,
        ),
        ("HTTP/1.1 201 Created", r#"{"id":"10001","key":"RW-123"}"#),
        ("HTTP/1.1 204 No Content", ""),
    ]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit([
        "--config-file",
        config.path_str(),
        "create",
        "--project",
        "RW",
        "--summary",
        "Implement backlog creation",
        "--current-sprint",
        "--board",
        "42",
        "--json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let payload: Value =
        serde_json::from_str(stdout(&output).trim()).expect("create output should be json");
    assert_eq!(payload["ticket"], "RW-123");
    assert_eq!(payload["project"], "RW");
    assert_eq!(payload["summary"], "Implement backlog creation");
    assert_eq!(payload["assignee"], "Cesar Ferreira");
    assert_eq!(payload["board"], "Explicit board");
    assert_eq!(payload["sprint"], "Board Sprint");
    assert_eq!(payload["backlog"], false);

    let captured = collect_requests(&requests, 5);
    assert!(captured[0].starts_with("GET /rest/api/3/myself HTTP/1.1"));
    assert!(captured[1].starts_with("GET /rest/agile/1.0/board/42 HTTP/1.1"));
    assert!(captured[2].starts_with("GET /rest/agile/1.0/board/42/sprint?state=active"));
    assert!(captured[3].starts_with("POST /rest/api/3/issue HTTP/1.1"));
    assert!(captured[4].starts_with("POST /rest/agile/1.0/sprint/300/issue HTTP/1.1"));

    server.join();
}

#[test]
fn create_defaults_to_current_user_and_current_sprint() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"accountId":"current-account","displayName":"Current user"}"#,
        ),
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":42,"name":"Project board"}],"isLast":true}"#,
        ),
        SPRINT_RESPONSE,
        ("HTTP/1.1 201 Created", r#"{"id":"10001","key":"RW-123"}"#),
        ("HTTP/1.1 204 No Content", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "create",
        "--project",
        "RW",
        "--summary",
        "Default ticket",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["assignee"], "Current user");
    assert_eq!(payload["sprint"], "Board Sprint");
    assert_eq!(payload["backlog"], false);
    let captured = collect_requests(&requests, 5);
    assert!(captured[0].starts_with("GET /rest/api/3/myself HTTP"));
    assert!(captured[1].contains("projectKeyOrId=RW"));
    assert!(captured[3].starts_with("POST /rest/api/3/issue HTTP"));
    let body: Value = serde_json::from_str(request_body(&captured[3])).unwrap();
    assert_eq!(body["fields"]["assignee"]["id"], "current-account");
    assert!(captured[4].starts_with("POST /rest/agile/1.0/sprint/300/issue HTTP"));
    server.join();
}

#[test]
fn create_backlog_skips_sprint_and_preserves_assignee_choices() {
    for assignee in ["me", "unassigned", "another-account"] {
        let mut responses = vec![];
        if assignee == "me" {
            responses.push((
                "HTTP/1.1 200 OK",
                r#"{"accountId":"current-account","displayName":"Current user"}"#,
            ));
        }
        responses.push(("HTTP/1.1 201 Created", r#"{"id":"10001","key":"RW-123"}"#));
        let (server, requests) = spawn_sequence_server(responses);
        let config = TempConfig::new(&server.base_url);
        let output = run_jit([
            "--config-file",
            config.path_str(),
            "create",
            "--project",
            "RW",
            "--summary",
            "Backlog ticket",
            "--backlog",
            "--assignee",
            assignee,
            "--json",
        ]);
        assert!(output.status.success(), "{}", stderr(&output));
        let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(payload["backlog"], true);
        assert!(payload.get("sprint").is_none());
        let captured = collect_requests(&requests, if assignee == "me" { 2 } else { 1 });
        let request = captured.last().unwrap();
        assert!(request.starts_with("POST /rest/api/3/issue HTTP"));
        let body: Value = serde_json::from_str(request_body(request)).unwrap();
        match assignee {
            "me" => assert_eq!(body["fields"]["assignee"]["id"], "current-account"),
            "unassigned" => assert!(body["fields"].get("assignee").is_none()),
            _ => assert_eq!(body["fields"]["assignee"]["id"], "another-account"),
        }
        server.join();
    }
}

#[test]
fn create_board_selects_default_sprint_without_current_sprint_flag() {
    let (server, requests) = spawn_sequence_server(vec![
        BOARD_RESPONSE,
        SPRINT_RESPONSE,
        ("HTTP/1.1 201 Created", r#"{"id":"10001","key":"RW-123"}"#),
        ("HTTP/1.1 204 No Content", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "create",
        "--project",
        "RW",
        "--summary",
        "Board ticket",
        "--board",
        "42",
        "--assignee",
        "unassigned",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["sprint"], "Board Sprint");
    assert_eq!(payload["backlog"], false);
    let captured = collect_requests(&requests, 4);
    assert!(captured[0].starts_with("GET /rest/agile/1.0/board/42 HTTP"));
    assert!(captured[3].starts_with("POST /rest/agile/1.0/sprint/300/issue HTTP"));
    server.join();
}

#[test]
fn create_without_active_sprint_fails_before_creating_ticket() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":42,"name":"Project board"}],"isLast":true}"#,
        ),
        ("HTTP/1.1 200 OK", r#"{"values":[],"isLast":true}"#),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "create",
        "--project",
        "RW",
        "--summary",
        "Do not create",
        "--assignee",
        "unassigned",
    ]);
    assert!(!output.status.success());
    let error = stderr(&output);
    assert!(error.contains("No active sprint"), "{error}");
    assert!(error.contains("--backlog"), "{error}");
    assert_eq!(collect_requests(&requests, 2).len(), 2);
    server.join();
}

#[test]
fn create_backlog_conflicts_with_sprint_options() {
    for option in [vec!["--current-sprint"], vec!["--board", "42"]] {
        let mut args = vec![
            "create",
            "--project",
            "RW",
            "--summary",
            "Invalid",
            "--backlog",
        ];
        args.extend(option);
        let output = run_jit(args);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("cannot be used with"),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn edit_json_updates_requested_fields() {
    let (server, requests) = spawn_sequence_server(vec![("HTTP/1.1 204 No Content", "")]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit([
        "--config-file",
        config.path_str(),
        "edit",
        "RW-123",
        "--summary",
        "Improve edit flow",
        "--description",
        "",
        "--assignee",
        "unassigned",
        "--json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let payload: Value =
        serde_json::from_str(stdout(&output).trim()).expect("edit output should be json");
    assert_eq!(payload["ticket"], "RW-123");
    assert_eq!(payload["summary"], "Improve edit flow");
    assert_eq!(payload["description"], Value::Null);
    assert_eq!(payload["assignee"], "Unassigned");
    assert_eq!(
        payload["updated_fields"],
        Value::Array(vec![
            Value::String("summary".to_string()),
            Value::String("description".to_string()),
            Value::String("assignee".to_string()),
        ])
    );

    let captured = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("update request should be captured");
    assert!(captured.starts_with("PUT /rest/api/3/issue/RW-123 HTTP/1.1"));
    let body = request_body(&captured);
    let body_json: Value = serde_json::from_str(body).expect("update request body should be json");
    assert_eq!(body_json["fields"]["summary"], "Improve edit flow");
    assert_eq!(body_json["fields"]["description"], Value::Null);
    assert_eq!(body_json["fields"]["assignee"], Value::Null);

    server.join();
}

const BOARD_RESPONSE: (&str, &str) = ("HTTP/1.1 200 OK", r#"{"id":42,"name":"Explicit board"}"#);
const SPRINT_RESPONSE: (&str, &str) = (
    "HTTP/1.1 200 OK",
    r#"{"values":[{"id":300,"name":"Board Sprint","startDate":"2026-04-01T09:00:00+00:00"}],"isLast":true}"#,
);

#[test]
fn edit_current_sprint_only_uses_board_and_reports_json_for_key_and_url() {
    for ticket in ["RW-123", "https://company.atlassian.net/browse/RW-123"] {
        let (server, requests) = spawn_sequence_server(vec![
            BOARD_RESPONSE,
            SPRINT_RESPONSE,
            ("HTTP/1.1 204 No Content", ""),
        ]);
        let config = TempConfig::new(&server.base_url);
        let output = run_jit([
            "--config-file",
            config.path_str(),
            "edit",
            ticket,
            "--current-sprint",
            "--board",
            "42",
            "--json",
        ]);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(payload["ticket"], "RW-123");
        assert_eq!(payload["sprint"], "Board Sprint");
        assert_eq!(payload["sprint_id"], 300);
        assert_eq!(payload["board"], "Explicit board");
        assert_eq!(payload["board_id"], 42);
        assert_eq!(payload["updated_fields"], serde_json::json!(["sprint"]));
        let captured = collect_requests(&requests, 3);
        assert!(captured[0].starts_with("GET /rest/agile/1.0/board/42 HTTP"));
        assert!(captured[2].starts_with("POST /rest/agile/1.0/sprint/300/issue HTTP"));
        assert_eq!(
            serde_json::from_str::<Value>(request_body(&captured[2])).unwrap(),
            serde_json::json!({"issues":["RW-123"]})
        );
        server.join();
    }
}

#[test]
fn edit_current_sprint_resolves_project_and_combines_core_fields() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":42,"name":"Project board"}],"isLast":true}"#,
        ),
        SPRINT_RESPONSE,
        ("HTTP/1.1 204 No Content", ""),
        ("HTTP/1.1 204 No Content", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "edit",
        "RW-123",
        "--current-sprint",
        "--summary",
        "Updated summary",
        "--json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["summary"], "Updated summary");
    assert_eq!(payload["sprint"], "Board Sprint");
    assert_eq!(
        payload["updated_fields"],
        serde_json::json!(["summary", "sprint"])
    );
    let captured = collect_requests(&requests, 4);
    assert!(captured[0].contains("projectKeyOrId=RW"));
    assert!(captured[2].starts_with("PUT /rest/api/3/issue/RW-123 HTTP"));
    assert_eq!(
        serde_json::from_str::<Value>(request_body(&captured[2])).unwrap(),
        serde_json::json!({"fields":{"summary":"Updated summary"}})
    );
    assert!(captured[3].starts_with("POST /rest/agile/1.0/sprint/300/issue HTTP"));
    server.join();
}

#[test]
fn edit_current_sprint_failure_reports_whether_fields_were_saved() {
    for combined in [false, true] {
        let mut responses = vec![BOARD_RESPONSE, SPRINT_RESPONSE];
        if combined {
            responses.push(("HTTP/1.1 204 No Content", ""));
        }
        responses.push((
            "HTTP/1.1 403 Forbidden",
            r#"{"errorMessages":["Sprint permission denied"]}"#,
        ));
        let (server, requests) = spawn_sequence_server(responses);
        let config = TempConfig::new(&server.base_url);
        let mut args = vec![
            "--config-file",
            config.path_str(),
            "edit",
            "RW-123",
            "--current-sprint",
            "--board",
            "42",
            "--json",
        ];
        if combined {
            args.extend(["--summary", "Updated summary"]);
        }
        let output = run_jit(args);
        assert!(!output.status.success());
        assert!(stdout(&output).is_empty());
        let error = stderr(&output);
        assert!(
            error.contains("failed to add RW-123 to sprint Board Sprint"),
            "{error}"
        );
        assert!(error.contains("Sprint permission denied"), "{error}");
        assert_eq!(error.contains("Updated core fields"), combined, "{error}");
        assert_eq!(
            collect_requests(&requests, if combined { 4 } else { 3 }).len(),
            if combined { 4 } else { 3 }
        );
        server.join();
    }
}

#[test]
fn edit_current_sprint_no_active_sprint_does_not_update_fields() {
    let (server, requests) = spawn_sequence_server(vec![
        BOARD_RESPONSE,
        ("HTTP/1.1 200 OK", r#"{"values":[],"isLast":true}"#),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "edit",
        "RW-123",
        "--current-sprint",
        "--board",
        "42",
        "--summary",
        "Do not save",
    ]);
    assert!(!output.status.success());
    let error = stderr(&output);
    assert!(error.contains("No active sprint"), "{error}");
    assert!(error.contains("--board"), "{error}");
    assert_eq!(collect_requests(&requests, 2).len(), 2);
    server.join();
}

#[test]
fn edit_current_sprint_core_update_failure_does_not_move_issue() {
    let (server, requests) = spawn_sequence_server(vec![
        BOARD_RESPONSE,
        SPRINT_RESPONSE,
        (
            "HTTP/1.1 400 Bad Request",
            r#"{"errorMessages":["Invalid summary"]}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "edit",
        "RW-123",
        "--current-sprint",
        "--board",
        "42",
        "--summary",
        "Invalid",
    ]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("Invalid summary"));
    let captured = collect_requests(&requests, 3);
    assert!(captured[2].starts_with("PUT /rest/api/3/issue/RW-123 HTTP"));
    server.join();
}

#[test]
fn edit_board_requires_current_sprint() {
    let output = run_jit(["edit", "RW-123", "--board", "42"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--current-sprint"));
}

#[test]
fn edit_without_fields_fails_before_network_call() {
    let config = TempConfig::new("http://127.0.0.1:9");
    let output = run_jit(["--config-file", config.path_str(), "edit", "RW-123"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("No editable fields provided"),
        "stderr was: {}",
        stderr(&output)
    );
}

#[test]
fn invalid_since_fails_before_loading_configuration() {
    let output = run_jit(["--since", "2026/01/01", "RW-123"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Invalid --since value '2026/01/01'. Use YYYY-MM-DD."),
        "stderr was: {}",
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains("No configuration found"),
        "stderr was: {}",
        stderr(&output)
    );
}

struct TempConfig {
    dir: PathBuf,
    path: PathBuf,
}

impl TempConfig {
    fn empty() -> Self {
        let unique = TEMP_CONFIG_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jit-cli-e2e-{}-{unique}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp config directory should be created");
        let path = dir.join("config.toml");

        Self { dir, path }
    }

    fn new(base_url: &str) -> Self {
        let config = Self::empty();
        fs::write(
            &config.path,
            format!(
                "[jira]\nbase_url = \"{base_url}\"\napi_token = \"token-123\"\nuser_email = \"user@example.com\"\n"
            ),
        )
        .expect("temp config file should be written");

        config
    }

    fn path_str(&self) -> &str {
        self.path
            .to_str()
            .expect("temp config path should be valid utf-8")
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct TestServer {
    base_url: String,
    handle: thread::JoinHandle<()>,
}

impl TestServer {
    fn join(self) {
        self.handle.join().expect("server thread should finish");
    }
}

fn run_jit<'a>(args: impl IntoIterator<Item = &'a str>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("jit command should run")
}

fn run_jit_with_stdin<'a>(args: impl IntoIterator<Item = &'a str>, stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .env("NO_COLOR", "1")
        .env("JIT_AUTH_SKIP_OPEN", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("jit command should spawn");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(stdin.as_bytes())
        .expect("stdin should be written");

    child.wait_with_output().expect("jit command should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be utf-8")
}

fn request_body(request: &str) -> &str {
    request
        .split("\r\n\r\n")
        .nth(1)
        .expect("http request should contain a body")
}

fn collect_requests(receiver: &mpsc::Receiver<String>, expected: usize) -> Vec<String> {
    (0..expected)
        .map(|_| {
            receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("request should be captured")
        })
        .collect()
}

fn spawn_sequence_server(
    responses: Vec<(&'static str, &'static str)>,
) -> (TestServer, mpsc::Receiver<String>) {
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

    (
        TestServer {
            base_url: format!("http://{}", addr),
            handle,
        },
        rx,
    )
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut temp = [0; 1024];
    let mut header_end = None;
    let mut expected_len = 0usize;

    loop {
        match stream.read(&mut temp) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&temp[..n]);

                if header_end.is_none() {
                    header_end = find_header_end(&buffer);
                    if let Some(end) = header_end {
                        expected_len = parse_content_length(&buffer[..end]);
                    }
                }

                if let Some(end) = header_end {
                    let body_len = buffer.len().saturating_sub(end);
                    if body_len >= expected_len {
                        break;
                    }
                }
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(err) => panic!("failed reading http request: {err}"),
        }
    }

    String::from_utf8(buffer).expect("http request should be utf-8")
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn parse_content_length(headers: &[u8]) -> usize {
    let header_text = String::from_utf8_lossy(headers);
    header_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}
