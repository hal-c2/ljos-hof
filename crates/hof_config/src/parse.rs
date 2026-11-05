//! Turning TOML text into a [`Config`], keeping track of where each key was
//! so that warnings and errors can point at a line.
//!
//! The text is read twice: once into toml's spanned document tree, which
//! knows every key and where it was, and once through serde into the typed
//! sections. The tree finds unknown keys and reads `[keys]`, where a mistake
//! should only be a warning; serde does the rest.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::Loaded;
use crate::config::{Config, ThemeConfig};
use crate::error::{Error, Warning, WarningKind};
use crate::keys::{Action, Chord, Keys};
use crate::scripts::{Command, PluginFile};

/// The keys Hof knows, as a tree mirroring the serde structs.
enum Node {
    /// A key whose value serde checks.
    Value,
    /// A table whose keys are checked against these.
    Table(&'static [(&'static str, Node)]),
    /// A table of the user's own keys, such as environment variables.
    Open,
    /// A table of the user's own keys, each a table checked against these.
    Each(&'static [(&'static str, Node)]),
}

use Node::{Each, Open, Table, Value};

// Kept in step with the structs in `config.rs`; the test that sets every key
// to a non-default value fails if the two drift apart.
const CONFIG_KEYS: &[(&str, Node)] = &[
    (
        "font",
        Table(&[
            ("family", Value),
            ("size", Value),
            ("line-height", Value),
            ("bold-is-bright", Value),
        ]),
    ),
    (
        "window",
        Table(&[
            ("padding", Value),
            ("opacity", Value),
            ("decorations", Value),
            ("width", Value),
            ("height", Value),
            ("title", Value),
            ("menu", Value),
        ]),
    ),
    (
        "terminal",
        Table(&[
            ("shell", Value),
            ("args", Value),
            ("scrollback", Value),
            ("cursor-blink", Value),
            ("cursor-shape", Value),
            ("copy-on-select", Value),
            ("confirm-unsafe-paste", Value),
            ("clipboard-read", Value),
            ("clipboard-write", Value),
            ("open-urls", Value),
            ("working-directory", Value),
            ("env", Open),
        ]),
    ),
    (
        "theme",
        Table(&[
            ("name", Value),
            ("foreground", Value),
            ("background", Value),
            ("cursor", Value),
            ("selection", Value),
            ("palette", Value),
        ]),
    ),
    (
        "dropdown",
        Table(&[
            ("height", Value),
            ("width", Value),
            ("position", Value),
            ("hide-on-focus-loss", Value),
            ("animation-ms", Value),
        ]),
    ),
    ("keys", Open),
    (
        "session",
        Table(&[("restore", Value), ("save-scrollback", Value)]),
    ),
    ("notify", Table(&[("agent-waiting", Value)])),
    (
        "mcp",
        Table(&[
            ("enabled", Value),
            ("socket", Value),
            ("allow-input", Value),
            ("allow-spawn", Value),
        ]),
    ),
    ("commands", Each(COMMAND_KEYS)),
    ("hooks", Table(HOOK_KEYS)),
    ("plugins", Table(&[("disabled", Value)])),
    (
        "hosts",
        Each(&[
            ("ssh", Value),
            ("user", Value),
            ("port", Value),
            ("ssh-options", Value),
            ("command", Value),
            ("hof", Value),
            ("bootstrap", Value),
            ("x11", Value),
            ("sound", Value),
            ("files", Value),
            ("clipboard", Value),
        ]),
    ),
    ("agents", Each(AGENT_KEYS)),
];

const AGENT_KEYS: &[(&str, Node)] = &[("process", Value), ("waiting", Value), ("working", Value)];

const COMMAND_KEYS: &[(&str, Node)] = &[
    ("run", Value),
    ("send", Value),
    ("in", Value),
    ("description", Value),
    ("keys", Value),
];

const HOOK_KEYS: &[(&str, Node)] = &[
    ("pane-opened", Value),
    ("pane-closed", Value),
    ("command-started", Value),
    ("command-finished", Value),
    ("bell", Value),
    ("notification", Value),
    ("directory-changed", Value),
    ("title-changed", Value),
    ("agent-changed", Value),
];

const PLUGIN_KEYS: &[(&str, Node)] = &[
    ("description", Value),
    ("commands", Each(COMMAND_KEYS)),
    ("hooks", Table(HOOK_KEYS)),
    ("agents", Each(AGENT_KEYS)),
];

const THEME_FILE_KEYS: &[(&str, Node)] = &[
    ("foreground", Value),
    ("background", Value),
    ("cursor", Value),
    ("selection", Value),
    ("palette", Value),
];

/// Whether `section.key` is a value Hof knows, as `font.size` is: the
/// keys a program may change one at a time ([`crate::edit::set`]).
///
/// A table of tables, as `[hosts]` is, takes its entry's name after a dot:
/// `hosts.dev`, `ssh`.
pub(crate) fn is_setting(section: &str, key: &str) -> bool {
    let has = |keys: &[(&str, Node)]| keys.iter().any(|(k, n)| *k == key && matches!(n, Value));
    let (top, entry) = match section.split_once('.') {
        Some((top, entry)) => (top, Some(entry)),
        None => (section, None),
    };
    CONFIG_KEYS.iter().any(|(name, node)| {
        *name == top
            && match (node, entry) {
                (Table(keys), None) => has(keys),
                (Each(keys), Some(entry)) => !entry.trim().is_empty() && has(keys),
                _ => false,
            }
    })
}

/// Whether `section` is an entry of a table of tables, as `hosts.dev`
/// is: what [`crate::edit::remove`] may take out.
pub(crate) fn is_entry(section: &str) -> bool {
    section.split_once('.').is_some_and(|(top, entry)| {
        !entry.trim().is_empty()
            && CONFIG_KEYS
                .iter()
                .any(|(name, node)| *name == top && matches!(node, Each(_)))
    })
}

/// One text being read, for turning byte offsets into `file:line` reports.
struct Source<'a> {
    text: &'a str,
    path: Option<&'a Path>,
}

impl Source<'_> {
    fn line_col(&self, offset: usize) -> (usize, usize) {
        let before = self.text.get(..offset).unwrap_or(self.text);
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        (line, before[line_start..].chars().count() + 1)
    }

    fn warning(&self, key: String, offset: Option<usize>, kind: WarningKind) -> Warning {
        Warning {
            path: self.path.map(Path::to_path_buf),
            line: offset.map(|o| self.line_col(o).0),
            key,
            kind,
        }
    }

    fn error(&self, offset: Option<usize>, message: String) -> Error {
        let (line, column) = offset.map_or((1, 1), |o| self.line_col(o));
        Error::Parse {
            path: self.path.map(Path::to_path_buf),
            line,
            column,
            message,
        }
    }

    /// A toml error, prefixed with the key it is about when the document
    /// tree can tell.
    fn toml_error(&self, err: &toml::de::Error, doc: Option<&DeTable>) -> Error {
        let offset = err.span().map(|s| s.start);
        let key = doc.zip(offset).and_then(|(d, o)| key_at(d, o, ""));
        let message = match key {
            Some(key) => format!("`{key}`: {}", err.message()),
            None => err.message().to_owned(),
        };
        self.error(offset, message)
    }

    fn document(&self) -> Result<Spanned<DeTable<'_>>, Error> {
        DeTable::parse(self.text).map_err(|e| self.toml_error(&e, None))
    }
}

pub(crate) fn config(text: &str, path: Option<&Path>) -> Result<Loaded, Error> {
    let src = Source { text, path };
    let doc = src.document()?;
    let doc = doc.get_ref();
    let mut warnings = Vec::new();
    unknown_keys(&src, doc, CONFIG_KEYS, "", &mut warnings);

    let mut config: Config = toml::from_str(text).map_err(|e| src.toml_error(&e, Some(doc)))?;

    if let Some((key, value)) = doc.get_key_value("keys") {
        let DeValue::Table(table) = value.get_ref() else {
            return Err(src.error(
                Some(key.span().start),
                "`keys` must be a table of action = chord".into(),
            ));
        };
        config.keys = read_keys(&src, table, &mut warnings);
    }
    read_command_keys(&src, doc, &mut config.commands, &mut warnings);

    let mut check = Checker {
        src: &src,
        doc,
        warnings: &mut warnings,
    };
    check.number("font.size", &mut config.font.size, |v| {
        (!(v > 0.0 && v.is_finite())).then_some(11.0)
    });
    check.number("font.line-height", &mut config.font.line_height, |v| {
        (!(v > 0.0 && v.is_finite())).then_some(1.0)
    });
    check.number("window.padding", &mut config.window.padding, |v| {
        (!(v >= 0.0 && v.is_finite())).then_some(0.0)
    });
    check.number("window.opacity", &mut config.window.opacity, |v| {
        clamp(v, 0.0, 1.0, 1.0)
    });
    check.number("dropdown.height", &mut config.dropdown.height, |v| {
        fraction(v, 0.4)
    });
    check.number("dropdown.width", &mut config.dropdown.width, |v| {
        fraction(v, 1.0)
    });
    check.cells("window.width", &mut config.window.width, 100);
    check.cells("window.height", &mut config.window.height, 30);
    check.palette("theme.palette", &mut config.theme);

    let home = home();
    for dir in [
        &mut config.terminal.working_directory,
        &mut config.mcp.socket,
    ]
    .into_iter()
    .flatten()
    {
        *dir = expand_tilde(dir, home.as_deref());
    }

    Ok(Loaded { config, warnings })
}

/// A theme file read from disk, before it is laid over `hof-dark`.
pub(crate) struct ThemeFile {
    pub theme: ThemeConfig,
    pub warnings: Vec<Warning>,
}

pub(crate) fn theme_file(path: &Path) -> Result<ThemeFile, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let src = Source {
        text: &text,
        path: Some(path),
    };
    let doc = src.document()?;
    let doc = doc.get_ref();
    let mut warnings = Vec::new();
    unknown_keys(&src, doc, THEME_FILE_KEYS, "", &mut warnings);
    let mut theme: ThemeConfig =
        toml::from_str(&text).map_err(|e| src.toml_error(&e, Some(doc)))?;
    Checker {
        src: &src,
        doc,
        warnings: &mut warnings,
    }
    .palette("palette", &mut theme);
    Ok(ThemeFile { theme, warnings })
}

pub(crate) fn read_file(path: &Path) -> Result<Option<String>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn unknown_keys(
    src: &Source,
    table: &DeTable,
    known: &[(&str, Node)],
    prefix: &str,
    warnings: &mut Vec<Warning>,
) {
    for (key, value) in table.iter() {
        let path = join(prefix, key.get_ref());
        match known.iter().find(|(name, _)| *name == key.get_ref()) {
            None => {
                warnings.push(src.warning(path, Some(key.span().start), WarningKind::UnknownKey))
            }
            // A table where a value belongs is a type error serde reports.
            Some((_, Table(children))) => {
                if let DeValue::Table(inner) = value.get_ref() {
                    unknown_keys(src, inner, children, &path, warnings);
                }
            }
            Some((_, Each(children))) => {
                if let DeValue::Table(inner) = value.get_ref() {
                    for (name, entry) in inner.iter() {
                        if let DeValue::Table(entry) = entry.get_ref() {
                            let path = join(&path, name.get_ref());
                            unknown_keys(src, entry, children, &path, warnings);
                        }
                    }
                }
            }
            Some((_, Value | Open)) => {}
        }
    }
}

fn read_keys(src: &Source, table: &DeTable, warnings: &mut Vec<Warning>) -> Keys {
    let mut keys = Keys::default();
    let mut lines: BTreeMap<Action, usize> = BTreeMap::new();
    for (name, value) in table.iter() {
        let key = format!("keys.{}", name.get_ref());
        let Ok(action) = name.get_ref().parse::<Action>() else {
            warnings.push(src.warning(key, Some(name.span().start), WarningKind::UnknownAction));
            continue;
        };
        let Some(chords) = read_chords(src, &key, value, warnings) else {
            continue;
        };
        let Some(chords) = chords else {
            keys.overrides.insert(action, Vec::new());
            continue;
        };
        // Every chord was a typo: keeping the defaults beats leaving the
        // action with no key at all.
        if !chords.is_empty() {
            lines.insert(action, src.line_col(name.span().start).0);
            keys.overrides.insert(action, chords);
        }
    }

    let mut owner: BTreeMap<Chord, Action> = BTreeMap::new();
    for (action, chords) in &mut keys.overrides {
        chords.retain(|chord| match owner.get(chord) {
            Some(kept) => {
                warnings.push(Warning {
                    line: lines.get(action).copied(),
                    ..src.warning(
                        format!("keys.{action}"),
                        None,
                        WarningKind::ChordConflict {
                            chord: chord.to_string(),
                            kept: kept.to_string(),
                        },
                    )
                });
                false
            }
            None => {
                owner.insert(chord.clone(), *action);
                true
            }
        });
    }
    // An action that lost every chord to a conflict was meant to be rebound,
    // not unbound, so it keeps its defaults.
    keys.overrides
        .retain(|action, chords| !chords.is_empty() || !lines.contains_key(action));
    keys
}

/// The chords a binding's `value` gives, a string or an array of them:
/// `Some(None)` when it says `"none"`, `None` (with a warning) when it is
/// not text at all. A chord that cannot be read is left out with a warning.
fn read_chords(
    src: &Source,
    key: &str,
    value: &Spanned<DeValue>,
    warnings: &mut Vec<Warning>,
) -> Option<Option<Vec<Chord>>> {
    let written: Option<Vec<(&str, usize)>> = match value.get_ref() {
        DeValue::String(s) => Some(vec![(s.as_ref(), value.span().start)]),
        DeValue::Array(items) => items
            .iter()
            .map(|item| match item.get_ref() {
                DeValue::String(s) => Some((s.as_ref(), item.span().start)),
                _ => None,
            })
            .collect(),
        _ => None,
    };
    let Some(written) = written else {
        warnings.push(src.warning(
            key.into(),
            Some(value.span().start),
            WarningKind::BadBinding,
        ));
        return None;
    };
    let is_none = |s: &str| s.trim().eq_ignore_ascii_case("none");
    if written.iter().all(|(s, _)| is_none(s)) {
        return Some(None);
    }
    let mut chords: Vec<Chord> = Vec::new();
    for (text, offset) in written.into_iter().filter(|(s, _)| !is_none(s)) {
        match text.parse::<Chord>() {
            Ok(chord) if !chords.contains(&chord) => chords.push(chord),
            Ok(_) => {}
            Err(reason) => warnings.push(src.warning(
                key.into(),
                Some(offset),
                WarningKind::BadChord {
                    chord: text.to_owned(),
                    reason: reason.to_string(),
                },
            )),
        }
    }
    Some(Some(chords))
}

/// The `keys` of each `[commands.<name>]`, which serde leaves alone.
fn read_command_keys(
    src: &Source,
    doc: &DeTable,
    commands: &mut BTreeMap<String, Command>,
    warnings: &mut Vec<Warning>,
) {
    let Some(DeValue::Table(table)) = doc.get("commands").map(Spanned::get_ref) else {
        return;
    };
    for (name, entry) in table.iter() {
        let DeValue::Table(entry) = entry.get_ref() else {
            continue;
        };
        let (Some(value), Some(command)) =
            (entry.get("keys"), commands.get_mut(name.get_ref().as_ref()))
        else {
            continue;
        };
        let key = format!("commands.{}.keys", name.get_ref());
        if let Some(chords) = read_chords(src, &key, value, warnings) {
            command.keys = Some(chords.unwrap_or_default());
        }
    }
}

/// A plugin file: commands and hooks, with what was wrong with them.
pub(crate) fn plugin(path: &Path) -> Result<(PluginFile, Vec<Warning>), Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let src = Source {
        text: &text,
        path: Some(path),
    };
    let doc = src.document()?;
    let doc = doc.get_ref();
    let mut warnings = Vec::new();
    unknown_keys(&src, doc, PLUGIN_KEYS, "", &mut warnings);
    let mut file: PluginFile = toml::from_str(&text).map_err(|e| src.toml_error(&e, Some(doc)))?;
    read_command_keys(&src, doc, &mut file.commands, &mut warnings);
    Ok((file, warnings))
}

/// Puts values that parse but make no sense back into range, with a warning.
struct Checker<'a, 'd> {
    src: &'a Source<'a>,
    doc: &'a DeTable<'d>,
    warnings: &'a mut Vec<Warning>,
}

impl Checker<'_, '_> {
    fn number(&mut self, key: &str, value: &mut f64, fix: impl Fn(f64) -> Option<f64>) {
        if let Some(used) = fix(*value) {
            self.out_of_range(key, value.to_string(), used.to_string());
            *value = used;
        }
    }

    fn cells(&mut self, key: &str, value: &mut u16, default: u16) {
        if *value == 0 {
            self.out_of_range(key, "0".into(), default.to_string());
            *value = default;
        }
    }

    fn palette(&mut self, key: &str, theme: &mut ThemeConfig) {
        let len = theme.palette.len();
        if len > 16 {
            let offset = value_offset(self.doc, key);
            self.warnings.push(self.src.warning(
                key.into(),
                offset,
                WarningKind::PaletteTooLong { len },
            ));
            theme.palette.truncate(16);
        }
    }

    fn out_of_range(&mut self, key: &str, value: String, used: String) {
        let offset = value_offset(self.doc, key);
        self.warnings.push(self.src.warning(
            key.into(),
            offset,
            WarningKind::OutOfRange { value, used },
        ));
    }
}

fn clamp(v: f64, lo: f64, hi: f64, nan: f64) -> Option<f64> {
    if v.is_nan() {
        Some(nan)
    } else {
        let c = v.clamp(lo, hi);
        (c != v).then_some(c)
    }
}

/// A share of the screen: zero or less means nothing would show, so it gets
/// the default rather than the nearest legal value.
fn fraction(v: f64, default: f64) -> Option<f64> {
    if v > 1.0 {
        Some(1.0)
    } else if v > 0.0 {
        None
    } else {
        Some(default)
    }
}

/// Byte offset of the value at a dotted `key`, for its line number.
fn value_offset(doc: &DeTable, key: &str) -> Option<usize> {
    let mut table = doc;
    let mut parts = key.split('.').peekable();
    while let Some(part) = parts.next() {
        let value = table.get(part)?;
        if parts.peek().is_none() {
            return Some(value.span().start);
        }
        let DeValue::Table(inner) = value.get_ref() else {
            return None;
        };
        table = inner;
    }
    None
}

/// The deepest key whose value covers `offset`, e.g. `font.size`.
fn key_at(table: &DeTable, offset: usize, prefix: &str) -> Option<String> {
    table
        .iter()
        .find_map(|(key, value)| value_key_at(value, offset, &join(prefix, key.get_ref())))
}

fn value_key_at(value: &Spanned<DeValue>, offset: usize, path: &str) -> Option<String> {
    let inner = match value.get_ref() {
        DeValue::Table(table) => key_at(table, offset, path),
        DeValue::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(i, item)| value_key_at(item, offset, &format!("{path}[{i}]"))),
        _ => None,
    };
    inner.or_else(|| value.span().contains(&offset).then(|| path.to_owned()))
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn home() -> Option<PathBuf> {
    non_empty(std::env::var_os("HOME")).map(PathBuf::from)
}

pub(crate) fn non_empty(value: Option<OsString>) -> Option<OsString> {
    value.filter(|v| !v.is_empty())
}

/// `~` and `~/rest` become paths under `home`; `~user` is left alone because
/// looking up other users' homes is not worth a dependency.
pub(crate) fn expand_tilde(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tilde_alone_is_home() {
        let home = Path::new("/home/ada");
        assert_eq!(expand_tilde(Path::new("~"), Some(home)), home);
    }

    #[test]
    fn a_tilde_prefix_is_expanded_under_home() {
        let got = expand_tilde(Path::new("~/src/hof"), Some(Path::new("/home/ada")));
        assert_eq!(got, Path::new("/home/ada/src/hof"));
    }

    #[test]
    fn other_users_homes_and_inner_tildes_are_left_alone() {
        let home = Some(Path::new("/home/ada"));
        assert_eq!(expand_tilde(Path::new("~bob/x"), home), Path::new("~bob/x"));
        assert_eq!(expand_tilde(Path::new("/tmp/~"), home), Path::new("/tmp/~"));
    }

    #[test]
    fn without_a_home_the_tilde_stays() {
        assert_eq!(expand_tilde(Path::new("~/x"), None), Path::new("~/x"));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let src = Source {
            text: "a = 1\nþú = 2",
            path: None,
        };
        assert_eq!(src.line_col(6), (2, 1));
        assert_eq!(src.line_col(6 + "þú".len()), (2, 3));
    }

    #[test]
    fn the_theme_section_and_theme_files_accept_the_same_colour_keys() {
        let Some((_, Table(theme))) = CONFIG_KEYS.iter().find(|(k, _)| *k == "theme") else {
            panic!("no theme section");
        };
        let section: Vec<_> = theme
            .iter()
            .map(|(k, _)| *k)
            .filter(|k| *k != "name")
            .collect();
        let file: Vec<_> = THEME_FILE_KEYS.iter().map(|(k, _)| *k).collect();
        assert_eq!(section, file);
    }
}
