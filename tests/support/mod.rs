#![allow(dead_code)]
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
pub struct TempConfig {
    dir: PathBuf,
    path: PathBuf,
}

impl TempConfig {
    pub fn empty() -> Self {
        let unique = TEMP_CONFIG_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jit-cli-e2e-{}-{unique}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp config directory should be created");
        let path = dir.join("config.toml");

        Self { dir, path }
    }

    pub fn new(base_url: &str) -> Self {
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

    pub fn path_str(&self) -> &str {
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

pub struct TestServer {
    pub base_url: String,
    handle: thread::JoinHandle<()>,
}

impl TestServer {
    pub fn join(self) {
        self.handle.join().expect("server thread should finish");
    }
}

pub fn run_jit<'a>(args: impl IntoIterator<Item = &'a str>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("jit command should run")
}

pub fn run_jit_with_stdin<'a>(args: impl IntoIterator<Item = &'a str>, stdin: &str) -> Output {
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

pub fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8")
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be utf-8")
}

pub fn request_body(request: &str) -> &str {
    request
        .split("\r\n\r\n")
        .nth(1)
        .expect("http request should contain a body")
}

pub fn collect_requests(receiver: &mpsc::Receiver<String>, expected: usize) -> Vec<String> {
    (0..expected)
        .map(|_| {
            receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("request should be captured")
        })
        .collect()
}

pub fn spawn_sequence_server(
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

pub fn read_http_request(stream: &mut TcpStream) -> String {
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

pub fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

pub fn parse_content_length(headers: &[u8]) -> usize {
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
