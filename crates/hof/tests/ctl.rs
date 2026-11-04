//! Scripts in a pane drive Hof with `hof ctl`, finding it by what Hof puts
//! in their environment. One test, as the config is read once a process.

use std::thread;
use std::time::{Duration, Instant};

use ljos::prelude::*;
use ljos::testing::TestWindow;

#[test]
fn a_script_in_a_pane_finds_hof_and_its_pane_and_splits_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("hof.toml");
    std::fs::write(&config, "[terminal.env]\nZQXJ = \"from the config\"\n").unwrap();
    let script = format!(
        "echo \"$ZQXJ in pane $HOF_PANE_ID\"; {hof} ctl split --down -- cat && echo split; exec cat",
        hof = env!("CARGO_BIN_EXE_hof"),
    );
    hof::set_launch(hof::Launch {
        command: vec!["sh".into(), "-c".into(), script],
        config: Some(config),
        ..hof::Launch::default()
    });
    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    // The script may split before this looks: the first pane by its id.
    let first = engine
        .invoke_by_name(root, "terminalOf", &[Value::from(1)])
        .unwrap()
        .as_object()
        .unwrap();

    let screen = |engine: &Engine| {
        engine
            .invoke_by_name(first, "screenText", &[])
            .unwrap()
            .to_js_string()
            .as_str()
            .to_owned()
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while !screen(&engine).contains("split") {
        assert!(
            Instant::now() < deadline,
            "the script never split: {:?}",
            screen(&engine)
        );
        window.advance(16).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    window.settle().unwrap();
    assert!(
        screen(&engine).contains("from the config in pane 1"),
        "{:?}",
        screen(&engine)
    );
    assert_eq!(engine.read_by_name(root, "paneCount").to_number(), 2.0);
}
