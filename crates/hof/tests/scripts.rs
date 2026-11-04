//! `[commands]`, `[hooks]` and plugins in a window: a command at a key
//! types, splits or runs in the background, and hooks hear panes open and
//! close. One test, as the config is read once a process.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use ljos::prelude::*;
use ljos::testing::TestWindow;

fn until(window: &mut TestWindow, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened");
        window.advance(16).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
}

fn log(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn commands_type_split_and_run_in_the_background_and_hooks_hear_panes() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("log");
    let config = dir.path().join("hof.toml");
    std::fs::write(
        &config,
        format!(
            r#"
            [terminal.env]
            LOG = {log:?}

            [commands.hello]
            send = "hello from a key\n"
            keys = "ctrl+alt+h"

            [commands.below]
            run = "exec cat"
            in = "split-down"

            [hooks]
            pane-opened = 'echo "$HOF_EVENT $HOF_PANE_ID" >> "$LOG"'
            pane-closed = 'echo "$HOF_EVENT $HOF_PANE_ID" >> "$LOG"'
            "#,
            log = log_path.display().to_string(),
        ),
    )
    .unwrap();
    let plugin = dir.path().join("plugins/notes");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("plugin.toml"),
        r#"
        [commands.note]
        run = 'echo "note from $HOF_PLUGIN_DIR in $HOF_PANE_ID" >> "$LOG"'
        in = "background"
        "#,
    )
    .unwrap();

    hof::set_launch(hof::Launch {
        command: vec!["cat".to_owned()],
        config: Some(config),
        ..hof::Launch::default()
    });
    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    until(&mut window, "the first pane's hook", || {
        log(&log_path).contains("pane-opened 1")
    });

    let run = |engine: &Engine, action: &str| {
        engine
            .invoke_by_name(root, "run", &[Value::from(action)])
            .unwrap();
    };
    let first = engine.read_by_name(root, "current").as_object().unwrap();
    run(&engine, "command:hello");
    until(&mut window, "the typed text", || {
        engine
            .invoke_by_name(first, "screenText", &[])
            .unwrap()
            .to_js_string()
            .as_str()
            .contains("hello from a key")
    });

    run(&engine, "command:note");
    let note = format!("note from {} in 1", plugin.display());
    until(&mut window, "the plugin's note", || {
        log(&log_path).contains(&note)
    });

    run(&engine, "command:below");
    window.settle().unwrap();
    assert_eq!(engine.read_by_name(root, "paneCount").to_number(), 2.0);
    until(&mut window, "the split's hook", || {
        log(&log_path).contains("pane-opened 2")
    });
    run(&engine, "close-pane");
    until(&mut window, "the closed pane's hook", || {
        log(&log_path).contains("pane-closed 2")
    });
}
