use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn default_query_without_ticket_lists_current_sprint_table() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"issues":[{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation","status":{"name":"In Progress"},"customfield_10020":[{"name":"Sprint 42","state":"active"}]}}]}"#,
    )]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit([
        "--config-file",
        config.path_str(),
    ]);

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
fn default_ticket_query_prints_summary_lines() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"id":"10001","key":"RW-123","fields":{"summary":"Implement backlog creation"}}"#,
    )]);
    let config = TempConfig::new(&server.base_url);

    let output = run_jit([
        "--config-file",
        config.path_str(),
        "RW-123",
    ]);

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

    let output = run_jit([
        "--config-file",
        config.path_str(),
        "--text",
        "RW-123",
    ]);

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
fn edit_json_updates_requested_fields() {
    let (server, requests) =
        spawn_sequence_server(vec![("HTTP/1.1 204 No Content", "")]);
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

#[test]
fn edit_without_fields_fails_before_network_call() {
    let config = TempConfig::new("http://127.0.0.1:9");
    let output = run_jit([
        "--config-file",
        config.path_str(),
        "edit",
        "RW-123",
    ]);

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
    fn new(base_url: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("jit-cli-e2e-{unique}"));
        fs::create_dir_all(&dir).expect("temp config directory should be created");

        let path = dir.join("config.toml");
        fs::write(
            &path,
            format!(
                "[jira]\nbase_url = \"{base_url}\"\napi_token = \"token-123\"\nuser_email = \"user@example.com\"\n"
            ),
        )
        .expect("temp config file should be written");

        Self { dir, path }
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
