//! The sections of `hof.toml` and their defaults.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agents::Agent;
use crate::error::{Error, Warning, WarningKind};
use crate::keys::Keys;
use crate::scripts::{Command, Hooks, Plugins};
use crate::theme::{self, DEFAULT_THEME, Rgba, Theme};

/// Everything in `hof.toml`. Every section and key is optional.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// `[font]`
    pub font: Font,
    /// `[window]`
    pub window: Window,
    /// `[terminal]`
    pub terminal: Terminal,
    /// `[theme]`; resolve it to colours with [`Config::theme`].
    pub theme: ThemeConfig,
    /// `[dropdown]`: the window that slides in from a screen edge.
    pub dropdown: Dropdown,
    /// `[keys]`; get the bindings in effect with [`Keys::bindings`].
    // Read by hand so a bad entry is a warning rather than a failed load.
    #[serde(skip)]
    pub keys: Keys,
    /// `[session]`
    pub session: Session,
    /// `[mcp]`
    pub mcp: Mcp,
    /// `[notify]`
    pub notify: Notify,
    /// `[commands.<name>]`, the user's and the plugins'.
    pub commands: BTreeMap<String, Command>,
    /// `[hooks]`, the user's and then the plugins'.
    pub hooks: Hooks,
    /// `[plugins]`
    pub plugins: Plugins,
    /// `[hosts.<name>]`: other machines panes can run on.
    pub hosts: BTreeMap<String, Host>,
    /// `[agents.<name>]`, the user's and then the plugins'; see
    /// [`Config::agents`] for the ones in effect.
    pub agents: BTreeMap<String, Agent>,
}

/// `[font]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Font {
    /// Font family name, as fontconfig knows it.
    pub family: String,
    /// Size in points.
    pub size: f64,
    /// Line height as a multiple of the font's own.
    pub line_height: f64,
    /// Draw bold text in the bright form of its ANSI colour.
    pub bold_is_bright: bool,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: "monospace".into(),
            size: 11.0,
            line_height: 1.0,
            bold_is_bright: false,
        }
    }
}

/// `[window]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Window {
    /// Space between the window edge and the cells, in logical pixels.
    pub padding: f64,
    /// Background opacity, 0 (clear) to 1 (opaque).
    pub opacity: f64,
    /// Ask the window manager for a title bar and borders.
    pub decorations: bool,
    /// Initial width in cells.
    pub width: u16,
    /// Initial height in cells.
    pub height: u16,
    /// Window title until the program running in the terminal sets one.
    pub title: String,
    /// Where the menu of Hof's actions is.
    pub menu: Menu,
}

/// Where the menu of Hof's actions is, beside the right-click menu every
/// pane has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Menu {
    /// The system's menu bar where there is one (macOS), else `button`.
    #[default]
    Auto,
    /// A menu bar: the system's on macOS, else across the top of the window.
    Bar,
    /// A button at the end of the tab bar, which is then always shown.
    Button,
    /// None: the keys and the right-click menu are enough.
    None,
}

impl Menu {
    /// What `Auto` is on this system: `Bar` on macOS, else `Button`.
    pub fn resolved(self) -> Menu {
        match self {
            Menu::Auto if cfg!(target_os = "macos") => Menu::Bar,
            Menu::Auto => Menu::Button,
            other => other,
        }
    }
}

impl Default for Window {
    fn default() -> Self {
        Self {
            padding: 6.0,
            opacity: 1.0,
            decorations: true,
            width: 100,
            height: 30,
            title: "Hof".into(),
            menu: Menu::Auto,
        }
    }
}

/// How the cursor is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CursorShape {
    /// The whole cell.
    #[default]
    Block,
    /// A thin line before the character.
    Bar,
    /// A line under the character.
    Underline,
}

/// `[terminal]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Terminal {
    /// Program to run; `None` means `$SHELL`, then `/bin/sh`. See
    /// [`Terminal::program`].
    pub shell: Option<String>,
    /// Arguments for `shell`.
    pub args: Vec<String>,
    /// Lines of scrollback kept per pane.
    pub scrollback: usize,
    /// Blink the cursor.
    pub cursor_blink: bool,
    /// Cursor shape until the program asks for another.
    pub cursor_shape: CursorShape,
    /// Copy to the clipboard as soon as text is selected.
    pub copy_on_select: bool,
    /// Ask before pasting text that contains a newline or control codes.
    pub confirm_unsafe_paste: bool,
    /// Whether programs here may read the clipboard (OSC 52); a program
    /// on another machine is under its host's `clipboard`.
    pub clipboard_read: Permission,
    /// Let programs put text on the clipboard (OSC 52).
    pub clipboard_write: bool,
    /// Whether programs may have URLs opened here, as `hof open` asks.
    pub open_urls: Permission,
    /// Directory new terminals start in, with a leading `~` already
    /// expanded; `None` means Hof's own working directory.
    pub working_directory: Option<PathBuf>,
    /// Extra environment variables for the program.
    pub env: BTreeMap<String, String>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            shell: None,
            args: Vec::new(),
            scrollback: 10_000,
            cursor_blink: true,
            cursor_shape: CursorShape::Block,
            copy_on_select: true,
            confirm_unsafe_paste: true,
            clipboard_read: Permission::Ask,
            clipboard_write: true,
            open_urls: Permission::Ask,
            working_directory: None,
            env: BTreeMap::new(),
        }
    }
}

impl Terminal {
    /// The program to run: `shell`, else `$SHELL`, else `/bin/sh`.
    pub fn program(&self) -> String {
        self.program_with(std::env::var("SHELL").ok())
    }

    fn program_with(&self, env_shell: Option<String>) -> String {
        self.shell
            .clone()
            .or(env_shell.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "/bin/sh".into())
    }
}

/// `[theme]`: a theme by name, with optional per-colour overrides.
///
/// Theme files in the themes directory use the same keys, without `name`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ThemeConfig {
    /// A built-in theme or the stem of a file in the themes directory.
    pub name: String,
    /// Overrides the theme's foreground.
    pub foreground: Option<Rgba>,
    /// Overrides the theme's background.
    pub background: Option<Rgba>,
    /// Overrides the theme's cursor colour.
    pub cursor: Option<Rgba>,
    /// Overrides the theme's selection colour.
    pub selection: Option<Rgba>,
    /// Overrides the first `len()` ANSI colours; at most 16.
    pub palette: Vec<Rgba>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: DEFAULT_THEME.into(),
            foreground: None,
            background: None,
            cursor: None,
            selection: None,
            palette: Vec::new(),
        }
    }
}

impl ThemeConfig {
    /// `base` with these overrides applied.
    pub fn apply(&self, mut base: Theme) -> Theme {
        if let Some(c) = self.foreground {
            base.foreground = c;
        }
        if let Some(c) = self.background {
            base.background = c;
        }
        if self.cursor.is_some() {
            base.cursor = self.cursor;
        }
        if self.selection.is_some() {
            base.selection = self.selection;
        }
        for (slot, colour) in base.palette.iter_mut().zip(&self.palette) {
            *slot = *colour;
        }
        base
    }
}

/// Which screen edge the dropdown window comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    /// The top edge.
    #[default]
    Top,
    /// The bottom edge.
    Bottom,
}

/// `[dropdown]`: the quake-style window toggled by a global shortcut.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Dropdown {
    /// Height as a fraction of the screen, above 0 and at most 1.
    pub height: f64,
    /// Width as a fraction of the screen, above 0 and at most 1.
    pub width: f64,
    /// The edge it slides in from.
    pub position: Position,
    /// Hide it when another window takes focus.
    pub hide_on_focus_loss: bool,
    /// Length of the slide, in milliseconds; 0 for none.
    pub animation_ms: u32,
}

impl Default for Dropdown {
    fn default() -> Self {
        Self {
            height: 0.4,
            width: 1.0,
            position: Position::Top,
            hide_on_focus_loss: true,
            animation_ms: 150,
        }
    }
}

/// `[session]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Session {
    /// Reopen the last session's windows, tabs and panes at start.
    pub restore: bool,
    /// Keep each pane's scrollback with the saved session.
    pub save_scrollback: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            restore: false,
            save_scrollback: true,
        }
    }
}

/// `[notify]`: what Hof tells the desktop of, as a notification.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Notify {
    /// An agent the user cannot see starts waiting for them: its pane is
    /// in another tab, does not have the keys, or the window is not the
    /// one in front.
    pub agent_waiting: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            agent_waiting: true,
        }
    }
}

/// `[mcp]`: the Model Context Protocol server that lets agents drive Hof.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Mcp {
    /// Run the server.
    pub enabled: bool,
    /// Unix socket to listen on, with a leading `~` already expanded;
    /// `None` lets Hof choose one in the runtime directory.
    pub socket: Option<PathBuf>,
    /// Let agents type into panes and run commands in them. Off, they can
    /// only look.
    pub allow_input: bool,
    /// Let agents open and close tabs and panes.
    pub allow_spawn: bool,
}

/// `[hosts.<name>]`: a machine panes can run on, through `hof agent` there.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Host {
    /// What to give `ssh` to reach it; empty means the host's name, which
    /// `~/.ssh/config` may know.
    pub ssh: String,
    /// More for `ssh`, before the destination: `["-p", "2222"]`.
    pub ssh_options: Vec<String>,
    /// A program, and its arguments, whose stdin and stdout reach `hof
    /// agent --stdio` some other way than ssh. Set, it replaces ssh.
    pub command: Vec<String>,
    /// The Hof to run on that machine, a name on its `PATH` or a path.
    pub hof: String,
    /// Copy Hof's agent there when the machine has no Hof. Only over ssh.
    pub bootstrap: bool,
    /// Let programs there open X11 windows here.
    pub x11: bool,
    /// Let programs there play sound here.
    pub sound: bool,
    /// Let Hof show that machine's files here.
    pub files: bool,
    /// Whether programs there may read this machine's clipboard. Writing
    /// to it is as `terminal.clipboard-write` says.
    pub clipboard: Permission,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            ssh: String::new(),
            ssh_options: Vec::new(),
            command: Vec::new(),
            hof: "hof".into(),
            bootstrap: true,
            x11: false,
            sound: false,
            files: false,
            clipboard: Permission::Ask,
        }
    }
}

impl Host {
    /// The program, and its arguments, that connects to the host `name`:
    /// `command`, or else ssh running `hof agent --stdio` there.
    pub fn transport(&self, name: &str) -> Vec<String> {
        if !self.command.is_empty() {
            return self.command.clone();
        }
        let mut argv = self.ssh_argv(name);
        argv.extend(["--", &self.hof, "agent", "--stdio"].map(String::from));
        argv
    }

    /// What runs a command on the host `name` over ssh, the command left
    /// to add after a `--`.
    pub fn ssh_argv(&self, name: &str) -> Vec<String> {
        let dest = if self.ssh.is_empty() { name } else { &self.ssh };
        // -T: the agent speaks frames on stdio, which a remote pty would
        // mangle.
        let mut argv = vec!["ssh".to_owned(), "-T".to_owned()];
        argv.extend(self.ssh_options.iter().cloned());
        argv.push(dest.to_owned());
        argv
    }

    /// Whether Hof copies its agent there, when it has none: over ssh,
    /// and unless `bootstrap` is off.
    pub fn bootstraps(&self) -> bool {
        self.bootstrap && self.command.is_empty()
    }
}

/// Whether something may happen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// Ask each time.
    #[default]
    Ask,
    /// Always.
    Allow,
    /// Never.
    Deny,
}

impl Config {
    /// Every `[agents]` table in effect: the ones Hof ships, replaced by
    /// name with the config's and the plugins'.
    pub fn agents(&self) -> BTreeMap<String, Agent> {
        crate::agents::in_effect(&self.agents)
    }

    /// The theme to draw with: `theme.name` looked up first as
    /// `<themes_dir>/<name>.toml`, then among the built-in themes, with the
    /// `[theme]` overrides applied on top.
    ///
    /// A theme file starts from `hof-dark`, so it only needs the colours it
    /// changes. An unknown name is a warning and falls back to `hof-dark`;
    /// a theme file that cannot be read or parsed is an error.
    pub fn theme(&self, themes_dir: &Path) -> Result<(Theme, Vec<Warning>), Error> {
        let name = self.theme.name.as_str();
        let mut warnings = Vec::new();
        let base = match theme_file(themes_dir, name) {
            Some(path) => {
                let loaded = crate::parse::theme_file(&path)?;
                warnings.extend(loaded.warnings);
                loaded.theme.apply(Theme::default())
            }
            None => match theme::builtin(name) {
                Some(theme) => theme.clone(),
                None => {
                    warnings.push(Warning {
                        path: None,
                        line: None,
                        key: "theme.name".into(),
                        kind: WarningKind::UnknownTheme { name: name.into() },
                    });
                    Theme::default()
                }
            },
        };
        Ok((self.theme.apply(base), warnings))
    }
}

/// The theme file for `name`, if one exists. Names that are not plain file
/// stems never reach the file system, so `theme.name` cannot point outside
/// the themes directory.
fn theme_file(themes_dir: &Path, name: &str) -> Option<PathBuf> {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.contains('\0');
    let path = themes_dir.join(format!("{name}.toml"));
    (plain && path.is_file()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_shell_wins_over_the_environment() {
        let t = Terminal {
            shell: Some("/usr/bin/fish".into()),
            ..Terminal::default()
        };
        assert_eq!(t.program_with(Some("/bin/zsh".into())), "/usr/bin/fish");
    }

    #[test]
    fn without_a_shell_the_environment_shell_is_used() {
        let t = Terminal::default();
        assert_eq!(t.program_with(Some("/bin/zsh".into())), "/bin/zsh");
    }

    #[test]
    fn with_no_shell_anywhere_it_is_bin_sh() {
        let t = Terminal::default();
        assert_eq!(t.program_with(None), "/bin/sh");
        assert_eq!(t.program_with(Some(String::new())), "/bin/sh");
    }

    #[test]
    fn a_short_palette_override_replaces_only_the_first_colours() {
        let overrides = ThemeConfig {
            palette: vec![Rgba::rgb(0x010203)],
            ..ThemeConfig::default()
        };
        let base = Theme::default();
        let theme = overrides.apply(base.clone());
        assert_eq!(theme.palette[0], Rgba::rgb(0x010203));
        assert_eq!(theme.palette[1..], base.palette[1..]);
    }

    #[test]
    fn theme_names_with_path_separators_never_read_files() {
        let dir = std::env::temp_dir();
        assert_eq!(theme_file(&dir, "../etc/passwd"), None);
        assert_eq!(theme_file(&dir, ".hidden"), None);
    }
}
