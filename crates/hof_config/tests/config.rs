//! What a user writing `hof.toml` gets.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hof_config::{
    Action, Binding, Chord, Config, CursorShape, Error, Keys, Permission, Place, Platform,
    Position, Program, Rgba, WarningKind, builtin, load, parse,
};

fn ok(text: &str) -> hof_config::Loaded {
    parse(text).unwrap_or_else(|e| panic!("{e}"))
}

fn chord(s: &str) -> Chord {
    s.parse().unwrap()
}

/// A fresh directory under `$TMPDIR` that is removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("hof-config-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Defaults

#[test]
fn an_empty_file_is_the_defaults() {
    let loaded = ok("");
    assert_eq!(loaded.config, Config::default());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn a_missing_file_is_the_defaults() {
    let dir = Scratch::new();
    let loaded = load(&dir.0.join("nope.toml")).unwrap();
    assert_eq!(loaded.config, Config::default());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn the_defaults_are_the_promised_ones() {
    let c = Config::default();
    assert_eq!(c.font.family, "monospace");
    assert_eq!(c.font.size, 11.0);
    assert_eq!(c.font.line_height, 1.0);
    assert!(!c.font.bold_is_bright);
    assert_eq!(c.window.padding, 6.0);
    assert_eq!(c.window.opacity, 1.0);
    assert_eq!(c.window.pane_border, 2.0);
    assert_eq!(c.window.pane_radius, 8.0);
    assert!(c.window.decorations);
    assert_eq!((c.window.width, c.window.height), (100, 30));
    assert_eq!(c.window.title, "Hof");
    assert_eq!(c.terminal.shell, None);
    assert_eq!(c.terminal.scrollback, 10_000);
    assert!(c.terminal.cursor_blink && c.terminal.copy_on_select);
    assert!(c.terminal.confirm_unsafe_paste);
    assert_eq!(c.terminal.clipboard_read, Permission::Ask);
    assert!(c.terminal.clipboard_write);
    assert_eq!(c.terminal.open_urls, Permission::Ask);
    assert_eq!(c.terminal.cursor_shape, CursorShape::Block);
    assert_eq!(c.theme.name, "hof-dark");
    assert_eq!(c.dropdown.height, 0.4);
    assert_eq!(c.dropdown.width, 1.0);
    assert_eq!(c.dropdown.position, Position::Top);
    assert!(c.dropdown.hide_on_focus_loss);
    assert_eq!(c.dropdown.animation_ms, 150);
    assert!(!c.session.restore && c.session.save_scrollback && c.session.keep_running);
    assert_eq!(
        (c.session.host.as_ref(), c.session.name.as_ref()),
        (None, None)
    );
    assert!(!c.mcp.enabled && c.mcp.socket.is_none());
    assert!(c.notify.agent_waiting);
}

#[test]
fn the_example_file_is_exactly_the_defaults() {
    let text = include_str!("../hof.example.toml");
    let loaded = ok(text);
    assert_eq!(loaded.warnings, vec![]);
    assert_eq!(loaded.config, Config::default());
}

/// The `[keys]` entries the example lists after `prefix`, as a file of
/// their own.
fn listed_keys(prefix: &str) -> String {
    let text = include_str!("../hof.example.toml");
    let section = text
        .split("[keys]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    let mut uncommented = String::from("[keys]\n");
    for line in section.lines() {
        let entry = line.strip_prefix(prefix).and_then(|l| {
            let (name, _) = l.split_once(" = ")?;
            name.parse::<Action>().is_ok().then_some(l)
        });
        if let Some(entry) = entry {
            uncommented.push_str(entry);
            uncommented.push('\n');
        }
    }
    uncommented
}

#[test]
fn the_key_defaults_listed_in_the_example_are_the_real_ones() {
    for (prefix, platform) in [("# ", Platform::Other), ("#   ", Platform::MacOs)] {
        let loaded = ok(&listed_keys(prefix));
        assert_eq!(loaded.warnings, vec![], "{platform:?}");
        let keys = loaded.config.keys.on(platform);
        let listed: BTreeSet<_> = keys.bindings().into_iter().collect();
        let real: BTreeSet<_> = Keys::defaults_on(platform).into_iter().collect();
        assert_eq!(listed, real, "{platform:?}");
        assert_eq!(keys.overrides.len(), Action::all().count(), "{platform:?}");
    }
}

// Every key

#[test]
fn every_key_is_read_into_its_field() {
    let loaded = ok(r##"
        [font]
        family = "Iosevka"
        size = 13.5
        line-height = 1.2
        bold-is-bright = true

        [window]
        padding = 2
        opacity = 0.9
        pane-border = 3
        pane-radius = 0
        decorations = false
        width = 120
        height = 40
        title = "Shrine"
        menu = "none"

        [terminal]
        shell = "/usr/bin/fish"
        args = ["-l"]
        scrollback = 500
        cursor-blink = false
        cursor-shape = "underline"
        copy-on-select = false
        confirm-unsafe-paste = false
        clipboard-read = "allow"
        clipboard-write = false
        open-urls = "deny"
        working-directory = "/srv"
        env = { EDITOR = "nvim", PAGER = "less" }

        [theme]
        name = "nord"
        foreground = "#010203"
        background = "#040506"
        cursor = "#07080980"
        selection = "#0a0b0c"
        palette = ["#000001", "#000002"]

        [dropdown]
        height = 0.5
        width = 0.8
        position = "bottom"
        hide-on-focus-loss = false
        animation-ms = 0

        [keys]
        new-tab = "ctrl+t"

        [session]
        restore = true
        save-scrollback = false
        keep-running = false
        host = "desktop"
        name = "work"

        [notify]
        agent-waiting = false

        [mcp]
        enabled = true
        socket = "/run/hof.sock"
        allow-input = true

        [commands.top]
        run = ["htop"]
        in = "split-down"
        description = "What is running"
        keys = "ctrl+alt+h"

        [commands.hello]
        send = "echo hello\n"

        [hooks]
        bell = "notify-send ding"
        command-finished = ["one", "two"]

        [plugins]
        disabled = ["noisy"]

        [hosts.dev]
        ssh = "me@dev.example.com"
        command = ["nc", "dev", "9"]
        hof = "~/bin/hof"
        x11 = true
        sound = true
        files = true
        clipboard = "deny"
        session = "build"
    "##);
    assert_eq!(loaded.warnings, vec![]);
    let c = loaded.config;
    assert_eq!(c.font.family, "Iosevka");
    assert_eq!(c.font.size, 13.5);
    assert_eq!(c.font.line_height, 1.2);
    assert!(c.font.bold_is_bright);
    assert_eq!(c.window.padding, 2.0);
    assert_eq!(c.window.opacity, 0.9);
    assert_eq!(c.window.pane_border, 3.0);
    assert_eq!(c.window.pane_radius, 0.0);
    assert!(!c.window.decorations);
    assert_eq!((c.window.width, c.window.height), (120, 40));
    assert_eq!(c.window.title, "Shrine");
    assert_eq!(c.window.menu, hof_config::Menu::None);
    assert_eq!(c.terminal.shell.as_deref(), Some("/usr/bin/fish"));
    assert_eq!(c.terminal.args, ["-l"]);
    assert_eq!(c.terminal.scrollback, 500);
    assert!(!c.terminal.cursor_blink);
    assert_eq!(c.terminal.cursor_shape, CursorShape::Underline);
    assert!(!c.terminal.copy_on_select);
    assert!(!c.terminal.confirm_unsafe_paste);
    assert_eq!(c.terminal.clipboard_read, Permission::Allow);
    assert!(!c.terminal.clipboard_write);
    assert_eq!(c.terminal.open_urls, Permission::Deny);
    assert_eq!(
        c.terminal.working_directory.as_deref(),
        Some(Path::new("/srv"))
    );
    assert_eq!(c.terminal.env["EDITOR"], "nvim");
    assert_eq!(c.terminal.env["PAGER"], "less");
    assert_eq!(c.theme.name, "nord");
    assert_eq!(c.theme.foreground, Some(Rgba::rgb(0x010203)));
    assert_eq!(c.theme.background, Some(Rgba::rgb(0x040506)));
    assert_eq!(
        c.theme.cursor,
        Some(Rgba {
            r: 7,
            g: 8,
            b: 9,
            a: 0x80
        })
    );
    assert_eq!(c.theme.selection, Some(Rgba::rgb(0x0a0b0c)));
    assert_eq!(c.theme.palette, [Rgba::rgb(1), Rgba::rgb(2)]);
    assert_eq!(c.dropdown.height, 0.5);
    assert_eq!(c.dropdown.width, 0.8);
    assert_eq!(c.dropdown.position, Position::Bottom);
    assert!(!c.dropdown.hide_on_focus_loss);
    assert_eq!(c.dropdown.animation_ms, 0);
    assert_eq!(c.keys.overrides[&Action::NewTab], [chord("ctrl+t")]);
    assert!(c.session.restore && !c.session.save_scrollback && !c.session.keep_running);
    assert_eq!(c.session.host.as_deref(), Some("desktop"));
    assert_eq!(c.session.name.as_deref(), Some("work"));
    assert!(!c.notify.agent_waiting);
    assert!(c.mcp.enabled);
    assert_eq!(c.mcp.socket.as_deref(), Some(Path::new("/run/hof.sock")));
    assert!(c.mcp.allow_input && !c.mcp.allow_spawn);
    let top = &c.commands["top"];
    assert_eq!(
        top.run.as_ref().map(Program::argv),
        Some(vec!["htop".into()])
    );
    assert_eq!(top.place, Place::SplitDown);
    assert_eq!(top.description, "What is running");
    assert_eq!(top.keys, Some(vec![chord("ctrl+alt+h")]));
    assert_eq!(c.commands["hello"].send.as_deref(), Some("echo hello\n"));
    assert_eq!(runs(c.hooks.of("bell")), ["notify-send ding"]);
    assert_eq!(runs(c.hooks.of("command-finished")), ["one", "two"]);
    assert_eq!(c.plugins.disabled, ["noisy"]);
    let dev = &c.hosts["dev"];
    assert_eq!(dev.ssh, "me@dev.example.com");
    assert_eq!(dev.command, ["nc", "dev", "9"]);
    assert_eq!(dev.hof, "~/bin/hof");
    assert!(dev.x11 && dev.sound && dev.files);
    assert_eq!(dev.clipboard, Permission::Deny);
    assert_eq!(dev.session.as_deref(), Some("build"));
}

#[test]
fn a_hosts_user_and_port_go_to_ssh_before_its_options() {
    let c = ok(r#"
        [hosts.dev]
        ssh = "dev.example.com"
        user = "olafur"
        port = 2222
        ssh-options = ["-o", "ForwardAgent=yes"]
    "#)
    .config;
    let dev = &c.hosts["dev"];
    assert_eq!((dev.user.as_str(), dev.port), ("olafur", Some(2222)));
    assert_eq!(
        dev.ssh_argv("dev"),
        [
            "ssh",
            "-T",
            "-l",
            "olafur",
            "-p",
            "2222",
            "-o",
            "ForwardAgent=yes",
            "dev.example.com"
        ]
    );
}

#[test]
fn a_host_is_reached_through_ssh_unless_it_names_another_way() {
    let c = ok(r#"
        [hosts.box]
        [hosts.dev]
        ssh = "me@dev.example.com"
        ssh-options = ["-p", "2222"]
        hof = "/opt/hof"
        bootstrap = false
        [hosts.local]
        command = ["hof", "agent", "--stdio"]
    "#)
    .config;
    assert!(c.hosts["box"].bootstraps());
    assert!(!c.hosts["dev"].bootstraps(), "turned off");
    assert!(!c.hosts["local"].bootstraps(), "not over ssh");
    let box_ = &c.hosts["box"];
    assert_eq!(
        box_.transport("box"),
        ["ssh", "-T", "box", "--", "hof", "agent", "--stdio"]
    );
    assert!(!box_.x11 && !box_.sound && !box_.files);
    assert_eq!(box_.clipboard, Permission::Ask);
    assert_eq!(
        c.hosts["dev"].transport("dev"),
        [
            "ssh",
            "-T",
            "-p",
            "2222",
            "me@dev.example.com",
            "--",
            "/opt/hof",
            "agent",
            "--stdio"
        ]
    );
    assert_eq!(
        c.hosts["local"].transport("local"),
        ["hof", "agent", "--stdio"]
    );
}

#[test]
fn an_unknown_key_in_a_host_is_a_warning() {
    let loaded = ok("[hosts.dev]
x12 = true");
    assert_eq!(loaded.warnings.len(), 1);
    assert!(
        loaded.warnings[0].to_string().contains("hosts.dev.x12"),
        "{}",
        loaded.warnings[0]
    );
}

fn runs(hooks: &[hof_config::Hook]) -> Vec<&str> {
    hooks.iter().map(|h| h.run.as_str()).collect()
}

#[test]
fn whole_numbers_are_fine_where_fractions_are_expected() {
    let c = ok("font.size = 12\nwindow.opacity = 1").config;
    assert_eq!(c.font.size, 12.0);
    assert_eq!(c.window.opacity, 1.0);
}

#[test]
fn a_section_can_be_written_inline_or_with_dotted_keys() {
    let inline = ok("font = { size = 14.0 }").config;
    let dotted = ok("font.size = 14.0").config;
    assert_eq!(inline, dotted);
    assert_eq!(inline.font.size, 14.0);
    assert_eq!(inline.font.family, "monospace");
}

#[test]
fn a_tilde_in_the_working_directory_means_home() {
    let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) else {
        return;
    };
    let c = ok(r#"terminal.working-directory = "~/src""#).config;
    assert_eq!(
        c.terminal.working_directory,
        Some(PathBuf::from(home).join("src"))
    );
}

// Warnings

#[test]
fn an_unknown_key_is_a_warning_with_its_path_and_line() {
    let loaded = ok("[font]\nsize = 12\nsise = 13\n");
    assert_eq!(loaded.config.font.size, 12.0);
    let [w] = &loaded.warnings[..] else {
        panic!("{:?}", loaded.warnings)
    };
    assert_eq!(w.key, "font.sise");
    assert_eq!(w.line, Some(3));
    assert_eq!(w.kind, WarningKind::UnknownKey);
}

#[test]
fn an_unknown_section_is_one_warning_not_one_per_key() {
    let loaded = ok("[future]\na = 1\nb = 2\n");
    let keys: Vec<_> = loaded.warnings.iter().map(|w| w.key.as_str()).collect();
    assert_eq!(keys, ["future"]);
}

#[test]
fn environment_variables_are_never_unknown_keys() {
    let loaded = ok("[terminal.env]\nANYTHING_AT_ALL = \"1\"\n");
    assert!(loaded.warnings.is_empty());
}

#[test]
fn warnings_name_the_file_they_came_from() {
    let dir = Scratch::new();
    let path = dir.write("hof.toml", "colour = 1\n");
    let loaded = load(&path).unwrap();
    assert_eq!(loaded.warnings[0].path.as_deref(), Some(path.as_path()));
    let shown = loaded.warnings[0].to_string();
    assert!(
        shown.starts_with(&format!("{}:1:", path.display())),
        "{shown}"
    );
}

#[test]
fn out_of_range_numbers_are_brought_back_with_a_warning() {
    let loaded =
        ok("window.opacity = 1.5\ndropdown.height = 0\nfont.size = -3\nwindow.width = 0\n");
    let c = &loaded.config;
    assert_eq!(c.window.opacity, 1.0);
    assert_eq!(c.dropdown.height, 0.4);
    assert_eq!(c.font.size, 11.0);
    assert_eq!(c.window.width, 100);
    let mut keys: Vec<_> = loaded
        .warnings
        .iter()
        .map(|w| (w.key.as_str(), w.line))
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            ("dropdown.height", Some(2)),
            ("font.size", Some(3)),
            ("window.opacity", Some(1)),
            ("window.width", Some(4)),
        ]
    );
    assert!(matches!(
        loaded.warnings[0].kind,
        WarningKind::OutOfRange { .. }
    ));
}

#[test]
fn a_palette_longer_than_sixteen_is_cut_with_a_warning() {
    let colours = vec!["\"#000000\""; 17].join(", ");
    let loaded = ok(&format!("theme.palette = [{colours}]"));
    assert_eq!(loaded.config.theme.palette.len(), 16);
    assert_eq!(
        loaded.warnings[0].kind,
        WarningKind::PaletteTooLong { len: 17 }
    );
}

// Errors

#[test]
fn malformed_toml_is_an_error_with_line_and_column() {
    let err = parse("[font]\nsize = = 3\n").unwrap_err();
    let Error::Parse {
        path, line, column, ..
    } = err
    else {
        panic!("{err}")
    };
    assert_eq!(path, None);
    assert_eq!(line, 2);
    assert!(column > 1);
}

#[test]
fn a_value_of_the_wrong_type_is_an_error_naming_the_key() {
    let err = parse("[font]\nfamily = \"x\"\nsize = \"big\"\n").unwrap_err();
    let Error::Parse { line, message, .. } = &err else {
        panic!("{err}")
    };
    assert_eq!(*line, 3);
    assert!(message.contains("`font.size`"), "{message}");
}

#[test]
fn a_bad_colour_is_an_error_naming_the_key() {
    let err = parse("[theme]\nforeground = \"#zzzzzz\"\n").unwrap_err();
    let Error::Parse { line, message, .. } = &err else {
        panic!("{err}")
    };
    assert_eq!(*line, 2);
    assert!(message.contains("`theme.foreground`"), "{message}");
    assert!(message.contains("#zzzzzz"), "{message}");
}

#[test]
fn a_bad_palette_entry_is_an_error_naming_the_palette_and_the_entry() {
    let err = parse("theme.palette = [\"#000000\", \"red\"]").unwrap_err();
    let shown = err.to_string();
    assert!(
        shown.contains("`theme.palette`") && shown.contains("`red`"),
        "{shown}"
    );
}

#[test]
fn an_unknown_cursor_shape_is_an_error() {
    let err = parse("terminal.cursor-shape = \"beam\"").unwrap_err();
    assert!(err.to_string().contains("terminal.cursor-shape"), "{err}");
}

#[test]
fn errors_from_a_file_name_the_file() {
    let dir = Scratch::new();
    let path = dir.write("hof.toml", "font = 3\n");
    let err = load(&path).unwrap_err();
    assert!(
        err.to_string()
            .starts_with(&format!("{}:1:", path.display())),
        "{err}"
    );
}

#[test]
fn a_file_that_cannot_be_read_is_an_error() {
    let dir = Scratch::new();
    // A directory exists but is not a readable file.
    let err = load(&dir.0).unwrap_err();
    assert!(matches!(err, Error::Read { .. }), "{err}");
}

// Keys

#[test]
fn a_key_entry_replaces_the_default_for_that_action() {
    let keys = ok("[keys]\nnew-tab = \"ctrl+alt+t\"\n")
        .config
        .keys
        .on(Platform::Other);
    assert_eq!(keys.action_for(&chord("ctrl+alt+t")), Some(Action::NewTab));
    assert_eq!(keys.action_for(&chord("ctrl+shift+t")), None);
    assert_eq!(
        keys.action_for(&chord("ctrl+shift+w")),
        Some(Action::ClosePane)
    );
}

#[test]
fn none_unbinds_an_action() {
    let keys = ok("[keys]\nsearch = \"none\"\n").config.keys;
    assert!(keys.bindings().iter().all(|(_, a)| *a != Action::Search));
    assert_eq!(keys.action_for(&chord("ctrl+shift+f")), None);
}

#[test]
fn an_action_can_have_several_chords() {
    let keys = ok("[keys]\ncopy = [\"ctrl+shift+c\", \"ctrl+insert\"]\n")
        .config
        .keys;
    assert_eq!(keys.action_for(&chord("ctrl+insert")), Some(Action::Copy));
    assert_eq!(keys.action_for(&chord("ctrl+shift+c")), Some(Action::Copy));
}

#[test]
fn an_unknown_action_is_a_warning() {
    let loaded = ok("[keys]\nlaunch-rockets = \"ctrl+r\"\n");
    assert_eq!(loaded.config.keys, Keys::default());
    let [w] = &loaded.warnings[..] else {
        panic!("{:?}", loaded.warnings)
    };
    assert_eq!(w.key, "keys.launch-rockets");
    assert_eq!(w.kind, WarningKind::UnknownAction);
    assert_eq!(w.line, Some(2));
}

#[test]
fn a_bad_chord_is_a_warning_and_the_default_stays() {
    let loaded = ok("[keys]\nnew-tab = \"ctrl+shoft+t\"\n");
    assert_eq!(
        loaded
            .config
            .keys
            .clone()
            .on(Platform::Other)
            .action_for(&chord("ctrl+shift+t")),
        Some(Action::NewTab)
    );
    assert!(
        matches!(&loaded.warnings[0].kind, WarningKind::BadChord { chord, .. } if chord == "ctrl+shoft+t")
    );
}

#[test]
fn a_binding_that_is_not_text_is_a_warning() {
    let loaded = ok("[keys]\nnew-tab = 5\n");
    assert_eq!(loaded.warnings[0].kind, WarningKind::BadBinding);
    assert_eq!(loaded.config.keys, Keys::default());
}

#[test]
fn one_chord_for_two_actions_is_a_warning_and_the_earlier_action_keeps_it() {
    let loaded = ok("[keys]\nsearch = \"ctrl+f\"\nnew-tab = \"ctrl+f\"\n");
    let keys = loaded.config.keys.clone().on(Platform::Other);
    assert_eq!(keys.action_for(&chord("ctrl+f")), Some(Action::NewTab));
    // Search lost its only chord to the conflict, so it keeps its default.
    assert_eq!(
        keys.action_for(&chord("ctrl+shift+f")),
        Some(Action::Search)
    );
    let [w] = &loaded.warnings[..] else {
        panic!("{:?}", loaded.warnings)
    };
    assert_eq!(w.key, "keys.search");
    assert_eq!(w.line, Some(2));
}

#[test]
fn keys_must_be_a_table() {
    assert!(parse("keys = \"ctrl+t\"").is_err());
}

// Themes

#[test]
fn the_default_theme_is_hof_dark() {
    let (theme, warnings) = Config::default().theme(Path::new("/nonexistent")).unwrap();
    assert_eq!(&theme, builtin("hof-dark").unwrap());
    assert!(warnings.is_empty());
}

#[test]
fn a_built_in_theme_is_found_by_name() {
    let c = ok("theme.name = \"dracula\"").config;
    let (theme, _) = c.theme(Path::new("/nonexistent")).unwrap();
    assert_eq!(theme.background, Rgba::rgb(0x282a36));
}

#[test]
fn overrides_are_laid_over_the_named_theme() {
    let c =
        ok("[theme]\nname = \"nord\"\nbackground = \"#000000\"\npalette = [\"#ff0000\"]").config;
    let (theme, _) = c.theme(Path::new("/nonexistent")).unwrap();
    let nord = builtin("nord").unwrap();
    assert_eq!(theme.background, Rgba::rgb(0));
    assert_eq!(theme.foreground, nord.foreground);
    assert_eq!(theme.palette[0], Rgba::rgb(0xff0000));
    assert_eq!(theme.palette[1], nord.palette[1]);
}

#[test]
fn an_unknown_theme_name_warns_and_falls_back_to_hof_dark() {
    let c = ok("theme.name = \"no-such-theme\"").config;
    let (theme, warnings) = c.theme(Path::new("/nonexistent")).unwrap();
    assert_eq!(&theme, builtin("hof-dark").unwrap());
    assert_eq!(
        warnings[0].kind,
        WarningKind::UnknownTheme {
            name: "no-such-theme".into()
        }
    );
}

#[test]
fn a_theme_file_starts_from_hof_dark_and_changes_what_it_lists() {
    let dir = Scratch::new();
    dir.write("themes/ember.toml", "background = \"#200000\"\n");
    let c = ok("theme.name = \"ember\"").config;
    let (theme, warnings) = c.theme(&dir.0.join("themes")).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(theme.background, Rgba::rgb(0x200000));
    assert_eq!(theme.foreground, builtin("hof-dark").unwrap().foreground);
}

#[test]
fn a_theme_file_shadows_a_built_in_of_the_same_name() {
    let dir = Scratch::new();
    dir.write("nord.toml", "foreground = \"#123456\"\n");
    let c = ok("theme.name = \"nord\"").config;
    let (theme, _) = c.theme(&dir.0).unwrap();
    assert_eq!(theme.foreground, Rgba::rgb(0x123456));
}

#[test]
fn the_config_overrides_win_over_a_theme_file() {
    let dir = Scratch::new();
    dir.write("ember.toml", "background = \"#200000\"\n");
    let c = ok("[theme]\nname = \"ember\"\nbackground = \"#000020\"").config;
    let (theme, _) = c.theme(&dir.0).unwrap();
    assert_eq!(theme.background, Rgba::rgb(0x000020));
}

#[test]
fn unknown_keys_in_a_theme_file_are_warnings_naming_that_file() {
    let dir = Scratch::new();
    let path = dir.write("ember.toml", "background = \"#200000\"\nglow = true\n");
    let c = ok("theme.name = \"ember\"").config;
    let (_, warnings) = c.theme(&dir.0).unwrap();
    let [w] = &warnings[..] else {
        panic!("{warnings:?}")
    };
    assert_eq!(w.key, "glow");
    assert_eq!(w.line, Some(2));
    assert_eq!(w.path.as_deref(), Some(path.as_path()));
}

#[test]
fn a_bad_colour_in_a_theme_file_is_an_error_naming_the_file_and_key() {
    let dir = Scratch::new();
    let path = dir.write("ember.toml", "cursor = \"orange\"\n");
    let c = ok("theme.name = \"ember\"").config;
    let err = c.theme(&dir.0).unwrap_err();
    let Error::Parse {
        path: Some(p),
        message,
        ..
    } = &err
    else {
        panic!("{err}")
    };
    assert_eq!(p, &path);
    assert!(message.contains("`cursor`"), "{message}");
}

// Commands

#[test]
fn a_command_s_program_is_a_shell_snippet_or_a_program_and_its_arguments() {
    let c = ok(r#"
        [commands.a]
        run = "git status | less"
        [commands.b]
        run = ["git", "status"]
    "#)
    .config;
    let argv = |name: &str| c.commands[name].run.as_ref().unwrap().argv();
    assert_eq!(argv("a"), ["sh", "-c", "git status | less"]);
    assert_eq!(argv("b"), ["git", "status"]);
    assert_eq!(c.commands["a"].place, Place::Tab, "a tab unless it says");
}

#[test]
fn a_command_that_does_nothing_or_two_things_is_left_out_with_a_warning() {
    let loaded = ok(r#"
        [commands.nothing]
        keys = "ctrl+alt+n"
        [commands.both]
        run = "ls"
        send = "ls"
        [commands.empty]
        run = []
    "#);
    assert!(loaded.config.commands.is_empty());
    let mut keys: Vec<_> = loaded
        .warnings
        .iter()
        .map(|w| {
            assert!(matches!(w.kind, WarningKind::BadCommand { .. }), "{w}");
            w.key.as_str()
        })
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["commands.both", "commands.empty", "commands.nothing"]
    );
}

#[test]
fn an_unknown_key_in_a_command_is_a_warning() {
    let loaded = ok("[commands.a]\nrun = \"ls\"\nplace = \"tab\"\n");
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].key, "commands.a.place");
    assert_eq!(loaded.warnings[0].kind, WarningKind::UnknownKey);
}

#[test]
fn a_command_s_chord_is_its_own_and_takes_the_place_of_a_default() {
    let c = ok(r#"
        [commands.tab]
        run = "htop"
        keys = ["ctrl+shift+t", "ctrl+alt+t"]
    "#)
    .config;
    let c = Config {
        keys: c.keys.on(Platform::Other),
        ..c
    };
    let bound = |chord_text: &str| {
        c.bindings()
            .into_iter()
            .find(|(c, _)| *c == chord(chord_text))
            .map(|(_, b)| b)
    };
    assert_eq!(bound("ctrl+shift+t"), Some(Binding::Command("tab".into())));
    assert_eq!(bound("ctrl+alt+t"), Some(Binding::Command("tab".into())));
    assert_eq!(Binding::Command("tab".into()).name(), "command:tab");
    // new-tab lost its default chord and has no other.
    assert!(
        !c.bindings()
            .iter()
            .any(|(_, b)| *b == Binding::Action(Action::NewTab))
    );
}

#[test]
fn a_chord_the_user_gave_an_action_stays_with_the_action() {
    let loaded = ok(r#"
        [keys]
        copy = "ctrl+alt+c"
        [commands.clip]
        run = "xclip -o"
        in = "background"
        keys = "ctrl+alt+c"
    "#);
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert_eq!(loaded.warnings[0].key, "commands.clip.keys");
    assert!(matches!(
        &loaded.warnings[0].kind,
        WarningKind::ChordConflict { kept, .. } if kept == "copy"
    ));
    let c = loaded.config;
    assert_eq!(c.commands["clip"].keys, Some(vec![]));
    assert!(
        c.bindings()
            .contains(&(chord("ctrl+alt+c"), Binding::Action(Action::Copy)))
    );
}

#[test]
fn a_bad_chord_in_a_command_is_a_warning_and_the_command_stays() {
    let loaded = ok("[commands.a]\nrun = \"ls\"\nkeys = \"ctrl+nope\"\n");
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].line, Some(3));
    assert!(matches!(
        loaded.warnings[0].kind,
        WarningKind::BadChord { .. }
    ));
    assert!(loaded.config.commands.contains_key("a"));
}

// Hooks

#[test]
fn an_event_hof_does_not_have_is_an_unknown_key() {
    let loaded = ok("[hooks]\nbel = \"x\"\n");
    assert_eq!(loaded.warnings[0].key, "hooks.bel");
    assert_eq!(loaded.warnings[0].kind, WarningKind::UnknownKey);
}

// Plugins

#[test]
fn plugins_beside_the_config_bring_commands_and_hooks() {
    let dir = Scratch::new();
    let config = dir.write(
        "hof.toml",
        "[hooks]\nbell = \"mine\"\n[commands.git]\nkeys = \"ctrl+alt+g\"\n",
    );
    dir.write(
        "plugins/git/plugin.toml",
        r#"
        description = "Git at a key"
        [commands.git]
        run = ["sh", "status.sh"]
        keys = "ctrl+alt+s"
        [hooks]
        bell = "theirs"
        "#,
    );
    dir.write("plugins/top.toml", "[commands.top]\nrun = \"htop\"\n");
    let c = load(&config).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.warnings, vec![]);
    let c = c.config;

    let git = &c.commands["git"];
    assert_eq!(git.run.as_ref().unwrap().argv(), ["sh", "status.sh"]);
    let plugin = git.plugin.as_ref().unwrap();
    assert_eq!(plugin.name, "git");
    assert_eq!(
        plugin.dir,
        dir.0.join("plugins/git"),
        "its scripts are beside it"
    );
    assert_eq!(
        git.keys,
        Some(vec![chord("ctrl+alt+g")]),
        "the user's entry rebinds it"
    );
    assert_eq!(
        c.commands["top"].plugin.as_ref().unwrap().dir,
        dir.0.join("plugins")
    );

    assert_eq!(runs(c.hooks.of("bell")), ["mine", "theirs"]);
    assert_eq!(c.hooks.of("bell")[1].plugin.as_ref().unwrap().name, "git");
}

#[test]
fn a_plugin_cannot_take_the_user_s_command_or_a_chord() {
    let dir = Scratch::new();
    let config = dir.write(
        "hof.toml",
        "[commands.top]\nrun = \"top\"\nkeys = \"ctrl+alt+t\"\n",
    );
    let plugin = dir.write(
        "plugins/p.toml",
        "[commands.top]\nrun = \"htop\"\n[commands.other]\nrun = \"x\"\nkeys = \"ctrl+alt+t\"\n",
    );
    let loaded = load(&config).unwrap();
    let c = &loaded.config;
    assert_eq!(c.commands["top"].run, Some(Program::Shell("top".into())));
    assert_eq!(c.commands["other"].keys, Some(vec![]));
    assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
    for w in &loaded.warnings {
        assert_eq!(w.path.as_deref(), Some(plugin.as_path()), "{w}");
    }
    assert!(matches!(
        &loaded.warnings[0].kind,
        WarningKind::CommandConflict { kept } if kept == "the config"
    ));
    assert!(matches!(
        &loaded.warnings[1].kind,
        WarningKind::ChordConflict { kept, .. } if kept == "command:top"
    ));
}

#[test]
fn a_disabled_plugin_is_not_read_and_a_broken_one_is_a_warning() {
    let dir = Scratch::new();
    let config = dir.write("hof.toml", "[plugins]\ndisabled = [\"off\"]\n");
    dir.write("plugins/off.toml", "[commands.off]\nrun = \"x\"\n");
    let broken = dir.write("plugins/broken.toml", "[commands.b\n");
    let loaded = load(&config).unwrap();
    assert!(loaded.config.commands.is_empty());
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].path.as_deref(), Some(broken.as_path()));
    assert_eq!(loaded.warnings[0].line, Some(1));
    assert!(matches!(
        loaded.warnings[0].kind,
        WarningKind::BadPlugin { .. }
    ));
}

#[test]
fn hof_knows_the_common_agents_and_a_table_of_the_user_s_adds_one() {
    let c = ok("[agents.goose]\nwaiting = [\"Proceed?\"]\n").config;
    let agents = c.agents();
    assert!(agents.contains_key("claude"));
    let goose = &agents["goose"];
    assert_eq!(goose.processes("goose"), ["goose"]);
    assert_eq!(goose.waiting, ["Proceed?"]);
}

#[test]
fn a_plugin_brings_an_agent_table_but_the_user_s_of_its_name_stays() {
    let dir = Scratch::new();
    let config = dir.write("hof.toml", "[agents.claude]\nworking = [\"mine\"]\n");
    dir.write(
        "plugins/agents.toml",
        "[agents.claude]\nworking = [\"theirs\"]\n[agents.goose]\nprocess = [\"goose-cli\"]\n",
    );
    let c = load(&config).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.warnings, vec![]);
    let agents = c.config.agents();
    assert_eq!(agents["claude"].working, ["mine"]);
    assert_eq!(agents["goose"].processes("goose"), ["goose-cli"]);
}

#[test]
fn an_agent_hook_and_the_key_to_the_next_waiting_agent_are_there() {
    let c = ok("[hooks]\nagent-changed = \"echo $HOF_AGENT_STATE\"\n").config;
    assert_eq!(c.hooks.of("agent-changed").len(), 1);
    assert_eq!(
        Keys::default()
            .on(Platform::Other)
            .action_for(&chord("ctrl+shift+a")),
        Some(Action::NextWaiting)
    );
}
