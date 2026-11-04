//! The window's view of the agents in its panes: which tab has one waiting,
//! and the key that goes to it.

use ljos::prelude::*;
use ljos::testing::TestWindow;

struct Hof {
    engine: Engine,
    window: TestWindow,
    root: ObjectId,
}

fn opened() -> Hof {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("agents");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("hof.toml");
    // No desktop notification from a test.
    std::fs::write(&config, "[notify]\nagent-waiting = false\n").unwrap();
    hof::set_launch(hof::Launch {
        command: vec!["cat".to_owned()],
        config: Some(config),
        ..hof::Launch::default()
    });
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

    fn current(&self) -> ObjectId {
        self.engine
            .read_by_name(self.root, "current")
            .as_object()
            .unwrap()
    }

    fn report(&mut self, t: ObjectId, state: &str, name: &str) {
        self.engine
            .invoke_by_name(t, "reportAgent", &[Value::from(state), Value::from(name)])
            .unwrap();
        self.window.settle().unwrap();
    }

    fn tab_state(&self, tab: i32) -> String {
        let layout = self
            .engine
            .read_by_name(self.root, "layout")
            .as_object()
            .unwrap();
        let panes = self
            .engine
            .invoke_by_name(layout, "tabPanes", &[Value::from(tab)])
            .unwrap();
        let state = self
            .engine
            .invoke_by_name(self.root, "agentStateOf", &[panes])
            .unwrap();
        state.to_js_string().as_str().to_owned()
    }

    fn panes(&self) -> serde_json::Value {
        let text = self
            .engine
            .invoke_by_name(self.root, "mcpPanes", &[])
            .unwrap();
        serde_json::from_str(text.to_js_string().as_str()).unwrap()
    }
}

#[test]
fn the_key_to_the_next_waiting_agent_goes_to_it_across_tabs() {
    let mut hof = opened();
    let first = hof.current();
    hof.run("split-right");
    let working = hof.current();
    hof.run("new-tab");
    let elsewhere = hof.current();
    hof.report(first, "waiting", "claude");
    hof.report(working, "working", "codex");
    assert_eq!(hof.tab_state(0), "waiting", "the one most in need");
    assert_eq!(hof.tab_state(1), "none");

    hof.run("next-waiting");
    assert_eq!(hof.current(), first);
    // Nothing else waits: it stays.
    hof.run("next-waiting");
    assert_eq!(hof.current(), first);
    hof.report(elsewhere, "waiting", "");
    hof.run("next-waiting");
    assert_eq!(hof.current(), elsewhere);

    hof.report(first, "idle", "");
    assert_eq!(hof.tab_state(0), "working");
}

#[test]
fn the_mcp_server_sees_each_pane_s_agent() {
    let mut hof = opened();
    let t = hof.current();
    hof.report(t, "waiting", "claude");
    let panes = hof.panes();
    assert_eq!(panes[0]["agent"], "claude");
    assert_eq!(panes[0]["agentState"], "waiting");
}
