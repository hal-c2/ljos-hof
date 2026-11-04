//! The tools an agent can call, and what each one asks of the [`Host`].

use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::host::{AgentState, Direction, Host, HostError, PaneId};

/// What agents may do beyond looking.
///
/// Reading panes, listing them and moving focus are always allowed. A tool
/// that is not allowed is left out of `tools/list`, and calling it anyway
/// is a tool error, so an agent learns why rather than seeing it fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Policy {
    /// Act on panes that exist: `send_input`, `run_command`, `close_pane`.
    pub allow_input: bool,
    /// Start processes in new panes: `new_tab`, `split_pane`.
    pub allow_spawn: bool,
}

impl Policy {
    /// Look, never touch. The default.
    pub const READ_ONLY: Self = Self {
        allow_input: false,
        allow_spawn: false,
    };
    pub const ALLOW_ALL: Self = Self {
        allow_input: true,
        allow_spawn: true,
    };
}

/// How long `run_command` waits when the agent does not say.
const DEFAULT_TIMEOUT_SECONDS: f64 = 30.0;
/// Past this a forgotten wait would tie up a connection for good.
const MAX_TIMEOUT_SECONDS: f64 = 3600.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    ListPanes,
    ReadScreen,
    ReadScrollback,
    SendInput,
    NewTab,
    SplitPane,
    ClosePane,
    FocusPane,
    RunCommand,
    SetAgentState,
    WaitForAgent,
}

impl Tool {
    const ALL: [Tool; 11] = [
        Tool::ListPanes,
        Tool::ReadScreen,
        Tool::ReadScrollback,
        Tool::SendInput,
        Tool::RunCommand,
        Tool::NewTab,
        Tool::SplitPane,
        Tool::FocusPane,
        Tool::ClosePane,
        Tool::SetAgentState,
        Tool::WaitForAgent,
    ];

    fn name(self) -> &'static str {
        match self {
            Tool::ListPanes => "list_panes",
            Tool::ReadScreen => "read_screen",
            Tool::ReadScrollback => "read_scrollback",
            Tool::SendInput => "send_input",
            Tool::NewTab => "new_tab",
            Tool::SplitPane => "split_pane",
            Tool::ClosePane => "close_pane",
            Tool::FocusPane => "focus_pane",
            Tool::RunCommand => "run_command",
            Tool::SetAgentState => "set_agent_state",
            Tool::WaitForAgent => "wait_for_agent",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    fn allowed(self, policy: Policy) -> bool {
        match self {
            // Saying how an agent is changes what Hof shows, never what
            // runs; waiting is looking.
            Tool::ListPanes
            | Tool::ReadScreen
            | Tool::ReadScrollback
            | Tool::FocusPane
            | Tool::SetAgentState
            | Tool::WaitForAgent => true,
            Tool::SendInput | Tool::RunCommand | Tool::ClosePane => policy.allow_input,
            Tool::NewTab | Tool::SplitPane => policy.allow_spawn,
        }
    }

    fn refusal(self) -> String {
        let what = match self {
            Tool::NewTab | Tool::SplitPane => "open panes",
            _ => "type into or close panes",
        };
        format!(
            "{} is turned off: Hof is not letting agents {what}. The user can allow it in Hof's \
             settings.",
            self.name()
        )
    }

    fn definition(self) -> Value {
        let (title, description, properties, required, read_only, destructive) = match self {
            Tool::ListPanes => (
                "List panes",
                "List every pane open in Hof: its id, tab, title, working directory, whether it \
                 has focus, whether a command is running in it, its size in cells, and the AI \
                 agent running in it, if any, with its state: working, waiting (for the user), \
                 idle or none. Every other tool takes a pane id from here.",
                json!({}),
                &[][..],
                true,
                false,
            ),
            Tool::ReadScreen => (
                "Read screen",
                "Read the text visible in a pane right now, one line per row, as the user sees \
                 it.",
                json!({ "pane": pane_property() }),
                &["pane"][..],
                true,
                false,
            ),
            Tool::ReadScrollback => (
                "Read scrollback",
                "Read a pane's history: the lines that scrolled off the top, then the visible \
                 screen, oldest first. Pass `lines` to get only the last few; without it you \
                 get everything Hof kept, which can be long.",
                json!({
                    "pane": pane_property(),
                    "lines": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "How many of the most recent lines to return."
                    }
                }),
                &["pane"][..],
                true,
                false,
            ),
            Tool::SendInput => (
                "Send input",
                "Type text into a pane as if the user typed it. Control keys are characters: \
                 \\r is Enter, \\u0003 is Ctrl-C, \\u0004 is Ctrl-D, \\u001b is Escape. Returns \
                 at once; read_screen shows the effect. To run a shell command and get its \
                 output, run_command is better.",
                json!({
                    "pane": pane_property(),
                    "text": { "type": "string", "description": "The keys to type." },
                    "submit": {
                        "type": "boolean",
                        "default": false,
                        "description": "Press Enter after the text."
                    }
                }),
                &["pane", "text"][..],
                false,
                true,
            ),
            Tool::RunCommand => (
                "Run command",
                "Run a one-line shell command in a pane that is at its shell prompt, wait for \
                 it to finish, and return its exit code and output. Needs shell integration \
                 (OSC 133) in that shell. When the timeout passes first the command keeps \
                 running: read_screen shows how it is doing and send_input with \\u0003 \
                 interrupts it.",
                json!({
                    "pane": pane_property(),
                    "command": { "type": "string", "description": "The command line, without a newline." },
                    "timeout_seconds": timeout_property("How long to wait for the command to finish.")
                }),
                &["pane", "command"][..],
                false,
                true,
            ),
            Tool::NewTab => (
                "New tab",
                "Open a new tab and return its pane id. It runs the user's shell unless you \
                 give a command.",
                json!({
                    "directory": {
                        "type": "string",
                        "description": "Absolute path to start in."
                    },
                    "command": command_property(),
                    "host": host_property("this machine")
                }),
                &[][..],
                false,
                false,
            ),
            Tool::SplitPane => (
                "Split pane",
                "Split a pane in two and return the new pane's id. The new pane runs the \
                 user's shell unless you give a command.",
                json!({
                    "pane": pane_property(),
                    "direction": {
                        "type": "string",
                        "enum": ["right", "down"],
                        "description": "Where the new pane goes."
                    },
                    "command": command_property(),
                    "host": host_property("the split pane's machine")
                }),
                &["pane", "direction"][..],
                false,
                false,
            ),
            Tool::FocusPane => (
                "Focus pane",
                "Give a pane keyboard focus, switching to its tab, so the user sees it.",
                json!({ "pane": pane_property() }),
                &["pane"][..],
                false,
                false,
            ),
            Tool::ClosePane => (
                "Close pane",
                "Close a pane, ending whatever runs in it.",
                json!({ "pane": pane_property() }),
                &["pane"][..],
                false,
                true,
            ),
            Tool::SetAgentState => (
                "Set agent state",
                "Tell Hof how the AI agent in a pane is, when Hof cannot tell by itself: \
                 working, waiting for the user, idle, or none to say there is no agent. Hof \
                 shows it on the pane and its tab, and tells the user when an agent they \
                 cannot see starts waiting. It holds while the program now in the pane's \
                 foreground runs.",
                json!({
                    "pane": pane_property(),
                    "state": agent_state_property(),
                    "agent": {
                        "type": "string",
                        "description": "The agent's name, when Hof does not know it."
                    }
                }),
                &["pane", "state"][..],
                false,
                false,
            ),
            Tool::WaitForAgent => (
                "Wait for agent",
                "Wait until the AI agent in a pane is in one of the given states (by default \
                 waiting or idle: it needs the user, or is done), and return the state it \
                 reached. Returns at once when it is in one already. For an agent you started \
                 in a pane with new_tab or split_pane, then read_screen to see what it says.",
                json!({
                    "pane": pane_property(),
                    "states": {
                        "type": "array",
                        "items": agent_state_property(),
                        "minItems": 1,
                        "default": ["waiting", "idle"],
                        "description": "The states to wait for."
                    },
                    "timeout_seconds": timeout_property("How long to wait.")
                }),
                &["pane"][..],
                true,
                false,
            ),
        };
        json!({
            "name": self.name(),
            "title": title,
            "description": description,
            "inputSchema": {
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false
            },
            "annotations": {
                "title": title,
                "readOnlyHint": read_only,
                "destructiveHint": destructive,
                "openWorldHint": false
            }
        })
    }
}

fn pane_property() -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "description": "The pane's id, from list_panes."
    })
}

fn agent_state_property() -> Value {
    json!({
        "type": "string",
        "enum": ["working", "waiting", "idle", "none"]
    })
}

fn timeout_property(description: &str) -> Value {
    json!({
        "type": "number",
        "exclusiveMinimum": 0,
        "maximum": MAX_TIMEOUT_SECONDS,
        "default": DEFAULT_TIMEOUT_SECONDS,
        "description": description
    })
}

fn command_property() -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "description": "Program and arguments to run instead of the shell."
    })
}

fn host_property(default: &str) -> Value {
    json!({
        "type": "string",
        "description": format!(
            "The machine to run on, a name from the user's [hosts]; empty for this \
             one. Without it, {default}."
        )
    })
}

/// The tools `policy` allows, as `tools/list` describes them.
pub(crate) fn definitions(policy: Policy) -> Vec<Value> {
    Tool::ALL
        .into_iter()
        .filter(|tool| tool.allowed(policy))
        .map(Tool::definition)
        .collect()
}

/// What a tool call came to: text for the agent, and whether it failed.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub text: String,
    pub is_error: bool,
}

impl Outcome {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

impl From<HostError> for Outcome {
    fn from(error: HostError) -> Self {
        Outcome::error(error.to_string())
    }
}

#[derive(Deserialize)]
struct PaneArgs {
    pane: PaneId,
}

#[derive(Deserialize)]
struct ScrollbackArgs {
    pane: PaneId,
    lines: Option<usize>,
}

#[derive(Deserialize)]
struct SendInputArgs {
    pane: PaneId,
    text: String,
    #[serde(default)]
    submit: bool,
}

#[derive(Deserialize)]
struct RunCommandArgs {
    pane: PaneId,
    command: String,
    timeout_seconds: Option<f64>,
}

#[derive(Deserialize)]
struct SetAgentStateArgs {
    pane: PaneId,
    state: AgentState,
    agent: Option<String>,
}

#[derive(Deserialize)]
struct WaitForAgentArgs {
    pane: PaneId,
    states: Option<Vec<AgentState>>,
    timeout_seconds: Option<f64>,
}

#[derive(Deserialize)]
struct NewTabArgs {
    directory: Option<String>,
    #[serde(default)]
    command: Vec<String>,
    host: Option<String>,
}

#[derive(Deserialize)]
struct SplitArgs {
    pane: PaneId,
    direction: Direction,
    #[serde(default)]
    command: Vec<String>,
    host: Option<String>,
}

/// Run `tool` with `arguments`, already known to be an object.
pub(crate) fn call(
    host: &dyn Host,
    policy: Policy,
    tool: Tool,
    arguments: Map<String, Value>,
) -> Outcome {
    if !tool.allowed(policy) {
        return Outcome::error(tool.refusal());
    }
    match run(host, tool, Value::Object(arguments)) {
        Ok(outcome) | Err(outcome) => outcome,
    }
}

fn run(host: &dyn Host, tool: Tool, arguments: Value) -> Result<Outcome, Outcome> {
    let outcome = match tool {
        Tool::ListPanes => {
            let panes = host.list_panes();
            Outcome::ok(serde_json::to_string(&panes).map_err(|e| Outcome::error(e.to_string()))?)
        }
        Tool::ReadScreen => {
            let PaneArgs { pane } = parse(tool, arguments)?;
            Outcome::ok(host.read_screen(pane)?)
        }
        Tool::ReadScrollback => {
            let ScrollbackArgs { pane, lines } = parse(tool, arguments)?;
            if lines == Some(0) {
                return Err(Outcome::error("lines must be at least 1"));
            }
            Outcome::ok(host.read_scrollback(pane, lines)?)
        }
        Tool::SendInput => {
            let SendInputArgs {
                pane,
                mut text,
                submit,
            } = parse(tool, arguments)?;
            if submit {
                text.push('\r');
            }
            let length = text.chars().count();
            host.send_input(pane, text)?;
            Outcome::ok(format!("typed {length} characters into pane {pane}"))
        }
        Tool::RunCommand => {
            let RunCommandArgs {
                pane,
                command,
                timeout_seconds,
            } = parse(tool, arguments)?;
            let timeout = timeout(timeout_seconds)?;
            if command.trim().is_empty() {
                return Err(Outcome::error("command is empty"));
            }
            // A second line would run as a second command, and the first
            // one's end mark would be taken for the whole thing's.
            if command.contains(['\n', '\r']) {
                return Err(Outcome::error(
                    "command must be one line; join commands with `&&` or `;`",
                ));
            }
            let result = host.run_command(pane, command, timeout)?;
            let status = match (result.timed_out, result.exit_code) {
                (true, _) => format!(
                    "still running after {}s in pane {pane}; output so far:",
                    timeout.as_secs_f64()
                ),
                (false, Some(code)) => format!("exit code {code}"),
                (false, None) => "finished; the shell did not report an exit code".to_owned(),
            };
            Outcome::ok(format!("{status}\n{}", result.output))
        }
        Tool::NewTab => {
            let NewTabArgs {
                directory,
                command,
                host: on,
            } = parse(tool, arguments)?;
            let pane = host.new_tab(directory, command, on)?;
            Outcome::ok(json!({ "pane": pane }).to_string())
        }
        Tool::SplitPane => {
            let SplitArgs {
                pane,
                direction,
                command,
                host: on,
            } = parse(tool, arguments)?;
            let pane = host.split(pane, direction, command, on)?;
            Outcome::ok(json!({ "pane": pane }).to_string())
        }
        Tool::FocusPane => {
            let PaneArgs { pane } = parse(tool, arguments)?;
            host.focus_pane(pane)?;
            Outcome::ok(format!("focused pane {pane}"))
        }
        Tool::ClosePane => {
            let PaneArgs { pane } = parse(tool, arguments)?;
            host.close_pane(pane)?;
            Outcome::ok(format!("closed pane {pane}"))
        }
        Tool::SetAgentState => {
            let SetAgentStateArgs { pane, state, agent } = parse(tool, arguments)?;
            let agent = agent.filter(|a| !a.trim().is_empty());
            host.set_agent_state(pane, state, agent)?;
            Outcome::ok(format!("the agent in pane {pane} is {}", state.name()))
        }
        Tool::WaitForAgent => {
            let WaitForAgentArgs {
                pane,
                states,
                timeout_seconds,
            } = parse(tool, arguments)?;
            let timeout = timeout(timeout_seconds)?;
            let states = states.unwrap_or_else(|| vec![AgentState::Waiting, AgentState::Idle]);
            if states.is_empty() {
                return Err(Outcome::error("states must name at least one state"));
            }
            let wait = host.wait_for_agent(pane, states, timeout)?;
            Outcome::ok(
                json!({
                    "agent": wait.agent,
                    "state": wait.state,
                    "timed_out": wait.timed_out,
                })
                .to_string(),
            )
        }
    };
    Ok(outcome)
}

/// Arguments that do not fit the schema are the agent's to fix, so they
/// come back as a tool error it can read, not a protocol error.
fn parse<T: DeserializeOwned>(tool: Tool, arguments: Value) -> Result<T, Outcome> {
    serde_json::from_value(arguments)
        .map_err(|e| Outcome::error(format!("bad arguments for {}: {e}", tool.name())))
}

fn timeout(seconds: Option<f64>) -> Result<Duration, Outcome> {
    let seconds = seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !(seconds > 0.0 && seconds <= MAX_TIMEOUT_SECONDS) {
        return Err(Outcome::error(format!(
            "timeout_seconds must be more than 0 and at most {MAX_TIMEOUT_SECONDS}"
        )));
    }
    Ok(Duration::from_secs_f64(seconds))
}
