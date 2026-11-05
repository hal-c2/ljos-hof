//! `hof attach work`: a session by name, kept apart from a plain `hof`'s
//! and restored though the config does not ask for it. One test, since the
//! launch, the config and where sessions are kept are the process's.

use std::path::{Path, PathBuf};

use ljos::prelude::*;
use ljos::testing::TestWindow;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("session-attach")
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
}

#[test]
fn a_named_session_is_kept_apart_and_restored_though_the_config_says_not() {
    let dir = dir();
    let _ = std::fs::remove_dir_all(dir.join("state"));
    std::fs::create_dir_all(&dir).unwrap();
    // Where sessions are kept, for this process only, before any thread
    // reads the environment.
    unsafe { std::env::set_var("XDG_STATE_HOME", dir.join("state")) };
    // `cat` for the shell, and no restoring but what `hof attach` asks.
    std::fs::write(
        dir.join("hof.toml"),
        "[terminal]\nshell = \"cat\"\n\n[session]\nrestore = false\n",
    )
    .unwrap();
    hof::set_launch(hof::Launch {
        config: Some(dir.join("hof.toml")),
        session_name: Some("work".into()),
        restore: Some(true),
        ..hof::Launch::default()
    });
    let work = dir.join("state/hof/sessions/work/session.json");

    let mut hof = opened();
    assert!(
        hof.engine
            .read_by_name(hof.root, "title")
            .to_js_string()
            .as_str()
            .ends_with(" — work"),
        "the window says whose it is"
    );
    hof.run("split-right");
    hof.window.advance(2500).unwrap();
    assert!(work.exists(), "kept under its name");
    assert!(
        !dir.join("state/hof/session.json").exists(),
        "a plain hof's is left be"
    );
    drop(hof);

    let mut hof = opened();
    assert_eq!(hof.panes(), 2, "opened again as it was");
    for _ in 0..2 {
        hof.run("close-pane");
    }
    assert!(
        !work.parent().unwrap().exists(),
        "closed on purpose, it is gone from `hof sessions`"
    );
}
