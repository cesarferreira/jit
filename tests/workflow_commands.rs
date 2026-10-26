mod support;
use serde_json::{Value, json};
use support::*;

const TRANSITIONS: &str = r#"{"transitions":[{"id":"21","name":"Start Work","to":{"name":"In Progress"}},{"id":"31","name":"Done","to":{"name":"Done"}}]}"#;
const LINK_TYPES: &str = r#"{"issueLinkTypes":[{"id":"9001","name":"Dependency","inward":"is blocked by","outward":"blocks"},{"id":"9002","name":"Relates","inward":"relates to","outward":"relates to"}]}"#;

#[test]
fn transitions_lists_available_actions_as_json() {
    let (server, requests) = spawn_sequence_server(vec![("HTTP/1.1 200 OK", TRANSITIONS)]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "transitions",
        "RW-123",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let value: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["transitions"][0]["id"], "21");
    assert!(
        collect_requests(&requests, 1)[0].starts_with("GET /rest/api/3/issue/RW-123/transitions")
    );
    server.join();
}

#[test]
fn transition_resolves_case_insensitive_action_name_and_posts_id() {
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 200 OK", TRANSITIONS),
        ("HTTP/1.1 204 No Content", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "transition",
        "https://example.atlassian.net/browse/RW-123",
        "--to",
        "start work",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let captured = collect_requests(&requests, 2);
    assert!(captured[1].starts_with("POST /rest/api/3/issue/RW-123/transitions"));
    assert_eq!(
        serde_json::from_str::<Value>(request_body(&captured[1])).unwrap(),
        json!({"transition":{"id":"21"}})
    );
    let value: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["ticket"], "RW-123");
    server.join();
}

#[test]
fn transition_accepts_id_and_rejects_unknown_or_ambiguous_names_without_writing() {
    for (selection, response, expected) in [
        ("missing", TRANSITIONS, "Available"),
        (
            "done",
            r#"{"transitions":[{"id":"31","name":"Done"},{"id":"41","name":"DONE"}]}"#,
            "ambiguous",
        ),
    ] {
        let (server, requests) = spawn_sequence_server(vec![("HTTP/1.1 200 OK", response)]);
        let config = TempConfig::new(&server.base_url);
        let output = run_jit([
            "--config-file",
            config.path_str(),
            "transition",
            "RW-123",
            "--to",
            selection,
        ]);
        assert!(!output.status.success());
        assert!(stderr(&output).contains(expected), "{}", stderr(&output));
        assert_eq!(collect_requests(&requests, 1).len(), 1);
        server.join();
    }
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 200 OK", TRANSITIONS),
        ("HTTP/1.1 204 No Content", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "transition",
        "RW-123",
        "--to",
        "31",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let captured = collect_requests(&requests, 2);
    assert_eq!(
        serde_json::from_str::<Value>(request_body(&captured[1])).unwrap()["transition"]["id"],
        "31"
    );
    server.join();
}

#[test]
fn comment_posts_adf_and_returns_created_comment_id() {
    let (server, requests) =
        spawn_sequence_server(vec![("HTTP/1.1 201 Created", r#"{"id":"12345"}"#)]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "comment",
        "RW-123",
        "--body",
        "First line\nSecond line",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let captured = collect_requests(&requests, 1);
    assert!(captured[0].starts_with("POST /rest/api/3/issue/RW-123/comment"));
    let body: Value = serde_json::from_str(request_body(&captured[0])).unwrap();
    assert_eq!(body["body"]["type"], "doc");
    assert_eq!(
        body["body"]["content"][0]["content"][0]["text"],
        "First line"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["id"],
        "12345"
    );
    server.join();
}

#[test]
fn comment_requires_nonempty_body_before_network() {
    let config = TempConfig::new("http://127.0.0.1:1");
    for extra in [vec![], vec!["--body", "   "]] {
        let mut args = vec!["--config-file", config.path_str(), "comment", "RW-123"];
        args.extend(extra);
        let output = run_jit(args);
        assert!(!output.status.success());
        assert!(
            stderr(&output).to_lowercase().contains("body"),
            "{}",
            stderr(&output)
        );
        assert!(!stderr(&output).contains("error sending request"));
    }
}

#[test]
fn blocks_discovers_custom_link_type_and_correct_direction() {
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 200 OK", LINK_TYPES),
        ("HTTP/1.1 201 Created", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "link",
        "RW-123",
        "--blocks",
        "RW-456",
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let captured = collect_requests(&requests, 2);
    assert!(captured[0].starts_with("GET /rest/api/3/issueLinkType"));
    assert!(captured[1].starts_with("POST /rest/api/3/issueLink "));
    let body: Value = serde_json::from_str(request_body(&captured[1])).unwrap();
    assert_eq!(
        body,
        json!({"type":{"name":"Dependency"},"inwardIssue":{"key":"RW-123"},"outwardIssue":{"key":"RW-456"}})
    );
    server.join();
}

#[test]
fn generic_link_resolves_type_name_and_id() {
    for kind in ["relates", "9002"] {
        let (server, requests) = spawn_sequence_server(vec![
            ("HTTP/1.1 200 OK", LINK_TYPES),
            ("HTTP/1.1 201 Created", ""),
        ]);
        let config = TempConfig::new(&server.base_url);
        let output = run_jit([
            "--config-file",
            config.path_str(),
            "link",
            "RW-123",
            "--to",
            "RW-456",
            "--type",
            kind,
        ]);
        assert!(output.status.success(), "{}", stderr(&output));
        let captured = collect_requests(&requests, 2);
        assert_eq!(
            serde_json::from_str::<Value>(request_body(&captured[1])).unwrap()["type"]["name"],
            "Relates"
        );
        server.join();
    }
}

#[test]
fn link_rejects_invalid_modes_before_network() {
    let config = TempConfig::new("http://127.0.0.1:1");
    for extra in [
        vec![],
        vec!["--to", "RW-456"],
        vec!["--type", "Relates"],
        vec!["--blocks", "RW-456", "--to", "RW-789", "--type", "Relates"],
    ] {
        let mut args = vec!["--config-file", config.path_str(), "link", "RW-123"];
        args.extend(extra);
        let output = run_jit(args);
        assert!(!output.status.success());
        assert!(!stderr(&output).contains("error sending request"));
    }
}

#[test]
fn comment_reads_file_or_stdin_and_rejects_conflicting_sources() {
    let file =
        std::env::temp_dir().join(format!("jit-workflow-comment-{}.txt", std::process::id()));
    std::fs::write(&file, "From file").unwrap();
    for source in [file.to_str().unwrap(), "-"] {
        let (server, requests) =
            spawn_sequence_server(vec![("HTTP/1.1 201 Created", r#"{"id":"123"}"#)]);
        let config = TempConfig::new(&server.base_url);
        let output = run_jit_with_stdin(
            [
                "--config-file",
                config.path_str(),
                "comment",
                "RW-123",
                "--body-file",
                source,
            ],
            "From stdin",
        );
        assert!(output.status.success(), "{}", stderr(&output));
        let captured = collect_requests(&requests, 1);
        let body: Value = serde_json::from_str(request_body(&captured[0])).unwrap();
        assert_eq!(
            body["body"]["content"][0]["content"][0]["text"],
            if source == "-" {
                "From stdin"
            } else {
                "From file"
            }
        );
        server.join();
    }
    std::fs::remove_file(file).unwrap();
    let config = TempConfig::new("http://127.0.0.1:1");
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "comment",
        "RW-123",
        "--body",
        "text",
        "--body-file",
        "-",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot be used with"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn blocks_respects_reversed_instance_direction() {
    let (server, requests) = spawn_sequence_server(vec![
        (
            "HTTP/1.1 200 OK",
            r#"{"issueLinkTypes":[{"id":"9001","name":"Dependency","inward":"blocks","outward":"is blocked by"}]}"#,
        ),
        ("HTTP/1.1 201 Created", ""),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "link",
        "RW-123",
        "--blocks",
        "RW-456",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let captured = collect_requests(&requests, 2);
    let body: Value = serde_json::from_str(request_body(&captured[1])).unwrap();
    assert_eq!(body["inwardIssue"]["key"], "RW-456");
    assert_eq!(body["outwardIssue"]["key"], "RW-123");
    server.join();
}

#[test]
fn unknown_link_type_is_actionable_and_does_not_write() {
    let (server, requests) = spawn_sequence_server(vec![("HTTP/1.1 200 OK", LINK_TYPES)]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "link",
        "RW-123",
        "--to",
        "RW-456",
        "--type",
        "Missing",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Available link types"),
        "{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("Relates"));
    assert_eq!(collect_requests(&requests, 1).len(), 1);
    server.join();
}

#[test]
fn transition_api_validation_error_reaches_user() {
    let (server, requests) = spawn_sequence_server(vec![
        ("HTTP/1.1 200 OK", TRANSITIONS),
        (
            "HTTP/1.1 400 Bad Request",
            r#"{"errors":{"resolution":"Resolution is required."}}"#,
        ),
    ]);
    let config = TempConfig::new(&server.base_url);
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "transition",
        "RW-123",
        "--to",
        "31",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Resolution is required"),
        "{}",
        stderr(&output)
    );
    assert_eq!(collect_requests(&requests, 2).len(), 2);
    server.join();
}
