//! One tool call over the socket, for `hof ctl` and scripts: the same tools
//! an agent has, without an MCP client.

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use serde_json::{Value, json};

/// What a tool answered: its text, and whether it is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub is_error: bool,
}

/// Calls the tool `name` with `arguments` on the server at `socket`.
///
/// `Err` when nothing answers there or the reply is not a tool's; a tool
/// that fails is an `Ok` [`Answer`] with `is_error` set.
pub fn call(socket: impl AsRef<Path>, name: &str, arguments: Value) -> io::Result<Answer> {
    let mut stream = crate::unix::connect(socket.as_ref())?;
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    stream.write_all(format!("{request}\n").as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    answer(&line)
}

fn answer(line: &str) -> io::Result<Answer> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let reply: Value = serde_json::from_str(line).map_err(|e| bad(&e.to_string()))?;
    if let Some(message) = reply["error"]["message"].as_str() {
        return Err(bad(message));
    }
    let result = &reply["result"];
    let text = result["content"][0]["text"]
        .as_str()
        .ok_or_else(|| bad("the reply has no text"))?;
    Ok(Answer {
        text: text.to_owned(),
        is_error: result["isError"].as_bool().unwrap_or(false),
    })
}
