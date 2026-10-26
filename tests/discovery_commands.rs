mod support;
use serde_json::{Value, json};
use support::*;

#[test]
fn search_follows_cursor_and_normalizes_issue_fields() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"issues":[{"key":"RW-1","fields":{"summary":"First","status":{"name":"Open"},"assignee":{"displayName":"Cesar"},"customfield_10020":[{"id":9,"name":"Sprint 9","state":"active"}]}}],"nextPageToken":"next","isLast":false}"#,
        ),
        (
            "HTTP/1.1 200 OK",
            r#"{"issues":[{"key":"RW-2","fields":{"summary":"Second","status":{"name":"Done"},"assignee":null}}],"isLast":true}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "search",
        "--jql",
        "project = RW",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(result["truncated"], false);
    assert_eq!(result["issues"][0]["ticket"], "RW-1");
    assert_eq!(result["issues"][0]["assignee"], "Cesar");
    assert_eq!(result["issues"][0]["sprint"], "Sprint 9");
    assert_eq!(result["issues"][1]["assignee"], Value::Null);
    let captured = collect_requests(&requests, 2);
    let first: Value = serde_json::from_str(request_body(&captured[0])).unwrap();
    let second: Value = serde_json::from_str(request_body(&captured[1])).unwrap();
    assert!(captured[0].starts_with("POST /rest/api/3/search/jql "));
    assert_eq!(first["jql"], "project = RW");
    assert_eq!(first["maxResults"], 50);
    assert!(
        first["fields"]
            .as_array()
            .unwrap()
            .contains(&json!("customfield_10020"))
    );
    assert_eq!(second["nextPageToken"], "next");
    assert_eq!(second["maxResults"], 49);
    server.join();
}

#[test]
fn search_limit_reports_truncation_without_fetching_another_page() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"issues":[{"key":"RW-1","fields":{"summary":"First","status":{"name":"Open"}}}],"nextPageToken":"next","isLast":false}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "search",
        "--jql",
        "project = RW",
        "--limit",
        "1",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(result["issues"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
    assert_eq!(collect_requests(&requests, 1).len(), 1);
    server.join();
}

#[test]
fn search_rejects_repeated_cursor() {
    let page = r#"{"issues":[{"key":"RW-1","fields":{}}],"nextPageToken":"same","isLast":false}"#;
    let (server, _requests) =
        spawn_sequence_server(vec![("HTTP/1.1 200 OK", page), ("HTTP/1.1 200 OK", page)]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "search",
        "--jql",
        "project = RW",
        "--json",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("pagination"),
        "{}",
        stderr(&output)
    );
    server.join();
}

#[test]
fn boards_paginates_and_encodes_project_filter() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":1,"name":"Main","type":"scrum"}],"startAt":0,"maxResults":1,"total":2,"isLast":false}"#,
        ),
        (
            "HTTP/1.1 200 OK",
            r#"{"values":[{"id":2,"name":"Other","type":"kanban"}],"startAt":1,"maxResults":1,"total":2,"isLast":true}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "boards",
        "--project",
        "A & B",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(
        result["boards"][1],
        json!({"id":2,"name":"Other","type":"kanban"})
    );
    let captured = collect_requests(&requests, 2);
    assert!(captured[0].starts_with("GET /rest/agile/1.0/board?"));
    assert!(captured[0].contains("projectKeyOrId=A+%26+B"));
    assert!(captured[1].contains("startAt=1"));
    server.join();
}

#[test]
fn sprints_keeps_dates_and_accepts_multiple_states() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"values":[{"id":9,"name":"Sprint 9","state":"active","startDate":"2026-10-26","endDate":"2026-11-02"},{"id":10,"name":"Sprint 10","state":"future"}],"isLast":true}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "sprints",
        "--board",
        "42",
        "--state",
        "active,future",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(result["sprints"][0]["startDate"], "2026-10-26");
    assert_eq!(result["sprints"][1]["name"], "Sprint 10");
    let captured = collect_requests(&requests, 1);
    assert!(captured[0].starts_with("GET /rest/agile/1.0/board/42/sprint?"));
    assert!(captured[0].contains("state=active%2Cfuture"));
    server.join();
}

#[test]
fn invalid_discovery_arguments_fail_before_network_access() {
    for args in [
        vec!["search", "--jql", "project = RW", "--limit", "0"],
        vec!["sprints", "--board", "42", "--state", "invalid"],
    ] {
        let output = run_jit(args);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("invalid value"),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn search_caps_page_size_and_prints_compact_text() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"issues":[{"key":"RW-7","fields":{"summary":"Readable title","status":{"name":"Open"}}}],"isLast":true}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "search",
        "--jql",
        "project = RW",
        "--limit",
        "201",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("RW-7: Readable title [Open]"));
    let captured = collect_requests(&requests, 1);
    let body: Value = serde_json::from_str(request_body(&captured[0])).unwrap();
    assert_eq!(body["maxResults"], 100);
    server.join();
}

#[test]
fn search_rejects_incomplete_page_without_cursor() {
    let (server, _requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"issues":[{"key":"RW-1","fields":{}}],"isLast":false}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "search",
        "--jql",
        "project = RW",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("pagination"),
        "{}",
        stderr(&output)
    );
    server.join();
}

#[test]
fn discovery_rejects_empty_nonfinal_page() {
    let (server, _requests) =
        spawn_sequence_server(vec![("HTTP/1.1 200 OK", r#"{"values":[],"isLast":false}"#)]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit(["--config-file", config.path_str(), "boards", "--json"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("pagination"),
        "{}",
        stderr(&output)
    );
    server.join();
}

#[test]
fn discovery_rejects_repeated_page_without_offset_metadata() {
    let page = r#"{"values":[{"id":1,"name":"Main","type":"scrum"}],"isLast":false}"#;
    let (server, _requests) =
        spawn_sequence_server(vec![("HTTP/1.1 200 OK", page), ("HTTP/1.1 200 OK", page)]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit(["--config-file", config.path_str(), "boards", "--json"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("pagination"),
        "{}",
        stderr(&output)
    );
    server.join();
}

#[test]
fn search_uses_configured_sprint_field() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"issues":[{"key":"RW-1","fields":{"summary":"Custom sprint","customfield_12345":[{"name":"Configured sprint","state":"active"}]}}],"isLast":true}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let mut contents = std::fs::read_to_string(config.path_str()).unwrap();
    contents.push_str("sprint_field = \"customfield_12345\"\n");
    std::fs::write(config.path_str(), contents).unwrap();
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "search",
        "--jql",
        "project = RW",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(result["issues"][0]["sprint"], "Configured sprint");
    let captured = collect_requests(&requests, 1);
    let body: Value = serde_json::from_str(request_body(&captured[0])).unwrap();
    assert!(
        body["fields"]
            .as_array()
            .unwrap()
            .contains(&json!("customfield_12345"))
    );
    server.join();
}
