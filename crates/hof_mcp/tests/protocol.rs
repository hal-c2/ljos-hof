//! What an MCP client sees when it talks to Hof.

mod common;

use common::FakeHost;
use hof_mcp::{CommandResult, PROTOCOL_VERSION, Policy, handle};
use serde_json::{Value, json};

fn ask(host: &FakeHost, policy: Policy, request: Value) -> Value {
    let reply = handle(host, policy, &request.to_string()).expect("a request gets a reply");
    assert!(!reply.contains('\n'), "a reply is one line: {reply}");
    serde_json::from_str(&reply).unwrap()
}

fn request(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

/// Call a tool with every permission, returning its text and error flag.
fn call(host: &FakeHost, name: &str, arguments: Value) -> (String, bool) {
    call_with(host, Policy::ALLOW_ALL, name, arguments)
}

fn call_with(host: &FakeHost, policy: Policy, name: &str, arguments: Value) -> (String, bool) {
    let reply = ask(
        host,
        policy,
        request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        ),
    );
    let result = &reply["result"];
    assert_eq!(result["content"][0]["type"], "text", "{reply}");
    (
        result["content"][0]["text"].as_str().unwrap().to_owned(),
        result["isError"].as_bool().unwrap(),
    )
}

fn error_code(reply: &Value) -> i64 {
    reply["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("{reply}"))
}

fn tool_names(host: &FakeHost, policy: Policy) -> Vec<String> {
    let reply = ask(host, policy, request("tools/list", json!({})));
    reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn initialize_announces_hof_with_tools_and_the_current_protocol() {
    let host = FakeHost::new();
    let reply = ask(
        &host,
        Policy::READ_ONLY,
        request(
            "initialize",
            json!({
                "protocolVersion": "2099-01-01",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1" }
            }),
        ),
    );
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], 1);
    let result = &reply["result"];
    assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(PROTOCOL_VERSION, "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "hof");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(result["capabilities"], json!({ "tools": {} }));
}

#[test]
fn initialize_keeps_an_older_protocol_the_client_asks_for() {
    let host = FakeHost::new();
    let reply = ask(
        &host,
        Policy::READ_ONLY,
        request("initialize", json!({ "protocolVersion": "2025-03-26" })),
    );
    assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
}

#[test]
fn ping_gets_an_empty_result_and_keeps_a_string_id() {
    let host = FakeHost::new();
    let reply = ask(
        &host,
        Policy::READ_ONLY,
        json!({ "jsonrpc": "2.0", "id": "abc", "method": "ping" }),
    );
    assert_eq!(
        reply,
        json!({ "jsonrpc": "2.0", "id": "abc", "result": {} })
    );
}

#[test]
fn notifications_get_no_reply_even_unknown_ones() {
    let host = FakeHost::new();
    for method in [
        "notifications/initialized",
        "notifications/cancelled",
        "made/up",
    ] {
        let line = json!({ "jsonrpc": "2.0", "method": method }).to_string();
        assert_eq!(handle(&host, Policy::ALLOW_ALL, &line), None, "{method}");
    }
}

#[test]
fn a_response_from_the_client_gets_no_reply() {
    let host = FakeHost::new();
    let line = json!({ "jsonrpc": "2.0", "id": 5, "result": {} }).to_string();
    assert_eq!(handle(&host, Policy::ALLOW_ALL, &line), None);
}

#[test]
fn every_tool_is_listed_with_an_object_input_schema_when_everything_is_allowed() {
    let host = FakeHost::new();
    let reply = ask(&host, Policy::ALLOW_ALL, request("tools/list", json!({})));
    let tools = reply["result"]["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "close_pane",
            "focus_pane",
            "list_panes",
            "new_tab",
            "read_screen",
            "read_scrollback",
            "run_command",
            "send_input",
            "split_pane"
        ]
    );
    for tool in tools {
        assert!(tool["description"].as_str().unwrap().len() > 20, "{tool}");
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object", "{tool}");
        for required in schema["required"].as_array().unwrap() {
            let required = required.as_str().unwrap();
            assert!(
                schema["properties"].get(required).is_some(),
                "{} requires {required} but does not describe it",
                tool["name"]
            );
        }
    }
}

#[test]
fn a_read_only_policy_lists_only_the_tools_that_look() {
    let host = FakeHost::new();
    let mut names = tool_names(&host, Policy::READ_ONLY);
    names.sort();
    assert_eq!(
        names,
        ["focus_pane", "list_panes", "read_screen", "read_scrollback"]
    );
}

#[test]
fn input_and_spawning_are_allowed_separately() {
    let host = FakeHost::new();
    let input_only = tool_names(
        &host,
        Policy {
            allow_input: true,
            allow_spawn: false,
        },
    );
    assert!(input_only.contains(&"send_input".to_owned()));
    assert!(input_only.contains(&"run_command".to_owned()));
    assert!(!input_only.contains(&"new_tab".to_owned()));

    let spawn_only = tool_names(
        &host,
        Policy {
            allow_input: false,
            allow_spawn: true,
        },
    );
    assert!(spawn_only.contains(&"new_tab".to_owned()));
    assert!(spawn_only.contains(&"split_pane".to_owned()));
    assert!(!spawn_only.contains(&"send_input".to_owned()));
}

#[test]
fn a_tool_the_policy_forbids_is_a_tool_error_and_never_reaches_the_host() {
    let host = FakeHost::new();
    let (text, is_error) = call_with(
        &host,
        Policy::READ_ONLY,
        "send_input",
        json!({ "pane": 1, "text": "rm -rf ~\r" }),
    );
    assert!(is_error);
    assert!(text.contains("turned off"), "{text}");
    let (_, is_error) = call_with(&host, Policy::READ_ONLY, "new_tab", json!({}));
    assert!(is_error);
    assert!(host.calls().is_empty(), "{:?}", host.calls());
}

#[test]
fn list_panes_returns_the_panes_as_json() {
    let host = FakeHost::new();
    let (text, is_error) = call(&host, "list_panes", json!({}));
    assert!(!is_error);
    let panes: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(panes[0]["id"], 1);
    assert_eq!(panes[0]["focused"], true);
    assert_eq!(panes[0]["working_directory"], "/home/user");
    assert_eq!(panes[1]["running"], true);
    assert_eq!(panes[1]["columns"], 80);
}

#[test]
fn list_panes_works_without_any_arguments() {
    let host = FakeHost::new();
    let reply = ask(
        &host,
        Policy::READ_ONLY,
        request("tools/call", json!({ "name": "list_panes" })),
    );
    assert_eq!(reply["result"]["isError"], false);
}

#[test]
fn read_screen_returns_the_pane_text() {
    let host = FakeHost::new();
    let (text, is_error) = call(&host, "read_screen", json!({ "pane": 2 }));
    assert!(!is_error);
    assert_eq!(text, "$ screen of pane 2");
}

#[test]
fn read_scrollback_passes_the_line_count_on() {
    let host = FakeHost::new();
    call(&host, "read_scrollback", json!({ "pane": 1, "lines": 50 }));
    call(&host, "read_scrollback", json!({ "pane": 1 }));
    assert_eq!(
        host.calls(),
        ["read_scrollback 1 Some(50)", "read_scrollback 1 None"]
    );
}

#[test]
fn send_input_types_the_text_and_submit_adds_enter() {
    let host = FakeHost::new();
    let (text, is_error) = call(&host, "send_input", json!({ "pane": 1, "text": "ls" }));
    assert!(!is_error);
    assert!(text.contains("pane 1"), "{text}");
    call(
        &host,
        "send_input",
        json!({ "pane": 1, "text": "ls", "submit": true }),
    );
    assert_eq!(
        host.calls(),
        [r#"send_input 1 "ls""#, r#"send_input 1 "ls\r""#]
    );
}

#[test]
fn new_tab_returns_the_new_pane_id() {
    let host = FakeHost::new();
    let (text, is_error) = call(
        &host,
        "new_tab",
        json!({ "directory": "/src", "command": ["htop"] }),
    );
    assert!(!is_error);
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({ "pane": 10 })
    );
    assert_eq!(host.calls(), [r#"new_tab Some("/src") ["htop"]"#]);
}

#[test]
fn split_pane_splits_in_the_asked_direction() {
    let host = FakeHost::new();
    let (text, is_error) = call(
        &host,
        "split_pane",
        json!({ "pane": 1, "direction": "down" }),
    );
    assert!(!is_error);
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({ "pane": 11 })
    );
    assert_eq!(host.calls(), ["split 1 Down []"]);
}

#[test]
fn a_new_pane_can_be_put_on_another_machine() {
    let host = FakeHost::new();
    call(&host, "new_tab", json!({ "host": "dev" }));
    call(
        &host,
        "split_pane",
        json!({ "pane": 1, "direction": "right", "host": "" }),
    );
    assert_eq!(
        host.calls(),
        [r#"new_tab None [] on "dev""#, "split 1 Right [] on \"\""]
    );
}

#[test]
fn split_pane_rejects_a_direction_it_does_not_know() {
    let host = FakeHost::new();
    let (text, is_error) = call(
        &host,
        "split_pane",
        json!({ "pane": 1, "direction": "diagonal" }),
    );
    assert!(is_error);
    assert!(text.contains("diagonal"), "{text}");
}

#[test]
fn focus_and_close_act_on_the_pane() {
    let host = FakeHost::new();
    assert_eq!(
        call(&host, "focus_pane", json!({ "pane": 2 })),
        ("focused pane 2".to_owned(), false)
    );
    assert_eq!(
        call(&host, "close_pane", json!({ "pane": 2 })),
        ("closed pane 2".to_owned(), false)
    );
    assert_eq!(host.calls(), ["focus_pane 2", "close_pane 2"]);
}

#[test]
fn run_command_reports_the_exit_code_and_output() {
    let host = FakeHost::new();
    let (text, is_error) = call(
        &host,
        "run_command",
        json!({ "pane": 1, "command": "echo hello", "timeout_seconds": 5 }),
    );
    assert!(!is_error);
    assert_eq!(text, "exit code 0\nhello\n");
    assert_eq!(host.calls(), [r#"run_command 1 "echo hello" 5s"#]);
}

#[test]
fn run_command_waits_thirty_seconds_unless_told_otherwise() {
    let host = FakeHost::new();
    call(
        &host,
        "run_command",
        json!({ "pane": 1, "command": "make" }),
    );
    assert_eq!(host.calls(), [r#"run_command 1 "make" 30s"#]);
}

#[test]
fn run_command_says_when_the_command_is_still_running() {
    let mut host = FakeHost::new();
    host.command_result = CommandResult {
        exit_code: None,
        output: "compiling".into(),
        timed_out: true,
    };
    let (text, is_error) = call(
        &host,
        "run_command",
        json!({ "pane": 1, "command": "make", "timeout_seconds": 2 }),
    );
    assert!(!is_error);
    assert!(text.starts_with("still running after 2s"), "{text}");
    assert!(text.ends_with("compiling"), "{text}");
}

#[test]
fn run_command_refuses_more_than_one_line_or_a_silly_timeout() {
    let host = FakeHost::new();
    for arguments in [
        json!({ "pane": 1, "command": "ls\nrm -rf /" }),
        json!({ "pane": 1, "command": "  " }),
        json!({ "pane": 1, "command": "ls", "timeout_seconds": 0 }),
        json!({ "pane": 1, "command": "ls", "timeout_seconds": 1e9 }),
    ] {
        let (_, is_error) = call(&host, "run_command", arguments.clone());
        assert!(is_error, "{arguments}");
    }
    assert!(host.calls().is_empty());
}

#[test]
fn a_host_failure_is_a_tool_error_with_its_message() {
    let host = FakeHost::new();
    let (text, is_error) = call(&host, "read_screen", json!({ "pane": 99 }));
    assert!(is_error);
    assert!(text.contains("no pane 99"), "{text}");
}

#[test]
fn missing_or_mistyped_arguments_are_a_tool_error_the_agent_can_read() {
    let host = FakeHost::new();
    let (text, is_error) = call(&host, "read_screen", json!({}));
    assert!(is_error);
    assert!(text.contains("pane"), "{text}");
    let (_, is_error) = call(&host, "read_screen", json!({ "pane": "one" }));
    assert!(is_error);
}

#[test]
fn an_unknown_method_is_method_not_found() {
    let host = FakeHost::new();
    let reply = ask(
        &host,
        Policy::ALLOW_ALL,
        request("resources/list", json!({})),
    );
    assert_eq!(error_code(&reply), -32601);
    assert_eq!(reply["id"], 1);
}

#[test]
fn an_unknown_tool_or_malformed_call_is_invalid_params() {
    let host = FakeHost::new();
    for params in [
        json!({ "name": "format_disk" }),
        json!({}),
        json!({ "name": "read_screen", "arguments": [1] }),
    ] {
        let reply = ask(
            &host,
            Policy::ALLOW_ALL,
            request("tools/call", params.clone()),
        );
        assert_eq!(error_code(&reply), -32602, "{params}");
    }
    let reply = ask(
        &host,
        Policy::ALLOW_ALL,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call" }),
    );
    assert_eq!(error_code(&reply), -32602);
    let reply = ask(&host, Policy::ALLOW_ALL, request("ping", json!([1, 2])));
    assert_eq!(error_code(&reply), -32602);
}

#[test]
fn a_line_that_is_not_json_is_a_parse_error_with_a_null_id() {
    let host = FakeHost::new();
    let reply = handle(&host, Policy::ALLOW_ALL, "{\"jsonrpc\": \"2.0\", ").unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(error_code(&reply), -32700);
    assert_eq!(reply["id"], Value::Null);
}

#[test]
fn a_message_that_is_not_a_request_object_is_an_invalid_request() {
    let host = FakeHost::new();
    for message in [
        json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]),
        json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": 7 }),
        json!({ "jsonrpc": "2.0", "id": {}, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 1 }),
    ] {
        let reply = ask(&host, Policy::ALLOW_ALL, message.clone());
        assert_eq!(error_code(&reply), -32600, "{message}");
    }
}
