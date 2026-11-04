//! Ljós Hof, the application: its window in QML (`qml/`, compiled ahead of
//! time), the terminal item from `hof_qml`, and the two singletons the QML
//! reads, [`App`] (how it was started) and [`Settings`] (the config file).

pub mod ctl;
mod ipc;
mod mcp;
mod scripts;
mod sessions;
mod settings;
pub use hof_agent::tty;

use std::path::PathBuf;
use std::sync::OnceLock;

use ljos::prelude::*;

pub use ipc::{Request, send as send_request, socket_path};
pub use mcp::host as mcp_host;
pub use settings::Settings;

/// How this run was started: what the command line said.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    /// Run as a dropdown: frameless, on top, shown and hidden by `hof
    /// --toggle`.
    pub dropdown: bool,
    /// The first pane's program and its arguments, in place of the shell.
    pub command: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// The config file, in place of the usual one.
    pub config: Option<PathBuf>,
    /// The socket a dropdown listens on.
    pub socket: Option<PathBuf>,
    /// Read the config file again whenever it is saved. Off unless asked,
    /// so a test opens no watch.
    pub watch_config: bool,
    /// Where the session is saved, in place of the usual place.
    pub session: Option<PathBuf>,
}

static LAUNCH: OnceLock<Launch> = OnceLock::new();

/// What [`App`] and [`Settings`] read. Set once, before [`app`] runs; a
/// test that sets none gets the defaults.
pub fn set_launch(launch: Launch) {
    let _ = LAUNCH.set(launch);
}

pub(crate) fn launch() -> &'static Launch {
    LAUNCH.get_or_init(Launch::default)
}

/// How Hof was started, for the QML: `App.dropdown`, `App.command`.
#[derive(Debug, QmlObject)]
#[qml(module = "Hof.App", name = "App")]
#[qml_singleton]
pub struct App {
    #[qml(readonly)]
    pub dropdown: bool,
    #[qml(readonly)]
    pub command: Vec<String>,
    #[qml(readonly)]
    pub working_directory: String,
    #[qml(readonly)]
    pub version: String,
    /// Where `hof ctl` reaches this window; panes get it as `HOF_SOCKET`.
    #[qml(notify, readonly)]
    pub control_socket: String,
    /// Where agents reach it; empty when the MCP server is off.
    #[qml(notify, readonly)]
    pub mcp_socket: String,
}

impl Default for App {
    fn default() -> Self {
        let l = launch();
        Self {
            dropdown: l.dropdown,
            command: l.command.clone(),
            working_directory: l
                .working_directory
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            control_socket: String::new(),
            mcp_socket: String::new(),
        }
    }
}

#[qml_impl]
impl App {
    /// `hof --toggle` asked the dropdown to show or hide.
    #[qml(signal)]
    fn toggle_requested();

    /// Another `hof` asked for a new tab here (`hof --tab`), optionally
    /// running `command` from `directory`.
    #[qml(signal)]
    fn tab_requested(command: Vec<String>, directory: String);

    /// Saves `workspace`'s tabs and splits as the session, with what
    /// `panes` (JSON: pane id to `{directory, command, title, scrollback}`)
    /// says of each pane, and the window's size. Returns whether it saved.
    #[qml(invokable)]
    fn save_session(
        cx: &Cx,
        workspace: Option<ObjectId>,
        panes: String,
        width: f64,
        height: f64,
    ) -> bool {
        let Some(layout) = workspace.and_then(|w| hof_qml::Workspace::layout(cx.engine, w)) else {
            return false;
        };
        let saved = sessions::save(&sessions::path(), &layout.borrow(), &panes, width, height);
        // Hof is going, with the session to come back to: the programs of
        // remote panes run on, for the next start to attach to.
        hof_qml::remote::keep_programs(saved.is_ok());
        saved
            .inspect_err(|e| eprintln!("hof: the session was not saved: {e}"))
            .is_ok()
    }

    /// Puts the saved session's tabs and splits into `workspace`. Returns
    /// JSON for starting them, `{width, height, panes}` with `panes` by id
    /// as `Pane.start` takes them, or "" when there is none to open.
    #[qml(invokable)]
    fn restore_session(cx: &Cx, workspace: Option<ObjectId>) -> String {
        let Some(workspace) = workspace else {
            return String::new();
        };
        match sessions::load(&sessions::path()) {
            Ok(Some((layout, restored))) => {
                hof_qml::Workspace::replace(cx.engine, workspace, layout);
                serde_json::to_string(&restored).unwrap_or_default()
            }
            Ok(None) => String::new(),
            Err(e) => {
                eprintln!("hof: the last session could not be opened: {e}");
                String::new()
            }
        }
    }

    /// Forgets the saved session, so the next start is a fresh one.
    #[qml(invokable)]
    fn clear_session(_cx: &Cx) {
        if let Err(e) = sessions::clear(&sessions::path()) {
            eprintln!("hof: the session was not cleared: {e}");
        }
    }

    /// Starts the servers that drive `window`, the `Main`, from outside:
    /// the control socket (`controlSocket`), and the MCP server when
    /// `[mcp] enabled` is set (`mcpSocket`).
    #[qml(invokable)]
    fn serve(cx: &Cx, window: Option<ObjectId>) {
        let Some(window) = window else {
            return;
        };
        let served = mcp::serve(cx.engine, window);
        let text =
            |p: Option<std::path::PathBuf>| p.map(|p| p.display().to_string()).unwrap_or_default();
        let _ = cx
            .engine
            .set_internal(cx.this, App::CONTROL_SOCKET, text(served.control));
        let _ = cx
            .engine
            .set_internal(cx.this, App::MCP_SOCKET, text(served.mcp));
    }

    /// Starts `command` outside any pane, with `environment` (`NAME=value`
    /// each) on top of Hof's, in `directory` when it is one: a background
    /// command, or a hook. Returns whether it started.
    #[qml(invokable)]
    fn spawn(_cx: &Cx, command: Vec<String>, environment: Vec<String>, directory: String) -> bool {
        scripts::spawn(&command, &environment, &directory)
            .inspect_err(|e| eprintln!("hof: could not run {command:?}: {e}"))
            .is_ok()
    }

    #[qml(on_complete)]
    fn completed(cx: &Cx) -> Result<(), EngineError> {
        if launch().dropdown {
            ipc::listen(cx.engine, cx.this);
        }
        Ok(())
    }
}

/// Registers what the QML imports: `Hof` (the terminal and the layout)
/// and `Hof.App`.
pub fn register(engine: &Engine) {
    hof_qml::register(engine);
    engine.register_singleton::<App>("Hof.App", Version::new(1, 0));
    engine.register_singleton::<Settings>("Hof.App", Version::new(1, 0));
}

/// The application, ready to run.
pub fn app() -> ljos::App {
    ljos::App::new("Hof")
        .desktop_file_name("org.ljos.Hof")
        .register(register)
        .source(ljos::include_qml!("qml.pack"))
        .compiled(ljos::include_compiled!("qml.rs"))
        .root("Main")
}
