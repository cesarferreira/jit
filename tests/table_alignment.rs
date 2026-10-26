mod support;
use regex::Regex;
use support::*;
use unicode_width::UnicodeWidthStr;

#[test]
fn sprint_table_aligns_emoji_wide_and_combining_text_with_colored_statuses() {
    let (server, requests) = spawn_sequence_server(vec![(
        "HTTP/1.1 200 OK",
        r#"{"issues":[
        {"id":"1","key":"OBX-3244","fields":{"summary":"Replace stale robotaxi route when a new job reuses task with a long summary","status":{"name":"Waiting for review"}}},
        {"id":"2","key":"OBX-3241","fields":{"summary":"🌵[android] black box testing for android auto","status":{"name":"Backlog"}}},
        {"id":"3","key":"OBX-3114","fields":{"summary":"[Android] 🌵 Find a replacement for the Predicted Speed with another long summary","status":{"name":"In progress"}}},
        {"id":"4","key":"OBX-3242","fields":{"summary":"测试自动驾驶界面","status":{"name":"検証中"}}},
        {"id":"5","key":"OBX-3243","fields":{"summary":"Café 👩‍💻 robotaxi testing","status":{"name":"Done"}}}
    ],"isLast":true}"#,
    )]);
    let config = TempConfig::new(&server.base_url);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(["--config-file", config.path_str(), "--my-tickets"])
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let raw = stdout(&output);
    assert!(raw.contains("\x1b["), "Statuses should retain ANSI color");
    assert!(raw.contains("🌵[android]"));
    assert!(raw.contains("Café 👩‍💻"));
    let plain = Regex::new(r"\x1b\[[0-9;]*m").unwrap().replace_all(&raw, "");
    let mut rows = 0;
    for line in plain.lines().filter(|line| line.starts_with('│')) {
        let offsets: Vec<_> = line
            .char_indices()
            .filter(|(_, ch)| *ch == '│')
            .map(|(index, _)| UnicodeWidthStr::width(&line[..index]))
            .collect();
        assert_eq!(offsets, [0, 21, 82, 103], "Misaligned row: {line}");
        rows += 1;
    }
    assert_eq!(rows, 6);
    collect_requests(&requests, 1);
    server.join();
}
