//! The way into the window for the two servers that drive it from outside:
//! the MCP server, for agents, and the control socket, for `hof ctl` and
//! scripts. Both answer on their own threads; the panes live on the UI
//! thread. So each request becomes a job posted there, which calls the
//! window's `mcp*` functions and the terminals' invokables, and the
//! server's thread waits for the answer.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use hof_mcp::{
    AgentState, AgentWait, CommandResult, Direction, Host, HostError, HostResult, PaneId, PaneInfo,
    Policy,
};
use ljos::prelude::*;
use serde::Deserialize;

/// How long a request waits for the window. A window that takes longer is
/// stuck, and the agent is better told so than left waiting.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);

thread_local! {
    /// The running servers; dropping one stops it and removes its socket.
    static SERVERS: RefCell<Vec<hof_mcp::Server>> = const { RefCell::new(Vec::new()) };
}

/// Where the servers for a window listen; `None` for one that does not run.
#[derive(Debug, Default)]
pub(crate) struct Served {
    pub control: Option<PathBuf>,
    pub mcp: Option<PathBuf>,
}

/// Starts the servers for `window`: the control socket always, and the
/// MCP server when the config asks for it.
pub(crate) fn serve(engine: &Engine, window: ObjectId) -> Served {
    let host = host(engine, window);
    // Only this user can reach it, who can do all this anyway.
    let control = kept_control(&host).or_else(|| {
        start(
            engine,
            "control socket",
            control_path(),
            Arc::clone(&host),
            Policy::ALLOW_ALL,
        )
    });
    let config = hof_config::load(&crate::settings::config_path())
        .map(|l| l.config.mcp)
        .unwrap_or_default();
    let mcp = config.enabled.then(|| {
        let path = config.socket.unwrap_or_else(hof_mcp::socket_path);
        let policy = Policy {
            allow_input: config.allow_input,
            allow_spawn: config.allow_spawn,
        };
        start(engine, "MCP server", path, host, policy)
    });
    Served {
        control,
        mcp: mcp.flatten(),
    }
}

fn start(
    engine: &Engine,
    what: &str,
    path: PathBuf,
    host: Arc<dyn Host>,
    policy: Policy,
) -> Option<PathBuf> {
    match hof_mcp::serve_unix(&path, host, policy) {
        Ok(server) => {
            SERVERS.with(|s| s.borrow_mut().push(server));
            Some(path)
        }
        Err(e) => {
            engine.warn(format!("hof: no {what}, {} is taken: {e}", path.display()));
            None
        }
    }
}

/// The control socket of the window whose panes may outlive it: where the
/// next Hof to keep its session serves too (`hof-session[-NAME].sock`), so
/// that `hof ctl` in a pane whose program ran on reaches the Hof that
/// attached it again. `None` for a window of a test or a dropdown, which
/// keep no session so.
fn kept_control(host: &Arc<dyn Host>) -> Option<PathBuf> {
    let launch = crate::launch();
    if launch.agent.is_none() || launch.dropdown {
        return None;
    }
    let path = crate::held::control_socket(launch.session_name.as_deref()?);
    let server = hof_mcp::serve_unix(&path, Arc::clone(host), Policy::ALLOW_ALL).ok()?;
    SERVERS.with(|s| s.borrow_mut().push(server));
    Some(path)
}

/// A socket of this window's own, beside the MCP one: `hof-<pid>-<n>.sock`,
/// as one process can open more than one window (tests do).
fn control_path() -> PathBuf {
    static WINDOWS: AtomicU32 = AtomicU32::new(0);
    let n = WINDOWS.fetch_add(1, Ordering::Relaxed);
    hof_mcp::socket_path().with_file_name(format!("hof-{}-{n}.sock", std::process::id()))
}

/// The [`Host`] that drives `window`, the QML `Main`. Made on the UI thread.
pub fn host(engine: &Engine, window: ObjectId) -> Arc<dyn Host> {
    Arc::new(WindowHost {
        post: Mutex::new(Platform::of(engine).poster()),
        window,
    })
}

struct WindowHost {
    /// Behind a lock as the server's threads share it.
    post: Mutex<Post>,
    window: ObjectId,
}

/// A pane as the window's `mcpPanes()` lists it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedPane {
    id: u64,
    tab: u64,
    title: String,
    working_directory: String,
    focused: bool,
    running: bool,
    columns: u16,
    rows: u16,
    agent: String,
    agent_state: String,
}

impl From<ListedPane> for PaneInfo {
    fn from(p: ListedPane) -> Self {
        PaneInfo {
            id: PaneId(p.id),
            tab: p.tab,
            title: p.title,
            working_directory: (!p.working_directory.is_empty()).then_some(p.working_directory),
            focused: p.focused,
            running: p.running,
            columns: p.columns,
            rows: p.rows,
            agent: (!p.agent.is_empty()).then_some(p.agent),
            agent_state: AgentState::parse(&p.agent_state).unwrap_or_default(),
        }
    }
}

impl WindowHost {
    /// Runs `job` on the UI thread with the window, and waits for what it
    /// returns.
    fn ask<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Engine, ObjectId) -> T + Send + 'static,
    ) -> HostResult<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        let window = self.window;
        self.post
            .lock()
            .map_err(|_| HostError::Failed("Hof's window has gone".into()))?
            .send(move |e: &Engine| {
                let _ = tx.send(job(e, window));
            });
        rx.recv_timeout(ANSWER_WITHIN)
            .map_err(|_| HostError::Failed("Hof's window is not answering".into()))
    }
}

/// Calls the QML function or invokable `name` of `obj`.
fn call(e: &Engine, obj: ObjectId, name: &str, args: &[Value]) -> Value {
    e.invoke_by_name(obj, name, args)
        .unwrap_or(Value::Undefined)
}

fn text(v: Value) -> String {
    v.to_js_string().as_str().to_owned()
}

fn pane_arg(pane: PaneId) -> Value {
    Value::from(i32::try_from(pane.0).unwrap_or(i32::MAX))
}

fn list(items: Vec<String>) -> Value {
    Value::list(items.iter().map(|i| Value::from(i.as_str())).collect())
}

/// The terminal of `pane`.
fn terminal(e: &Engine, window: ObjectId, pane: PaneId) -> HostResult<ObjectId> {
    call(e, window, "terminalOf", &[pane_arg(pane)])
        .as_object()
        .ok_or(HostError::NoSuchPane(pane))
}

/// A new pane's id, as the window's functions return it: 0 for none.
fn new_pane(v: Value) -> HostResult<PaneId> {
    match v.to_number() {
        n if n >= 1.0 => Ok(PaneId(n as u64)),
        _ => Err(HostError::Failed("no pane was made".into())),
    }
}

/// The agent of terminal `t`, as it is now.
fn agent_of(e: &Engine, t: ObjectId) -> (Option<String>, AgentState) {
    let name = text(e.read_by_name(t, "agent"));
    let state = AgentState::parse(&text(e.read_by_name(t, "agentState"))).unwrap_or_default();
    ((!name.is_empty()).then_some(name), state)
}

/// The last `lines` lines of `text`, or all of it.
fn last_lines(text: &str, lines: Option<usize>) -> String {
    let Some(n) = lines else {
        return text.to_owned();
    };
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(n)..].join("\n")
}

impl Host for WindowHost {
    fn list_panes(&self) -> Vec<PaneInfo> {
        self.ask(|e, w| text(call(e, w, "mcpPanes", &[])))
            .ok()
            .and_then(|json| serde_json::from_str::<Vec<ListedPane>>(&json).ok())
            .map(|panes| panes.into_iter().map(PaneInfo::from).collect())
            .unwrap_or_default()
    }

    fn read_screen(&self, pane: PaneId) -> HostResult<String> {
        self.ask(move |e, w| Ok(text(call(e, terminal(e, w, pane)?, "screenText", &[]))))?
    }

    fn read_scrollback(&self, pane: PaneId, lines: Option<usize>) -> HostResult<String> {
        let all = self.ask(move |e, w| Ok(text(call(e, terminal(e, w, pane)?, "text", &[]))))??;
        Ok(last_lines(&all, lines))
    }

    fn send_input(&self, pane: PaneId, input: String) -> HostResult<()> {
        self.ask(move |e, w| {
            call(
                e,
                terminal(e, w, pane)?,
                "send",
                &[Value::from(input.as_str())],
            );
            Ok(())
        })?
    }

    fn new_tab(
        &self,
        directory: Option<String>,
        command: Vec<String>,
        host: Option<String>,
    ) -> HostResult<PaneId> {
        let directory = directory.unwrap_or_default();
        let host = host.unwrap_or_default();
        let pane = self.ask(move |e, w| {
            call(
                e,
                w,
                "mcpNewTab",
                &[
                    Value::from(directory.as_str()),
                    list(command),
                    Value::from(host.as_str()),
                ],
            )
        })?;
        new_pane(pane)
    }

    fn split(
        &self,
        pane: PaneId,
        direction: Direction,
        command: Vec<String>,
        host: Option<String>,
    ) -> HostResult<PaneId> {
        let toward = match direction {
            Direction::Right => "right",
            Direction::Down => "down",
        };
        self.ask(move |e, w| {
            terminal(e, w, pane)?;
            // Null: the split pane's own.
            let host = host.as_deref().map_or(Value::Null, Value::from);
            let args = [pane_arg(pane), Value::from(toward), list(command), host];
            new_pane(call(e, w, "mcpSplit", &args))
        })?
    }

    fn close_pane(&self, pane: PaneId) -> HostResult<()> {
        self.ask(move |e, w| {
            terminal(e, w, pane)?;
            call(e, w, "closePane", &[pane_arg(pane)]);
            Ok(())
        })?
    }

    fn focus_pane(&self, pane: PaneId) -> HostResult<()> {
        self.ask(
            move |e, w| match call(e, w, "mcpFocus", &[pane_arg(pane)]).to_boolean() {
                true => Ok(()),
                false => Err(HostError::NoSuchPane(pane)),
            },
        )?
    }

    fn run_command(
        &self,
        pane: PaneId,
        command: String,
        timeout: Duration,
    ) -> HostResult<CommandResult> {
        let (done, finished) = mpsc::sync_channel::<Option<i32>>(1);
        // Typed at the prompt, and an ear on the terminal for the shell's
        // end-of-command mark: the wait is here, not on the UI thread.
        let (t, ear) = self.ask(move |e, w| {
            let t = terminal(e, w, pane)?;
            if !e.read_by_name(t, "shellIntegration").to_boolean() {
                return Err(HostError::NoShellIntegration(pane));
            }
            if e.read_by_name(t, "commandRunning").to_boolean() {
                return Err(HostError::Busy(pane));
            }
            call(e, t, "markCommand", &[]);
            let ear = e
                .connect_by_name(t, "commandFinished", move |_: &Engine, args: &[Value]| {
                    let code = args.first().map(|v| v.to_number() as i32);
                    // -1 is the terminal's "no code given".
                    let _ = done.try_send(code.filter(|&c| c >= 0));
                })
                .map_err(|e| HostError::Failed(e.to_string()))?;
            call(
                e,
                t,
                "send",
                &[Value::from(format!("{command}\r").as_str())],
            );
            Ok((t, ear))
        })??;
        let ended = finished.recv_timeout(timeout);
        let output = self.ask(move |e, _| {
            e.disconnect(ear);
            let output = text(call(e, t, "commandOutput", &[]));
            call(e, t, "endCommand", &[]);
            output
        })?;
        Ok(match ended {
            Ok(exit_code) => CommandResult {
                exit_code,
                output,
                timed_out: false,
            },
            Err(_) => CommandResult {
                exit_code: None,
                output,
                timed_out: true,
            },
        })
    }

    fn set_agent_state(
        &self,
        pane: PaneId,
        state: AgentState,
        agent: Option<String>,
    ) -> HostResult<()> {
        self.ask(move |e, w| {
            let t = terminal(e, w, pane)?;
            let name = Value::from(agent.as_deref().unwrap_or(""));
            call(e, t, "reportAgent", &[Value::from(state.name()), name]);
            Ok(())
        })?
    }

    fn wait_for_agent(
        &self,
        pane: PaneId,
        states: Vec<AgentState>,
        timeout: Duration,
    ) -> HostResult<AgentWait> {
        let (reached, came) = mpsc::sync_channel::<()>(1);
        // Looked at and listened to in one job on the UI thread, so no
        // change can slip between the two.
        let (t, ear) = self.ask(move |e, w| {
            let t = terminal(e, w, pane)?;
            let (_, now) = agent_of(e, t);
            if states.contains(&now) {
                return Ok((t, None));
            }
            let ear = e
                .connect_by_name(t, "agentStatus", move |_: &Engine, args: &[Value]| {
                    let state = args.get(1).map(|v| text(v.clone()));
                    if state
                        .and_then(|s| AgentState::parse(&s))
                        .is_some_and(|s| states.contains(&s))
                    {
                        let _ = reached.try_send(());
                    }
                })
                .map_err(|e| HostError::Failed(e.to_string()))?;
            Ok((t, Some(ear)))
        })??;
        let timed_out = ear.is_some() && came.recv_timeout(timeout).is_err();
        self.ask(move |e, w| {
            if let Some(ear) = ear {
                e.disconnect(ear);
            }
            // It may have closed meanwhile.
            terminal(e, w, pane)?;
            let (agent, state) = agent_of(e, t);
            Ok(AgentWait {
                agent,
                state,
                timed_out,
            })
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_lines_are_the_newest() {
        assert_eq!(last_lines("a\nb\nc", Some(2)), "b\nc");
        assert_eq!(last_lines("a\nb", Some(5)), "a\nb");
        assert_eq!(last_lines("a\nb", None), "a\nb");
    }
}
