//! Hof quitting with a session to keep, though the last save of it failed:
//! the programs of its panes run on all the same. One test, since the
//! choice is the process's.

use std::path::{Path, PathBuf};

use ljos::prelude::*;
use ljos::testing::TestWindow;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("unsaved-quit")
}

#[test]
fn a_session_not_saved_as_hof_quits_leaves_its_programs_running() {
    let dir = dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("hof.toml"),
        "[terminal]\nshell = \"cat\"\n\n[session]\nrestore = true\n",
    )
    .unwrap();
    // Under a file, not a directory: no save there can succeed, as none to
    // a machine out of reach can.
    let blocked = dir.join("a-file");
    std::fs::write(&blocked, "").unwrap();
    hof::set_launch(hof::Launch {
        config: Some(dir.join("hof.toml")),
        session: Some(blocked.join("session.json")),
        ..hof::Launch::default()
    });

    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();

    let saved = engine
        .invoke_by_name(root, "saveSession", &[Value::from(true)])
        .unwrap();
    assert!(!saved.to_boolean(), "the save failed");
    assert!(
        hof_qml::remote::keeps_programs(),
        "the programs run on, for the session saved before to attach to"
    );
    hof_qml::remote::keep_programs(false);
}
