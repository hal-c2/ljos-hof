//! MCP over JSON-RPC 2.0, one message per line.

use serde_json::{Map, Value, json};

use crate::host::Host;
use crate::tools::{self, Policy, Tool};

/// The MCP revision this server is written against.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Revisions whose clients understand everything this server sends: it
/// only offers tools, and their shape has not changed since the first.
const SUPPORTED_VERSIONS: [&str; 3] = [PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Answer one line from a client: the response line to send back, without
/// its newline, or `None` when the line was a notification or a response.
///
/// Holds no state between calls, so any number of connections can share
/// one `host`.
pub fn handle(host: &dyn Host, policy: Policy, line: &str) -> Option<String> {
    let message = match serde_json::from_str::<Value>(line) {
        Ok(message) => message,
        Err(error) => return Some(parse_error(&format!("not JSON: {error}"))),
    };
    let Value::Object(message) = message else {
        // MCP dropped JSON-RPC batches in 2025-06-18.
        return Some(error_line(
            &Value::Null,
            RpcError::new(INVALID_REQUEST, "expected a JSON-RPC message object"),
        ));
    };
    let Some(method) = message.get("method") else {
        // The server sends no requests, so a client's response answers
        // nothing; replying to it would start a loop.
        if message.contains_key("result") || message.contains_key("error") {
            return None;
        }
        let id = valid_id(message.get("id")).unwrap_or(&Value::Null);
        return Some(error_line(
            id,
            RpcError::new(INVALID_REQUEST, "a request needs a method"),
        ));
    };
    let Some(id) = message.get("id") else {
        // Notifications get no reply, even ones this server does not know.
        return None;
    };
    let Some(id) = valid_id(Some(id)) else {
        return Some(error_line(
            &Value::Null,
            RpcError::new(INVALID_REQUEST, "id must be a string or a number"),
        ));
    };
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_line(
            id,
            RpcError::new(INVALID_REQUEST, "jsonrpc must be \"2.0\""),
        ));
    }
    let Some(method) = method.as_str() else {
        return Some(error_line(
            id,
            RpcError::new(INVALID_REQUEST, "method must be a string"),
        ));
    };
    let params = match message.get("params") {
        None => None,
        Some(Value::Object(params)) => Some(params),
        Some(_) => {
            return Some(error_line(
                id,
                RpcError::new(INVALID_PARAMS, "params must be an object"),
            ));
        }
    };
    let line = match dispatch(host, policy, method, params) {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Err(error) => error_line(id, error),
    };
    Some(line)
}

/// The reply to a line that could not even be read as text.
pub(crate) fn parse_error(detail: &str) -> String {
    error_line(&Value::Null, RpcError::new(PARSE_ERROR, detail))
}

fn valid_id(id: Option<&Value>) -> Option<&Value> {
    id.filter(|id| id.is_string() || id.is_number())
}

fn error_line(id: &Value, error: RpcError) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": error.code, "message": error.message }
    })
    .to_string()
}

fn dispatch(
    host: &dyn Host,
    policy: Policy,
    method: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions(policy) })),
        "tools/call" => call_tool(host, policy, params),
        _ => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("no method {method:?}"),
        )),
    }
}

fn initialize(params: Option<&Map<String, Value>>) -> Value {
    let asked = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = asked
        .filter(|asked| SUPPORTED_VERSIONS.contains(asked))
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "hof",
            "title": "Ljós Hof",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Hof is the terminal emulator the user is working in. Call list_panes \
            to see its panes, read_screen or read_scrollback to see what is in them, and \
            run_command to run a shell command in one and get its output."
    })
}

fn call_tool(
    host: &dyn Host,
    policy: Policy,
    params: Option<&Map<String, Value>>,
) -> Result<Value, RpcError> {
    let params = params.ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call needs params"))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call needs a tool name"))?;
    let tool = Tool::from_name(name)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("no tool {name:?}")))?;
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(_) => return Err(RpcError::new(INVALID_PARAMS, "arguments must be an object")),
    };
    let outcome = tools::call(host, policy, tool, arguments);
    Ok(json!({
        "content": [{ "type": "text", "text": outcome.text }],
        "isError": outcome.is_error
    }))
}
