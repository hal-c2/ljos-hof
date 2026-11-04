//! The settings dialog changes the config file, and the window follows.
//! A test binary of its own: it writes the config file it reads.

use ljos::prelude::*;
use ljos::testing::TestWindow;

const CONFIG: &str = "\
# Mine, with my comments.
[window]
menu = \"none\"   # keys only

[font]
size = 11 # points
";

struct Hof {
    engine: Engine,
    window: TestWindow,
    root: ObjectId,
    config: std::path::PathBuf,
}

fn opened() -> Hof {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("settings");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("hof.toml");
    std::fs::write(&config, CONFIG).unwrap();
    hof::set_launch(hof::Launch {
        command: vec!["cat".to_owned()],
        config: Some(config.clone()),
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
        config,
    }
}

impl Hof {
    fn find(&self, name: &str) -> ObjectId {
        let mut items = vec![self.root];
        while let Some(item) = items.pop() {
            if self
                .engine
                .read_by_name(item, "objectName")
                .to_js_string()
                .as_str()
                == name
            {
                return item;
            }
            items.extend(self.engine.children(item));
        }
        panic!("nothing is called {name}");
    }

    fn flag(&self, obj: ObjectId, name: &str) -> bool {
        self.engine.read_by_name(obj, name).to_boolean()
    }

    fn file(&self) -> String {
        std::fs::read_to_string(&self.config).unwrap()
    }

    /// Changes a setting as a control of the dialog does.
    fn put(&mut self, section: &str, key: &str, value: Value) -> String {
        let dialog = self.find("settingsDialog");
        self.engine
            .invoke_by_name(
                dialog,
                "put",
                &[Value::from(section), Value::from(key), value],
            )
            .unwrap();
        self.window.settle().unwrap();
        self.engine
            .read_by_name(dialog, "error")
            .to_js_string()
            .as_str()
            .to_owned()
    }

    fn click(&mut self, item: ObjectId) {
        let (w, h) = (
            self.engine.read_by_name(self.root, "width").to_number(),
            self.engine.read_by_name(self.root, "height").to_number(),
        );
        self.window.resize(w, h).unwrap();
        let at = ljos::item::scene_rect(&self.engine, item).center();
        let now = self.window.now();
        self.window
            .pointer(&ljos::item::PointerEvent::press(at.x, at.y, now));
        self.window
            .pointer(&ljos::item::PointerEvent::release(at.x, at.y, now));
        self.window.settle().unwrap();
    }
}

#[test]
fn a_change_in_the_settings_is_written_into_the_file_and_followed() {
    let mut hof = opened();
    assert!(
        !hof.flag(hof.root, "tabsShown"),
        "no menu button, and one tab: no tab bar"
    );
    hof.engine
        .invoke_by_name(hof.root, "run", &[Value::from("settings")])
        .unwrap();
    hof.window.settle().unwrap();
    assert!(hof.flag(hof.root, "settingsShown"));

    // The menu moves to a button, which brings the tab bar.
    assert_eq!(hof.put("window", "menu", Value::from("button")), "");
    assert!(hof.flag(hof.root, "tabsShown"));
    assert!(hof.flag(hof.find("menuButton"), "visible"));

    assert_eq!(hof.put("font", "size", Value::from(14)), "");
    assert_eq!(
        hof.engine.read_by_name(hof.root, "fontSize").to_number(),
        14.0
    );

    // A switch, clicked.
    let bold = hof.find("settingBoldIsBright");
    assert!(!hof.flag(bold, "checked"));
    hof.click(bold);
    assert!(hof.flag(bold, "checked"));

    let file = hof.file();
    assert!(file.starts_with("# Mine, with my comments.\n"), "{file}");
    assert!(file.contains("menu = \"button\" # keys only"), "{file}");
    assert!(file.contains("size = 14 # points"), "{file}");
    assert!(file.contains("bold-is-bright = true"), "{file}");

    // What the file cannot have is turned down, and the file left alone.
    let error = hof.put("font", "no-such-key", Value::from(1));
    assert!(error.contains("no-such-key"), "{error}");
    assert_eq!(hof.file(), file);
    let shown = hof.engine.read_by_name(hof.find("settingsError"), "text");
    assert_eq!(shown.to_js_string().as_str(), error);
}
