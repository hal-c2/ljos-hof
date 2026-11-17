//! The dropdown's slide in and out, as `hof --dropdown` runs it.

use ljos::prelude::*;
use ljos::testing::TestWindow;

struct Dropdown {
    engine: Engine,
    window: TestWindow,
    root: ObjectId,
    _dir: tempfile::TempDir,
}

fn opened() -> Dropdown {
    let dir = tempfile::tempdir().unwrap();
    // The defaults, whatever the user's own config says: a slide of
    // 150 ms, and hiding when the window loses the keyboard.
    hof::set_launch(hof::Launch {
        dropdown: true,
        command: vec!["cat".to_owned()],
        config: Some(dir.path().join("none/hof.toml")),
        socket: Some(dir.path().join("dropdown.sock")),
        ..hof::Launch::default()
    });
    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    Dropdown {
        engine,
        window,
        root,
        _dir: dir,
    }
}

impl Dropdown {
    fn call(&self, function: &str) {
        self.engine
            .invoke_by_name(self.root, function, &[])
            .unwrap();
    }

    fn flag(&self, name: &str) -> bool {
        self.engine.read_by_name(self.root, name).to_boolean()
    }

    fn fraction(&self) -> f64 {
        self.engine
            .read_by_name(self.root, "shownFraction")
            .to_number()
    }

    /// Past any slide the defaults ask for.
    fn after_the_slide(&mut self) {
        for _ in 0..10 {
            self.window.advance(50).unwrap();
        }
        self.window.settle().unwrap();
    }
}

#[test]
fn the_dropdown_slides_in_when_it_starts() {
    let mut d = opened();
    d.after_the_slide();
    assert!(d.flag("visible"));
    assert_eq!(d.fraction(), 1.0);
}

#[test]
fn a_hide_before_the_slide_in_has_moved_hides_the_window() {
    let mut d = opened();
    d.after_the_slide();
    d.call("hideDropdown");
    d.after_the_slide();
    assert!(!d.flag("visible"), "hidden once it has slid out");

    // Shown again and, in the same turn, hidden again, as when the
    // window loses the keyboard before its slide has started: nothing
    // has moved in, so there is nothing to slide out.
    d.call("showDropdown");
    d.call("hideDropdown");
    d.after_the_slide();
    assert!(!d.flag("visible"), "no empty window is left on screen");
    assert!(!d.flag("hiding"), "the hide is over");

    // And `hof --toggle` shows it again, all the way in.
    d.call("toggleDropdown");
    d.after_the_slide();
    assert!(d.flag("visible"));
    assert_eq!(d.fraction(), 1.0);
}
