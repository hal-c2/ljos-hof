//! `[mcp] enabled`: the window serves agents on its socket, and `hof mcp`'s
//! bridge reaches it. One test, as the config is read once a process.

use std::io::Cursor;
use std::thread;
use std::time::{Duration, Instant};

use ljos::testing::TestWindow;
use serde_json::{Value as Json, json};

#[test]
fn an_agent_reaches_the_window_through_the_socket_the_config_names() {
    // Not under the target directory: a socket's path has to be short.
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("hof-mcp.sock");
    let config = dir.path().join("hof.toml");
    std::fs::write(
        &config,
        format!(
            "[mcp]\nenabled = true\nsocket = {:?}\n",
            socket.display().to_string()
        ),
    )
    .unwrap();
    hof::set_launch(hof::Launch {
        command: vec!["cat".to_owned()],
        config: Some(config),
        ..hof::Launch::default()
    });
    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    assert!(socket.exists(), "the window listens");

    let input = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "list_panes" } }),
        // Read-only unless the config says otherwise: typing is refused.
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "send_input",
                            "arguments": { "pane": 1, "text": "hello" } } }),
    ]
    .map(|message| format!("{message}\n"))
    .concat();
    let agent = {
        let socket = socket.clone();
        thread::spawn(move || {
            let mut output = Vec::new();
            hof_mcp::bridge(&socket, Cursor::new(input.into_bytes()), &mut output).unwrap();
            output
        })
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while !agent.is_finished() {
        assert!(Instant::now() < deadline, "the agent got no answer");
        window.advance(16).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    let replies: Vec<Json> = String::from_utf8(agent.join().unwrap())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 3, "{replies:?}");

    let panes: Json =
        serde_json::from_str(replies[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(panes.as_array().map(Vec::len), Some(1), "{panes}");
    assert_eq!(panes[0]["id"], 1);
    assert_eq!(panes[0]["focused"], true);

    assert_eq!(replies[2]["result"]["isError"], true, "{}", replies[2]);
}
