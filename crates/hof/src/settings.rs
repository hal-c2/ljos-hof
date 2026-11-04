//! `Settings`: the config file as the QML reads it. Read when the window
//! opens, and again whenever the file (or a theme beside it) is saved.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hof_config::{Chord, Position, Rgba};
use ljos::prelude::*;

/// What the config file says, with every default filled in.
#[derive(Debug, QmlObject)]
#[qml(module = "Hof.App", name = "Settings")]
#[qml_singleton]
pub struct Settings {
    /// The file read, whether or not it exists.
    #[qml(notify, readonly)]
    pub path: String,

    #[qml(notify, readonly)]
    pub font_family: String,
    /// In points.
    #[qml(notify, readonly)]
    pub font_size: f64,
    #[qml(notify, readonly)]
    pub bold_is_bright: bool,
    #[qml(notify, readonly)]
    pub padding: f64,
    /// Of the window's background, 0 to 1.
    #[qml(notify, readonly)]
    pub opacity: f64,
    #[qml(notify, readonly)]
    pub decorations: bool,
    #[qml(notify, readonly)]
    pub title: String,
    /// The window's first size, in cells.
    #[qml(notify, readonly)]
    pub columns: i32,
    #[qml(notify, readonly)]
    pub rows: i32,

    #[qml(notify, readonly)]
    pub foreground: Color,
    #[qml(notify, readonly)]
    pub background: Color,
    /// Invalid for the terminal's own choice.
    #[qml(notify, readonly)]
    pub cursor: Color,
    #[qml(notify, readonly)]
    pub selection: Color,
    /// The 16 ANSI colours.
    #[qml(notify, readonly)]
    pub palette: Vec<Color>,

    /// Empty for the user's shell.
    #[qml(notify, readonly)]
    pub shell: String,
    #[qml(notify, readonly)]
    pub shell_arguments: Vec<String>,
    /// `[terminal.env]`, each `NAME=value`.
    #[qml(notify, readonly)]
    pub environment: Vec<String>,
    #[qml(notify, readonly)]
    pub working_directory: String,
    #[qml(notify, readonly)]
    pub scrollback: i32,
    #[qml(notify, readonly)]
    pub cursor_blink: bool,
    /// `"block"`, `"bar"` or `"underline"`.
    #[qml(notify, readonly)]
    pub cursor_shape: String,
    #[qml(notify, readonly)]
    pub copy_on_select: bool,
    #[qml(notify, readonly)]
    pub confirm_unsafe_paste: bool,

    /// Of the screen, 0 to 1.
    #[qml(notify, readonly)]
    pub dropdown_width: f64,
    #[qml(notify, readonly)]
    pub dropdown_height: f64,
    #[qml(notify, readonly)]
    pub dropdown_bottom: bool,
    #[qml(notify, readonly)]
    pub hide_on_focus_loss: bool,
    #[qml(notify, readonly)]
    pub animation_ms: i32,

    /// Reopen the last session's tabs and panes at start.
    #[qml(notify, readonly)]
    pub session_restore: bool,
    /// Keep each pane's scrollback with the saved session.
    #[qml(notify, readonly)]
    pub session_save_scrollback: bool,

    /// Key bindings, as two lists of the same length: the action and the
    /// key sequence, in the form `Shortcut.sequence` takes.
    #[qml(notify, readonly)]
    pub binding_actions: Vec<String>,
    #[qml(notify, readonly)]
    pub binding_sequences: Vec<String>,

    /// `[commands]`, as JSON: see `scripts::commands`.
    #[qml(notify, readonly)]
    pub commands: String,
    /// `[hooks]`, as JSON: see `scripts::hooks`.
    #[qml(notify, readonly)]
    pub hooks: String,

    /// `[hosts]`: the names of the machines panes can run on.
    #[qml(notify, readonly)]
    pub hosts: Vec<String>,

    /// What was wrong with the file, a line each; empty when nothing was.
    #[qml(notify, readonly)]
    pub problems: Vec<String>,

    /// Each host's name and the command that reaches it, for the terminals.
    #[qml(skip)]
    pub transports: Vec<(String, Vec<String>)>,
}

/// The file this run reads: the command line's, else the usual one.
pub(crate) fn config_path() -> PathBuf {
    crate::launch()
        .config
        .clone()
        .unwrap_or_else(hof_config::path)
}

fn color(c: Rgba) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// `chord` as Qt writes a key sequence: `Ctrl+Shift+PgUp`.
pub(crate) fn sequence(chord: &Chord) -> String {
    let mut out = String::new();
    for (held, name) in [
        (chord.ctrl, "Ctrl+"),
        (chord.alt, "Alt+"),
        (chord.shift, "Shift+"),
        (chord.super_, "Meta+"),
    ] {
        if held {
            out.push_str(name);
        }
    }
    let key = match chord.key.as_str() {
        "enter" => "Return",
        "tab" => "Tab",
        "space" => "Space",
        "escape" => "Esc",
        "backspace" => "Backspace",
        "delete" => "Del",
        "insert" => "Ins",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PgUp",
        "pagedown" => "PgDown",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "plus" => "+",
        "minus" => "-",
        "menu" => "Menu",
        "print" => "Print",
        "pause" => "Pause",
        other => {
            out.push_str(&other.to_uppercase());
            return out;
        }
    };
    out.push_str(key);
    out
}

impl Settings {
    /// The settings in the file at `path`. What cannot be read is left at
    /// its default and said in `problems`.
    pub fn read(path: &Path) -> Self {
        let mut problems = Vec::new();
        let config = match hof_config::load(path) {
            Ok(loaded) => {
                problems.extend(loaded.warnings.iter().map(ToString::to_string));
                loaded.config
            }
            Err(e) => {
                problems.push(e.to_string());
                hof_config::Config::default()
            }
        };
        let themes = hof_config::themes_dir_for(path);
        let theme = match config.theme(&themes) {
            Ok((theme, warnings)) => {
                problems.extend(warnings.iter().map(ToString::to_string));
                theme
            }
            Err(e) => {
                problems.push(e.to_string());
                let base = hof_config::builtin(hof_config::DEFAULT_THEME)
                    .cloned()
                    .unwrap_or_else(|| hof_config::themes()[0].1.clone());
                config.theme.apply(base)
            }
        };
        let (binding_actions, binding_sequences) = config
            .bindings()
            .iter()
            .map(|(chord, binding)| (binding.name(), sequence(chord)))
            .unzip();
        let (font, window, terminal, dropdown) = (
            &config.font,
            &config.window,
            &config.terminal,
            &config.dropdown,
        );
        Self {
            path: path.display().to_string(),
            font_family: font.family.clone(),
            font_size: font.size,
            bold_is_bright: font.bold_is_bright,
            padding: window.padding,
            opacity: window.opacity,
            decorations: window.decorations,
            title: window.title.clone(),
            columns: i32::from(window.width),
            rows: i32::from(window.height),
            foreground: color(theme.foreground),
            background: color(theme.background),
            cursor: theme.cursor.map_or(Color::INVALID, color),
            selection: theme.selection.map_or(Color::INVALID, color),
            palette: theme.palette.iter().copied().map(color).collect(),
            shell: terminal.shell.clone().unwrap_or_default(),
            shell_arguments: terminal.args.clone(),
            environment: terminal
                .env
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect(),
            working_directory: terminal
                .working_directory
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            scrollback: i32::try_from(terminal.scrollback).unwrap_or(i32::MAX),
            cursor_blink: terminal.cursor_blink,
            cursor_shape: format!("{:?}", terminal.cursor_shape).to_lowercase(),
            copy_on_select: terminal.copy_on_select,
            confirm_unsafe_paste: terminal.confirm_unsafe_paste,
            dropdown_width: dropdown.width,
            dropdown_height: dropdown.height,
            dropdown_bottom: dropdown.position == Position::Bottom,
            hide_on_focus_loss: dropdown.hide_on_focus_loss,
            animation_ms: i32::try_from(dropdown.animation_ms).unwrap_or(i32::MAX),
            session_restore: config.session.restore,
            session_save_scrollback: config.session.save_scrollback,
            binding_actions,
            binding_sequences,
            commands: crate::scripts::commands(&config),
            hooks: crate::scripts::hooks(&config.hooks),
            hosts: config.hosts.keys().cloned().collect(),
            problems,
            transports: config
                .hosts
                .iter()
                .map(|(name, host)| (name.clone(), host.transport(name)))
                .collect(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        let settings = Self::read(&config_path());
        hof_qml::remote::set_hosts(settings.transports.clone());
        settings
    }
}

/// Sets every property of the singleton `me` to `new`'s; only those that
/// changed tell their bindings.
fn apply(engine: &Engine, me: ObjectId, new: Settings) {
    macro_rules! set {
        ($($field:ident => $prop:ident),* $(,)?) => {
            $( let _ = engine.set_internal(me, Settings::$prop, new.$field); )*
        };
    }
    set!(
        path => PATH,
        font_family => FONT_FAMILY,
        font_size => FONT_SIZE,
        bold_is_bright => BOLD_IS_BRIGHT,
        padding => PADDING,
        opacity => OPACITY,
        decorations => DECORATIONS,
        title => TITLE,
        columns => COLUMNS,
        rows => ROWS,
        foreground => FOREGROUND,
        background => BACKGROUND,
        cursor => CURSOR,
        selection => SELECTION,
        palette => PALETTE,
        shell => SHELL,
        shell_arguments => SHELL_ARGUMENTS,
        environment => ENVIRONMENT,
        working_directory => WORKING_DIRECTORY,
        scrollback => SCROLLBACK,
        cursor_blink => CURSOR_BLINK,
        cursor_shape => CURSOR_SHAPE,
        copy_on_select => COPY_ON_SELECT,
        confirm_unsafe_paste => CONFIRM_UNSAFE_PASTE,
        dropdown_width => DROPDOWN_WIDTH,
        dropdown_height => DROPDOWN_HEIGHT,
        dropdown_bottom => DROPDOWN_BOTTOM,
        hide_on_focus_loss => HIDE_ON_FOCUS_LOSS,
        animation_ms => ANIMATION_MS,
        session_restore => SESSION_RESTORE,
        session_save_scrollback => SESSION_SAVE_SCROLLBACK,
        binding_actions => BINDING_ACTIONS,
        binding_sequences => BINDING_SEQUENCES,
        commands => COMMANDS,
        hooks => HOOKS,
        hosts => HOSTS,
        problems => PROBLEMS,
    );
}

fn reload_now(cx: &Cx) {
    let path = PathBuf::from(cx.get(Settings::PATH).unwrap_or_default());
    let new = Settings::read(&path);
    hof_qml::remote::set_hosts(new.transports.clone());
    apply(cx.engine, cx.this, new);
    let _ = Settings::emit_reloaded(cx);
}

/// Reloads `me` whenever the file or a theme beside it is saved. Watches
/// the directories rather than the file: an editor that saves by renaming
/// a new file into place would leave a watch on the file watching nothing.
fn watch(engine: &Engine, me: ObjectId, path: &Path) {
    use notify::{RecursiveMode, Watcher};

    let Some(dir) = path.parent().map(Path::to_path_buf) else {
        return;
    };
    let themes = hof_config::themes_dir_for(path);
    let file = path.to_path_buf();
    let post = Platform::of(engine).poster();
    // A save is several events; one reload is enough for all that arrive
    // before it runs.
    let pending = Arc::new(AtomicBool::new(false));
    let theirs = {
        let themes = themes.clone();
        move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if event.kind.is_access() {
                return;
            }
            let ours = event
                .paths
                .iter()
                .any(|p| p == &file || p.starts_with(&themes));
            if !ours || pending.swap(true, Ordering::AcqRel) {
                return;
            }
            let pending = Arc::clone(&pending);
            post.send(move |e: &Engine| {
                pending.store(false, Ordering::Release);
                reload_now(&Cx::new(e, me));
            });
        }
    };
    let mut watcher = match notify::recommended_watcher(theirs) {
        Ok(w) => w,
        Err(e) => {
            engine.warn(format!("hof: cannot watch {}: {e}", dir.display()));
            return;
        }
    };
    // Neither need exist yet; the first save of a new file is missed, the
    // ones after are not once Hof is restarted.
    let _ = watcher.watch(&dir, RecursiveMode::NonRecursive);
    let _ = watcher.watch(&themes, RecursiveMode::NonRecursive);
    // Lives as long as the process: the singleton never goes away.
    std::mem::forget(watcher);
}

#[qml_impl]
impl Settings {
    /// The file was read again.
    #[qml(signal)]
    fn reloaded();

    /// Reads the file again.
    #[qml(invokable)]
    fn reload(cx: &Cx) {
        reload_now(cx);
    }

    #[qml(on_complete)]
    fn completed(cx: &Cx) -> Result<(), EngineError> {
        if crate::launch().watch_config {
            let path = PathBuf::from(cx.get(Settings::PATH)?);
            watch(cx.engine, cx.this, &path);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        text.parse().unwrap()
    }

    #[test]
    fn chords_are_written_as_qt_writes_key_sequences() {
        assert_eq!(sequence(&chord("ctrl+shift+t")), "Ctrl+Shift+T");
        assert_eq!(sequence(&chord("shift+pageup")), "Shift+PgUp");
        assert_eq!(sequence(&chord("ctrl+shift+enter")), "Ctrl+Shift+Return");
        assert_eq!(sequence(&chord("ctrl+plus")), "Ctrl++");
        assert_eq!(sequence(&chord("ctrl+shift+minus")), "Ctrl+Shift+-");
        assert_eq!(sequence(&chord("super+alt+f11")), "Alt+Meta+F11");
    }

    #[test]
    fn a_missing_file_is_the_defaults_with_nothing_wrong() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::read(&dir.path().join("hof.toml"));
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.palette.len(), 16);
        assert!(s.background.is_valid());
        assert_eq!(s.binding_actions.len(), s.binding_sequences.len());
        assert!(s.binding_actions.iter().any(|a| a == "new-tab"));
    }

    #[test]
    fn what_the_file_says_is_what_the_window_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hof.toml");
        std::fs::write(
            &path,
            "[font]\nsize = 14\n[theme]\nbackground = \"#102030\"\n[keys]\nnew-tab = \"ctrl+alt+n\"\n[dropdown]\nposition = \"bottom\"\n",
        )
        .unwrap();
        let s = Settings::read(&path);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.font_size, 14.0);
        assert_eq!(s.background, Color::from_rgba8(0x10, 0x20, 0x30, 255));
        assert!(s.dropdown_bottom);
        let at = s
            .binding_actions
            .iter()
            .position(|a| a == "new-tab")
            .unwrap();
        assert_eq!(s.binding_sequences[at], "Ctrl+Alt+N");
    }

    #[test]
    fn a_command_s_keys_are_bound_to_it_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hof.toml");
        std::fs::write(
            &path,
            "[commands.top]\nrun = \"htop\"\nkeys = \"ctrl+shift+t\"\n",
        )
        .unwrap();
        let s = Settings::read(&path);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let at = s
            .binding_sequences
            .iter()
            .position(|q| q == "Ctrl+Shift+T")
            .unwrap();
        assert_eq!(s.binding_actions[at], "command:top");
        assert!(
            !s.binding_actions.iter().any(|a| a == "new-tab"),
            "it took new-tab's only chord"
        );
    }

    #[test]
    fn a_broken_file_is_said_and_the_defaults_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hof.toml");
        std::fs::write(&path, "[font\nsize = 14\n").unwrap();
        let s = Settings::read(&path);
        assert_eq!(s.problems.len(), 1, "{:?}", s.problems);
        assert_eq!(
            s.font_size,
            Settings::read(&dir.path().join("none.toml")).font_size
        );
    }

    #[test]
    fn an_unknown_key_is_a_problem_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hof.toml");
        std::fs::write(&path, "[font]\nsize = 13\nsizee = 2\n").unwrap();
        let s = Settings::read(&path);
        assert_eq!(s.font_size, 13.0);
        assert_eq!(s.problems.len(), 1);
        assert!(s.problems[0].contains("sizee"), "{}", s.problems[0]);
    }
}
