//! `[session] restore`: the window opens again as it was closed. One test,
//! since every window of this process reads the same config and session.

use std::path::{Path, PathBuf};

use ljos::prelude::*;
use ljos::testing::TestWindow;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("session-restore")
}

fn session() -> PathBuf {
    dir().join("session.json")
}

fn launch() {
    let dir = dir();
    std::fs::create_dir_all(&dir).unwrap();
    // `cat` for the shell: it prints nothing of its own, whatever the
    // user's shell does at start.
    std::fs::write(
        dir.join("hof.toml"),
        "[terminal]\nshell = \"cat\"\n\n[session]\nrestore = true\n",
    )
    .unwrap();
    hof::set_launch(hof::Launch {
        config: Some(dir.join("hof.toml")),
        session: Some(session()),
        ..hof::Launch::default()
    });
}

struct Hof {
    engine: Engine,
    window: TestWindow,
    root: ObjectId,
}

fn opened() -> Hof {
    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    Hof {
        engine,
        window,
        root,
    }
}

impl Hof {
    fn run(&mut self, action: &str) {
        self.engine
            .invoke_by_name(self.root, "run", &[Value::from(action)])
            .unwrap();
        self.window.settle().unwrap();
    }

    fn panes(&self) -> usize {
        self.engine.read_by_name(self.root, "paneCount").to_number() as usize
    }

    fn current(&self) -> ObjectId {
        self.engine
            .read_by_name(self.root, "current")
            .as_object()
            .expect("a terminal has the keys")
    }

    fn text(&self, t: ObjectId) -> String {
        self.engine
            .invoke_by_name(t, "text", &[])
            .unwrap()
            .to_js_string()
            .as_str()
            .to_owned()
    }

    fn save(&mut self) {
        let saved = self
            .engine
            .invoke_by_name(self.root, "saveSession", &[])
            .unwrap()
            .to_boolean();
        assert!(saved, "the session is saved");
        self.window.settle().unwrap();
    }
}

#[test]
fn a_window_opens_again_as_it_was_until_everything_in_it_is_closed() {
    launch();
    let _ = std::fs::remove_file(session());

    // Nothing saved yet: one fresh pane.
    let mut hof = opened();
    assert_eq!(hof.panes(), 1);
    let first = hof.current();
    hof.engine
        .invoke_by_name(first, "write", &[Value::from("zqxj was here\r\n")])
        .unwrap();
    hof.run("split-right");
    hof.run("new-tab");
    hof.run("previous-tab");
    hof.save();
    drop(hof);
    assert!(session().exists());

    // Both tabs, the split, and what the first pane showed.
    let mut hof = opened();
    assert_eq!(hof.panes(), 3);
    let layout = hof
        .engine
        .read_by_name(hof.root, "layout")
        .as_object()
        .unwrap();
    assert_eq!(hof.engine.read_by_name(layout, "tabCount").to_number(), 2.0);
    assert_eq!(
        hof.engine.read_by_name(layout, "currentTab").to_number(),
        0.0,
        "the tab that was shown is shown"
    );
    hof.run("focus-left");
    let first = hof.current();
    assert!(
        hof.engine.read_by_name(first, "running").to_boolean(),
        "its program runs again"
    );
    assert!(
        hof.text(first).contains("zqxj was here"),
        "the scrollback came back: {:?}",
        hof.text(first)
    );

    // Closing every pane is closing on purpose: nothing to open next time.
    for _ in 0..3 {
        hof.run("close-pane");
    }
    assert!(!session().exists(), "the session is forgotten");
}
