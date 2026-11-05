//! `hof attach --host desktop work`: a session kept on another machine,
//! its panes run there and the session saved there as it changes. The
//! other machine is an agent in this process, its sessions in a directory
//! of the test's. One test, since the launch, the hosts and the session
//! kept elsewhere are the process's.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
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

/// The session as `desktop` keeps it: two panes side by side, its own.
fn left_on_desktop() -> Session {
    let pane = |id| {
        Box::new(SavedNode::Pane(SavedPane {
            id,
            ..SavedPane::default()
        }))
    };
    Session::new(vec![SavedWindow {
        tabs: vec![SavedTab {
            root: SavedNode::Split {
                axis: hof_session::SavedAxis::Horizontal,
                ratio: 0.5,
                first: pane(1),
                second: pane(2),
            },
            focused: 2,
            zoomed: false,
        }],
        current_tab: 0,
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
    hof_session::save(
        &hof_session::named_beside(&places.default, "work"),
        &left_on_desktop(),
    )
    .unwrap();
    let agent = Agent::new(Options {
        name: "desktop".into(),
        sessions: places.clone(),
        ..Options::default()
    });
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
        2.0,
        "opened as it was left there"
    );

    engine
        .invoke_by_name(root, "run", &[Value::from("split-right")])
        .unwrap();
    window.settle().unwrap();
    // All run there, and the session is kept there as it changed: each
    // pane its own there, picked up again by its number.
    until("the session to be kept there", &mut window, || {
        saved_there(&places).is_some_and(|s| {
            let panes = s.windows[0].tabs[0].panes();
            panes.len() == 3 && panes.iter().all(|p| p.host.is_none() && p.remote.is_some())
        })
    });
    assert!(!agent.is_idle(), "the programs run there");
    assert!(
        !Path::new(&here.join("state/hof")).exists(),
        "nothing of it kept here"
    );

    for _ in 0..3 {
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
