//! What extends Hof from the config: `[commands.<name>]` the user can bind
//! to keys, `[hooks]` run when something happens in a pane, and plugins,
//! files in a `plugins` directory beside the config that bring their own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

use crate::config::Config;
use crate::error::{Warning, WarningKind};
use crate::keys::{Action, Chord};

/// `[commands.<name>]`: something to run, or to type, at a key.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Command {
    /// The program, run in a new pane or in the background (see `place`).
    pub run: Option<Program>,
    /// Text to type into the focused pane, in place of running anything.
    pub send: Option<String>,
    /// Where `run` runs.
    #[serde(rename = "in")]
    pub place: Place,
    /// What it does, in a few words, for menus and `hof ctl`.
    pub description: String,
    /// The chords that run it; `None` when the file gave none. Read by hand,
    /// so a bad chord is a warning.
    #[serde(skip)]
    pub keys: Option<Vec<Chord>>,
    /// The plugin's directory, for a command a plugin brought.
    #[serde(skip)]
    pub plugin: Option<Plugin>,
}

/// A program: one string for `sh -c`, or the program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Program {
    /// Run by `sh -c`, so it can use pipes and `$VARIABLES`.
    Shell(String),
    /// Run as it is.
    Argv(Vec<String>),
}

impl Program {
    /// The program and its arguments.
    pub fn argv(&self) -> Vec<String> {
        match self {
            Program::Shell(script) => vec!["sh".into(), "-c".into(), script.clone()],
            Program::Argv(argv) => argv.clone(),
        }
    }
}

/// Where a command's program runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    /// A new tab.
    #[default]
    Tab,
    /// A new pane beside the focused one.
    SplitRight,
    /// A new pane below the focused one.
    SplitDown,
    /// No pane: its output goes to Hof's.
    Background,
}

impl Place {
    /// The kebab-case name the file uses.
    pub fn name(self) -> &'static str {
        match self {
            Place::Tab => "tab",
            Place::SplitRight => "split-right",
            Place::SplitDown => "split-down",
            Place::Background => "background",
        }
    }
}

/// Where a command or a hook came from, when a plugin brought it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// The plugin's name: its file's stem, or its directory's name.
    pub name: String,
    /// What its programs get as `HOF_PLUGIN_DIR`: the directory its file
    /// is in, where it keeps its scripts.
    pub dir: PathBuf,
    /// The file it was read from.
    pub file: PathBuf,
}

/// One shell snippet for an event, run by `sh -c`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    /// The snippet.
    pub run: String,
    /// The plugin that brought it.
    pub plugin: Option<Plugin>,
}

/// `[hooks]`: what to run when something happens in a pane. Each event
/// takes a snippet or a list of them, run by `sh -c` in the background
/// with what happened in `HOF_*` variables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Hooks {
    /// A pane opened.
    #[serde(deserialize_with = "snippets")]
    pub pane_opened: Vec<Hook>,
    /// A pane closed, or its program ended.
    #[serde(deserialize_with = "snippets")]
    pub pane_closed: Vec<Hook>,
    /// The shell started a command (shell integration): `HOF_COMMAND`.
    #[serde(deserialize_with = "snippets")]
    pub command_started: Vec<Hook>,
    /// The shell's command finished: `HOF_EXIT_CODE`, empty when the shell
    /// did not say.
    #[serde(deserialize_with = "snippets")]
    pub command_finished: Vec<Hook>,
    /// The program rang the bell.
    #[serde(deserialize_with = "snippets")]
    pub bell: Vec<Hook>,
    /// The program asked for a desktop notification: `HOF_TITLE`, `HOF_BODY`.
    #[serde(deserialize_with = "snippets")]
    pub notification: Vec<Hook>,
    /// The shell said it is somewhere else: `HOF_DIRECTORY`.
    #[serde(deserialize_with = "snippets")]
    pub directory_changed: Vec<Hook>,
    /// The program set the title: `HOF_TITLE`.
    #[serde(deserialize_with = "snippets")]
    pub title_changed: Vec<Hook>,
    /// An agent in the pane started or stopped working or waiting:
    /// `HOF_AGENT`, its name, and `HOF_AGENT_STATE`, how it is now
    /// (`working`, `waiting`, `idle`, or `none` once it is gone).
    #[serde(deserialize_with = "snippets")]
    pub agent_changed: Vec<Hook>,
}

impl Hooks {
    /// Every event, by the name the file uses.
    pub const EVENTS: [&str; 9] = [
        "pane-opened",
        "pane-closed",
        "command-started",
        "command-finished",
        "bell",
        "notification",
        "directory-changed",
        "title-changed",
        "agent-changed",
    ];

    /// The hooks for `event`, by the name the file uses; none for a name
    /// that is not an event.
    pub fn of(&self, event: &str) -> &[Hook] {
        match event {
            "pane-opened" => &self.pane_opened,
            "pane-closed" => &self.pane_closed,
            "command-started" => &self.command_started,
            "command-finished" => &self.command_finished,
            "bell" => &self.bell,
            "notification" => &self.notification,
            "directory-changed" => &self.directory_changed,
            "title-changed" => &self.title_changed,
            "agent-changed" => &self.agent_changed,
            _ => &[],
        }
    }

    fn of_mut(&mut self, event: &str) -> Option<&mut Vec<Hook>> {
        Some(match event {
            "pane-opened" => &mut self.pane_opened,
            "pane-closed" => &mut self.pane_closed,
            "command-started" => &mut self.command_started,
            "command-finished" => &mut self.command_finished,
            "bell" => &mut self.bell,
            "notification" => &mut self.notification,
            "directory-changed" => &mut self.directory_changed,
            "title-changed" => &mut self.title_changed,
            "agent-changed" => &mut self.agent_changed,
            _ => return None,
        })
    }

    /// `other`'s hooks run after these.
    pub(crate) fn extend(&mut self, other: Hooks) {
        for event in Self::EVENTS {
            let theirs = other.of(event).to_vec();
            if let Some(mine) = self.of_mut(event) {
                mine.extend(theirs);
            }
        }
    }

    /// Marks every hook as `plugin`'s.
    pub(crate) fn brought_by(mut self, plugin: &Plugin) -> Self {
        for event in Self::EVENTS {
            for hook in self.of_mut(event).into_iter().flatten() {
                hook.plugin = Some(plugin.clone());
            }
        }
        self
    }
}

fn snippets<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Hook>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    let runs = match OneOrMany::deserialize(d)? {
        OneOrMany::One(run) => vec![run],
        OneOrMany::Many(runs) => runs,
    };
    Ok(runs
        .into_iter()
        .map(|run| Hook { run, plugin: None })
        .collect())
}

/// `[plugins]`
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Plugins {
    /// Plugins, by name, to leave unloaded.
    pub disabled: Vec<String>,
}

/// What a key runs: one of Hof's actions, or a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// A built-in action.
    Action(Action),
    /// A `[commands.<name>]` entry, by name.
    Command(String),
}

impl Binding {
    /// The action's name, or `command:<name>`: the prefix keeps a command
    /// called `copy` from being taken for the action.
    pub fn name(&self) -> String {
        match self {
            Binding::Action(action) => action.name().to_owned(),
            Binding::Command(name) => format!("command:{name}"),
        }
    }
}

impl Config {
    /// Every key binding in effect. The user's own `[keys]` entries come
    /// first, then the commands' chords (the user's before plugins'), then
    /// the default bindings no one took. Each chord appears once.
    pub fn bindings(&self) -> Vec<(Chord, Binding)> {
        let actions = self.keys.bindings();
        let mine = |a: &Action| self.keys.overrides.contains_key(a);
        let mut out: Vec<(Chord, Binding)> = actions
            .iter()
            .filter(|(_, a)| mine(a))
            .map(|(c, a)| (c.clone(), Binding::Action(*a)))
            .collect();
        for (name, command) in self.commands_in_order() {
            for chord in command.keys.iter().flatten() {
                if !out.iter().any(|(c, _)| c == chord) {
                    out.push((chord.clone(), Binding::Command(name.to_owned())));
                }
            }
        }
        for (chord, action) in actions {
            if !mine(&action) && !out.iter().any(|(c, _)| *c == chord) {
                out.push((chord, Binding::Action(action)));
            }
        }
        out
    }

    /// The user's commands, then the plugins'.
    fn commands_in_order(&self) -> impl Iterator<Item = (&str, &Command)> {
        let (user, plugins): (Vec<_>, Vec<_>) =
            self.commands.iter().partition(|(_, c)| c.plugin.is_none());
        user.into_iter()
            .chain(plugins)
            .map(|(name, command)| (name.as_str(), command))
    }
}

/// The plugin files in `dir`, sorted by name: `<name>.toml`, and
/// `<name>/plugin.toml` for a plugin that keeps scripts beside it.
pub(crate) fn plugin_files(dir: &Path) -> Vec<Plugin> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Plugin> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_str()?.to_owned();
            if name.starts_with('.') {
                return None;
            }
            if path.is_dir() {
                let file = path.join("plugin.toml");
                return file.is_file().then_some(Plugin {
                    name,
                    dir: path,
                    file,
                });
            }
            Some(Plugin {
                name: name.strip_suffix(".toml")?.to_owned(),
                dir: dir.to_path_buf(),
                file: path,
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// One plugin file, read.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub(crate) struct PluginFile {
    #[allow(dead_code, reason = "read so the file may say what it is")]
    pub description: String,
    pub commands: BTreeMap<String, Command>,
    pub hooks: Hooks,
    pub agents: BTreeMap<String, crate::Agent>,
}

/// Adds `plugin`'s commands and hooks to `config`. A command the user or
/// an earlier plugin already defines stays theirs; a user's entry that
/// only gives `keys` rebinds the plugin's command.
pub(crate) fn add_plugin(
    config: &mut Config,
    plugin: &Plugin,
    file: PluginFile,
    warnings: &mut Vec<Warning>,
) {
    for (name, mut command) in file.commands {
        command.plugin = Some(plugin.clone());
        match config.commands.get_mut(&name) {
            None => {
                config.commands.insert(name, command);
            }
            Some(mine) if mine.plugin.is_none() && !mine.does_something() => {
                command.keys = mine.keys.take().or(command.keys);
                if !mine.description.is_empty() {
                    command.description = std::mem::take(&mut mine.description);
                }
                *mine = command;
            }
            Some(kept) => warnings.push(Warning {
                path: Some(plugin.file.clone()),
                line: None,
                key: format!("commands.{name}"),
                kind: WarningKind::CommandConflict {
                    kept: match &kept.plugin {
                        Some(p) => format!("the plugin `{}`", p.name),
                        None => "the config".into(),
                    },
                },
            }),
        }
    }
    config.hooks.extend(file.hooks.brought_by(plugin));
    // The user's table of a name, or an earlier plugin's, stays.
    for (name, agent) in file.agents {
        config.agents.entry(name).or_insert(agent);
    }
}

impl Command {
    fn does_something(&self) -> bool {
        self.run.is_some() || self.send.is_some()
    }
}

/// Drops commands that do nothing, and gives each chord to one command:
/// the user's `[keys]` entries keep theirs, then the user's commands, then
/// the plugins', in name order.
pub(crate) fn settle(config: &mut Config, path: Option<&Path>, warnings: &mut Vec<Warning>) {
    let where_from = |c: &Command| match &c.plugin {
        Some(p) => Some(p.file.clone()),
        None => path.map(Path::to_path_buf),
    };
    config.commands.retain(|name, command| {
        let reason = match (&command.run, &command.send) {
            (None, None) => "needs `run` or `send`",
            (Some(_), Some(_)) => "takes `run` or `send`, not both",
            (Some(Program::Argv(argv)), None) if argv.is_empty() => "`run` is an empty list",
            _ => return true,
        };
        warnings.push(Warning {
            path: where_from(command),
            line: None,
            key: format!("commands.{name}"),
            kind: WarningKind::BadCommand {
                reason: reason.into(),
            },
        });
        false
    });

    let mut owner: BTreeMap<Chord, String> = BTreeMap::new();
    for (action, chords) in &config.keys.overrides {
        for chord in chords {
            owner.insert(chord.clone(), action.name().to_owned());
        }
    }
    let names: Vec<String> = config
        .commands_in_order()
        .map(|(n, _)| n.to_owned())
        .collect();
    for name in names {
        let command = config.commands.get_mut(&name).expect("named above");
        let from = where_from(command);
        let Some(chords) = command.keys.as_mut() else {
            continue;
        };
        chords.retain(|chord| match owner.get(chord) {
            Some(kept) => {
                warnings.push(Warning {
                    path: from.clone(),
                    line: None,
                    key: format!("commands.{name}.keys"),
                    kind: WarningKind::ChordConflict {
                        chord: chord.to_string(),
                        kept: kept.clone(),
                    },
                });
                false
            }
            None => {
                owner.insert(chord.clone(), format!("command:{name}"));
                true
            }
        });
    }
}
