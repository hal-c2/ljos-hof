//! `hof attach --host desktop work`: a session kept on another machine,
//! its panes run there and the session saved there as it changes. The
//! other machine is an agent in this process, its sessions in a directory
//! of the test's. One test, since the launch, the hosts and the session
//! kept elsewhere are the process's.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::{Agent, Options};
use hof_remote::client::{self, Connection, Host, Transport};
use hof_session::service::Places;
use hof_session::{SavedNode, SavedPane, SavedTab, SavedWindow, Session};
use ljos::prelude::*;
use ljos::testing::TestWindow;

const PATIENCE: Duration = Duration::from_secs(10);

/// A transport to an agent in this process.
struct Pipe(Agent);

impl Transport for Pipe {
    fn connect(&mut self) -> io::Result<Connection> {
        let (ours, theirs) = UnixStream::pair()?;
        let agent = self.0.clone();
        thread::spawn(move || {
            let reader = theirs.try_clone().unwrap();
            let _ = agent.serve(reader, theirs);
        });
        let cut = ours.try_clone()?;
        Ok(Connection {
            reader: Box::new(ours.try_clone()?),
            writer: Box::new(ours),
            end: Box::new(move || {
                let _ = cut.shutdown(std::net::Shutdown::Both);
                None
            }),
        })
    }
}

/// A pane's program on `desktop`, as a window there left it running.
#[derive(Default)]
struct Running {
    up: Mutex<Option<u32>>,
}

impl client::Screen for Running {
    fn output(&self, _: &[u8]) {}
    fn replace(&self, _: &[u8]) {}
    fn exited(&self, _: Option<i32>) {}
    fn state(&self, state: client::State) {
        if let client::State::Up { pane } = state {
            *self.up.lock().unwrap() = Some(pane);
        }
    }
}

/// Starts `n` programs on `desktop`'s agent and leaves them running, as a
/// window there that closed would. Returns their panes there.
fn left_running(agent: &Agent, n: usize) -> Vec<u32> {
    let host = Host::new(Pipe(agent.clone()), client::Options::default());
    let running: Vec<_> = (0..n)
        .map(|_| {
            let seen = Arc::new(Running::default());
            let start = client::Start {
                command: vec!["cat".into()],
                cols: 40,
                rows: 5,
                ..client::Start::default()
            };
            (host.open(start, seen.clone()), seen)
        })
        .collect();
    let deadline = Instant::now() + PATIENCE;
    let ids = loop {
        let ids: Option<Vec<u32>> = running.iter().map(|(_, s)| *s.up.lock().unwrap()).collect();
        if let Some(ids) = ids {
            break ids;
        }
        assert!(Instant::now() < deadline, "the programs start there");
        thread::sleep(Duration::from_millis(20));
    };
    for (pane, _) in &running {
        pane.detach();
    }
    drop(running);
    host.finish(true, Duration::from_secs(2));
    ids
}

/// The session as `desktop` keeps it: two tabs, the first split three
/// ways and the second two, the second tab current, all its own, their
/// programs `running` there.
fn left_on_desktop(running: &[u32]) -> Session {
    let pane = |id: u32| {
        Box::new(SavedNode::Pane(SavedPane {
            id,
            working_directory: Some("/".into()),
            remote: Some(running[id as usize - 1]),
            ..SavedPane::default()
        }))
    };
    let split = |axis, first, second| {
        Box::new(SavedNode::Split {
            axis,
            ratio: 0.5,
            first,
            second,
        })
    };
    use hof_session::SavedAxis::{Horizontal, Vertical};
    Session::new(vec![SavedWindow {
        tabs: vec![
            SavedTab {
                root: *split(Horizontal, pane(1), split(Vertical, pane(2), pane(3))),
                focused: 2,
                zoomed: false,
            },
            SavedTab {
                root: *split(Vertical, pane(4), pane(5)),
                focused: 5,
                zoomed: false,
            },
        ],
        current_tab: 1,
        width: 0,
        height: 0,
        dropdown: false,
    }])
}

fn until(what: &str, window: &mut TestWindow, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < deadline, "waiting for {what}");
        window.advance(50).unwrap();
        thread::sleep(Duration::from_millis(20));
    }
}

fn saved_there(places: &Places) -> Option<Session> {
    hof_session::load(&hof_session::named_beside(&places.default, "work")).unwrap()
}

#[test]
fn a_session_on_another_machine_runs_and_is_kept_there() {
    let dir = tempfile::tempdir().unwrap();
    let here = dir.path().join("here");
    std::fs::create_dir_all(&here).unwrap();
    // This machine's state and runtime files, and the shell the agent
    // there starts, for this process only, before any thread reads them.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", here.join("state"));
        std::env::set_var("XDG_RUNTIME_DIR", here.join("run"));
        std::env::set_var("SHELL", "cat");
    }
    let places = Places {
        locks: dir.path().join("there/run"),
        default: dir.path().join("there/state/session.json"),
    };
    let agent = Agent::new(Options {
        name: "desktop".into(),
        sessions: places.clone(),
        ..Options::default()
    });
    let running = left_running(&agent, 5);
    hof_session::save(
        &hof_session::named_beside(&places.default, "work"),
        &left_on_desktop(&running),
    )
    .unwrap();
    hof_qml::remote::set_host(
        "desktop",
        Host::new(Pipe(agent.clone()), client::Options::default()),
    );
    let config = here.join("hof.toml");
    std::fs::write(&config, "[session]\nrestore = false\n").unwrap();

    let name = hof::away::open(
        "desktop",
        Some("work"),
        &hof_config::load(&config).unwrap().config,
    )
    .unwrap();
    assert_eq!(name, "work");
    assert!(
        hof_session::is_open_in(&places.locks, "work"),
        "held there, as a window there would"
    );
    hof::set_launch(hof::Launch {
        config: Some(config),
        session_name: Some(name),
        restore: Some(true),
        ..hof::Launch::default()
    });

    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    let title = engine.read_by_name(root, "title").to_js_string();
    assert!(
        title.as_str().ends_with(" — work on desktop"),
        "the window says whose it is and where: {title:?}"
    );
    assert_eq!(
        engine.read_by_name(root, "paneCount").to_number(),
        5.0,
        "opened as it was left there"
    );

    // Each pane, in either tab, is up and is the program it was there:
    // what is typed in it comes back from that program.
    for (pane, there) in (1..=5).zip(running.iter().copied()) {
        let terminal = || {
            engine
                .invoke_by_name(root, "terminalOf", &[Value::from(pane)])
                .unwrap()
                .as_object()
        };
        until(&format!("pane {pane} up"), &mut window, || {
            terminal().is_some_and(|t| {
                engine.read_by_name(t, "connection").to_js_string().as_str() == "up"
            })
        });
        let t = terminal().unwrap();
        assert_eq!(
            engine.read_by_name(t, "remotePane").to_number(),
            f64::from(there),
            "pane {pane} attached to its program there"
        );
        let word = format!("hello-{pane}");
        engine
            .invoke_by_name(t, "send", &[Value::from(format!("{word}\r").as_str())])
            .unwrap();
        until(&format!("pane {pane} answering"), &mut window, || {
            let shown = engine.invoke_by_name(t, "screenText", &[]).unwrap();
            shown.to_js_string().as_str().contains(&word)
        });
    }

    engine
        .invoke_by_name(root, "run", &[Value::from("split-right")])
        .unwrap();
    window.settle().unwrap();
    // All run there, and the session is kept there as it changed: each
    // pane its own there, picked up again by its number.
    until("the session to be kept there", &mut window, || {
        saved_there(&places).is_some_and(|s| {
            let w = &s.windows[0];
            let panes: Vec<_> = w.tabs.iter().flat_map(|t| t.panes()).collect();
            w.tabs.len() == 2
                && w.current_tab == 1
                && w.tabs[1].panes().len() == 3
                && panes.len() == 6
                && panes.iter().all(|p| p.host.is_none() && p.remote.is_some())
        })
    });
    // Each pane is the program it was, there: attached, none started anew.
    let saved = saved_there(&places).unwrap();
    let attached: Vec<_> = saved.windows[0]
        .tabs
        .iter()
        .flat_map(|t| t.panes())
        .filter(|p| p.id <= 5)
        .map(|p| (p.id, p.remote.unwrap()))
        .collect();
    let left: Vec<_> = (1..=5).zip(running.iter().copied()).collect();
    assert_eq!(attached, left, "each pane attached to its program there");
    assert!(!agent.is_idle(), "the programs run there");
    assert!(
        !Path::new(&here.join("state/hof")).exists(),
        "nothing of it kept here"
    );

    for _ in 0..6 {
        engine
            .invoke_by_name(root, "run", &[Value::from("close-pane")])
            .unwrap();
        window.settle().unwrap();
    }
    assert!(
        saved_there(&places).is_none(),
        "closed on purpose, it is forgotten there"
    );
    drop(window);
    drop(engine);
    hof_qml::remote::finish(Duration::from_secs(2));
}
