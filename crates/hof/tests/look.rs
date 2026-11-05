//! How the settings window, the menus and the host picker look: each drawn, and held to
//! the Ljós design audit (contrast, names, targets, text that is cut off).
//! A test binary of its own: it writes the config file it reads.
//! `HOF_PICTURES=dir` keeps a PNG of each page there.

use ljos::prelude::*;

const CONFIG: &str = "\
[hosts.dev]
ssh = \"dev.example.com\"
user = \"olafur\"
port = 2222
x11 = true

[hosts.box]
";

fn find(engine: &Engine, root: ObjectId, name: &str) -> ObjectId {
    let mut items = vec![root];
    while let Some(item) = items.pop() {
        if engine
            .read_by_name(item, "objectName")
            .to_js_string()
            .as_str()
            == name
        {
            return item;
        }
        items.extend(engine.children(item));
    }
    panic!("nothing is called {name}");
}

#[test]
fn the_settings_the_menus_and_the_host_picker_pass_the_design_audit() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("look");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("hof.toml");
    std::fs::write(&config, CONFIG).unwrap();
    hof::set_launch(hof::Launch {
        command: vec!["cat".to_owned()],
        config: Some(config),
        ..hof::Launch::default()
    });
    let (engine, root) = hof::app().build().expect("the window builds");
    let picture = ljos::render::Picture::new(1000, 640).settle(400);
    picture.install(&engine);
    // The window as large as the picture, not the size its cells ask.
    engine
        .write_by_name(root, "width", Value::from(1000.0))
        .unwrap();
    engine
        .write_by_name(root, "height", Value::from(640.0))
        .unwrap();
    let dialog = find(&engine, root, "settingsDialog");
    let pictures = std::env::var_os("HOF_PICTURES").map(std::path::PathBuf::from);

    let mut failed = String::new();
    let mut look = |name: &str| {
        if let Some(out) = &pictures {
            let png = picture.png(&engine, root).expect("the window draws");
            std::fs::write(out.join(format!("{name}.png")), png).unwrap();
        }
        let mut scene = picture.scene(&engine, root).expect("the window lays out");
        let options = ljos_audit::Options {
            // Pressing things would write the config file as it goes.
            act: false,
            ..ljos_audit::Options::desktop()
        };
        let report = ljos_audit::audit(&engine, &mut scene, picture.now(), &options);
        let errors = report.count(ljos_audit::Severity::Error);
        let warnings = report.count(ljos_audit::Severity::Warning);
        eprintln!(
            "== {name}: {errors} error(s), {warnings} warning(s)\n{}",
            report.by_rule("")
        );
        if errors > 0 {
            failed.push_str(&format!("{name}: {errors} error(s)\n"));
        }
    };
    // Panes of a split, each framed, the focused one's frame bright.
    for action in ["split-right", "split-down"] {
        engine
            .invoke_by_name(root, "run", &[Value::from(action)])
            .unwrap();
    }
    look("splits");

    engine
        .invoke_by_name(root, "run", &[Value::from("settings")])
        .unwrap();
    for (page, name) in ["appearance", "terminal", "window", "hosts"]
        .iter()
        .enumerate()
    {
        engine
            .write_by_name(dialog, "page", Value::from(page as f64))
            .unwrap();
        look(&format!("settings-{name}"));
    }
    engine.invoke_by_name(dialog, "close", &[]).unwrap();

    let menu = find(&engine, root, "windowMenu");
    engine
        .invoke_by_name(menu, "popup", &[Value::from(600.0), Value::from(30.0)])
        .unwrap();
    look("window-menu");
    engine.invoke_by_name(menu, "close", &[]).unwrap();

    let menu = find(&engine, root, "paneMenu");
    engine
        .invoke_by_name(menu, "popup", &[Value::from(80.0), Value::from(80.0)])
        .unwrap();
    look("pane-menu");
    engine.invoke_by_name(menu, "close", &[]).unwrap();

    engine
        .invoke_by_name(root, "run", &[Value::from("new-tab-on-host")])
        .unwrap();
    look("host-picker");

    assert!(failed.is_empty(), "{failed}");
}
