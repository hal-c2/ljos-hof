//! Talking to Hof over its Unix socket, as `hof mcp` does.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::FakeHost;
use hof_mcp::{Policy, bridge, serve_unix};
use serde_json::{Value, json};

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.sock");
    (dir, path)
}

fn host() -> Arc<FakeHost> {
    Arc::new(FakeHost::new())
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    fn connect(path: &PathBuf) -> Self {
        let stream = UnixStream::connect(path).unwrap();
        // A hung test should fail, not hang.
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Self {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
        }
    }

    fn send(&mut self, message: Value) {
        writeln!(self.writer, "{message}").unwrap();
    }

    fn receive(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    }
}

#[test]
fn an_agent_can_initialize_list_tools_and_call_one_over_the_socket() {
    let (_dir, path) = scratch();
    let host = host();
    let server = serve_unix(&path, host.clone(), Policy::ALLOW_ALL).unwrap();
    let mut client = Client::connect(&path);

    client.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "test", "version": "0" } }
    }));
    assert_eq!(client.receive()["result"]["serverInfo"]["name"], "hof");
    client.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    client.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let tools = client.receive();
    assert_eq!(tools["id"], 2, "the notification got no reply");
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 9);

    client.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "run_command", "arguments": { "pane": 1, "command": "echo hello" } }
    }));
    let reply = client.receive();
    assert_eq!(reply["id"], 3);
    assert_eq!(
        reply["result"]["content"][0]["text"],
        "exit code 0\nhello\n"
    );
    assert_eq!(host.calls(), [r#"run_command 1 "echo hello" 30s"#]);

    drop(client);
    server.shutdown();
}

#[test]
fn blank_lines_are_skipped_and_garbage_gets_a_parse_error() {
    let (_dir, path) = scratch();
    let _server = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    let mut client = Client::connect(&path);
    client.writer.write_all(b"\n\r\n\xff\xfe\n").unwrap();
    assert_eq!(client.receive()["error"]["code"], -32700);
    client.writer.write_all(b"{oops\n").unwrap();
    assert_eq!(client.receive()["error"]["code"], -32700);
    client.send(json!({ "jsonrpc": "2.0", "id": 9, "method": "ping" }));
    assert_eq!(
        client.receive()["id"],
        9,
        "the connection survives bad lines"
    );
}

#[test]
fn the_socket_is_private_to_the_user() {
    let (_dir, path) = scratch();
    let _server = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    let metadata = fs::metadata(&path).unwrap();
    assert!(metadata.file_type().is_socket());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
}

#[test]
fn dropping_the_server_removes_the_socket() {
    let (_dir, path) = scratch();
    let server = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    assert!(path.exists());
    drop(server);
    assert!(!path.exists());
    assert_eq!(
        fs::read_dir(path.parent().unwrap()).unwrap().count(),
        0,
        "nothing is left behind"
    );
}

#[test]
fn shutting_down_hangs_up_on_connected_clients() {
    let (_dir, path) = scratch();
    let server = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    let mut client = Client::connect(&path);
    client.send(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
    client.receive();
    server.shutdown();
    let mut rest = Vec::new();
    client.reader.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
}

#[test]
fn shutting_down_returns_even_when_the_socket_file_was_deleted() {
    let (_dir, path) = scratch();
    let server = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    fs::remove_file(&path).unwrap();
    server.shutdown();
}

#[test]
fn a_socket_left_by_a_dead_hof_is_replaced() {
    let (_dir, path) = scratch();
    drop(UnixListener::bind(&path).unwrap());
    assert!(path.exists(), "the dead listener left its file");
    let _server = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    let mut client = Client::connect(&path);
    client.send(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
    assert_eq!(client.receive()["result"], json!({}));
}

#[test]
fn a_socket_that_still_answers_is_left_alone() {
    let (_dir, path) = scratch();
    let first = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    let error = serve_unix(&path, host(), Policy::READ_ONLY)
        .err()
        .expect("the second server must not start");
    assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
    let mut client = Client::connect(&path);
    client.send(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
    assert_eq!(client.receive()["id"], 1, "the first server still serves");
    drop(first);
}

#[test]
fn a_file_that_is_not_a_socket_is_never_removed() {
    let (_dir, path) = scratch();
    fs::write(&path, "precious").unwrap();
    let error = serve_unix(&path, host(), Policy::READ_ONLY)
        .err()
        .expect("must not replace a file");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&path).unwrap(), "precious");
}

#[test]
fn the_stdio_bridge_carries_a_whole_session_and_ends_when_input_does() {
    let (_dir, path) = scratch();
    let host = host();
    let _server = serve_unix(&path, host.clone(), Policy::READ_ONLY).unwrap();
    let input = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "read_screen", "arguments": { "pane": 2 } } }),
    ]
    .map(|message| format!("{message}\n"))
    .concat();
    let mut output = Vec::new();
    bridge(&path, Cursor::new(input.into_bytes()), &mut output).unwrap();

    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        replies[1]["result"]["content"][0]["text"],
        "$ screen of pane 2"
    );
}

#[test]
fn the_bridge_says_hof_is_not_running_when_nothing_listens() {
    let (_dir, path) = scratch();
    let error = bridge(&path, Cursor::new(Vec::new()), Vec::new()).unwrap_err();
    assert!(error.to_string().contains("is Hof running"), "{error}");
}

#[test]
fn one_call_gets_the_tool_s_answer() {
    let (_dir, path) = scratch();
    let _server = serve_unix(&path, host(), Policy::READ_ONLY).unwrap();
    let screen = hof_mcp::call(&path, "read_screen", json!({ "pane": 2 })).unwrap();
    assert_eq!(
        screen,
        hof_mcp::Answer {
            text: "$ screen of pane 2".into(),
            is_error: false
        }
    );
    let refused = hof_mcp::call(&path, "send_input", json!({ "pane": 2, "text": "x" })).unwrap();
    assert!(refused.is_error, "{refused:?}");
}

#[test]
fn a_call_with_nothing_listening_is_an_error() {
    let (_dir, path) = scratch();
    assert!(hof_mcp::call(&path, "list_panes", json!({})).is_err());
}
