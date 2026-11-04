//! Hof's configuration: one TOML file where every key is optional.
//!
//! [`load`] reads it from [`path`]. A missing file is the defaults. A key
//! Hof does not know is a [`Warning`], not an error, so an older Hof still
//! starts with a file written for a newer one. Only text that is not TOML,
//! or a value of the wrong type or form, is an [`Error`].
//!
//! `hof.example.toml` next to this crate's manifest lists every key with its
//! default.

mod agents;
mod config;
mod error;
mod keys;
mod parse;
mod scripts;
mod theme;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub use agents::Agent;
pub use config::{
    Config, CursorShape, Dropdown, Font, Host, Mcp, Notify, Permission, Position, Session,
    Terminal, ThemeConfig, Window,
};
pub use error::{Error, Warning, WarningKind};
pub use keys::{Action, BadChord, Chord, Keys, Platform, UnknownAction};
pub use scripts::{Binding, Command, Hook, Hooks, Place, Plugin, Plugins, Program};
pub use theme::{BadColour, DEFAULT_THEME, Rgba, Theme, builtin, themes};

/// The example file: every key, commented out, at its default. What
/// `edit-config` writes when there is no file yet.
pub const EXAMPLE: &str = include_str!("../hof.example.toml");

/// A configuration that Hof can use, with what it had to ignore or adjust
/// to get there.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The configuration.
    pub config: Config,
    /// Things worth telling the user, in file order where possible.
    pub warnings: Vec<Warning>,
}

/// Where the configuration file lives: `$HOF_CONFIG`, else
/// `$XDG_CONFIG_HOME/hof/hof.toml`, else `~/.config/hof/hof.toml`.
pub fn path() -> PathBuf {
    path_from(
        std::env::var_os("HOF_CONFIG"),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// Where theme files live: a `themes` directory beside [`path`].
pub fn themes_dir() -> PathBuf {
    themes_dir_for(&path())
}

/// The `plugins` directory beside the configuration file at `config`.
pub fn plugins_dir_for(config: &Path) -> PathBuf {
    config
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("plugins")
}

/// The `themes` directory beside the configuration file at `config`.
pub fn themes_dir_for(config: &Path) -> PathBuf {
    config
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("themes")
}

// Empty variables count as unset, as the XDG base directory spec asks.
fn path_from(
    hof_config: Option<OsString>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> PathBuf {
    if let Some(explicit) = parse::non_empty(hof_config) {
        return explicit.into();
    }
    let config_home = parse::non_empty(xdg_config_home)
        .map(PathBuf::from)
        .or_else(|| parse::non_empty(home).map(|h| PathBuf::from(h).join(".config")))
        // With no home at all the best guess is the working directory.
        .unwrap_or_default();
    config_home.join("hof").join("hof.toml")
}

/// Reads the configuration at `path`, and the plugins beside it (see
/// [`plugins_dir_for`]) that `[plugins] disabled` does not name. A missing
/// file is the defaults; a plugin that cannot be read is a warning.
pub fn load(path: &Path) -> Result<Loaded, Error> {
    let mut loaded = match parse::read_file(path)? {
        Some(text) => parse::config(&text, Some(path))?,
        None => Loaded {
            config: Config::default(),
            warnings: Vec::new(),
        },
    };
    let Loaded { config, warnings } = &mut loaded;
    for plugin in scripts::plugin_files(&plugins_dir_for(path)) {
        if config.plugins.disabled.contains(&plugin.name) {
            continue;
        }
        match parse::plugin(&plugin.file) {
            Ok((file, theirs)) => {
                warnings.extend(theirs);
                scripts::add_plugin(config, &plugin, file, warnings);
            }
            Err(e) => {
                // The warning names the file already.
                let (line, message) = match e {
                    Error::Parse { line, message, .. } => (Some(line), message),
                    Error::Read { source, .. } => (None, source.to_string()),
                };
                warnings.push(Warning {
                    path: Some(plugin.file.clone()),
                    line,
                    key: format!("plugins.{}", plugin.name),
                    kind: WarningKind::BadPlugin { message },
                });
            }
        }
    }
    scripts::settle(config, Some(path), warnings);
    Ok(loaded)
}

/// Reads a configuration from text, for tests and scripts: no plugins.
/// Errors and warnings carry no path.
pub fn parse(text: &str) -> Result<Loaded, Error> {
    let mut loaded = parse::config(text, None)?;
    scripts::settle(&mut loaded.config, None, &mut loaded.warnings);
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> Option<OsString> {
        Some(s.into())
    }

    #[test]
    fn hof_config_names_the_file_outright() {
        let p = path_from(os("/etc/hof.toml"), os("/xdg"), os("/home/ada"));
        assert_eq!(p, Path::new("/etc/hof.toml"));
    }

    #[test]
    fn xdg_config_home_comes_next() {
        let p = path_from(None, os("/xdg"), os("/home/ada"));
        assert_eq!(p, Path::new("/xdg/hof/hof.toml"));
    }

    #[test]
    fn otherwise_the_file_is_under_dot_config_in_home() {
        let p = path_from(None, None, os("/home/ada"));
        assert_eq!(p, Path::new("/home/ada/.config/hof/hof.toml"));
    }

    #[test]
    fn empty_variables_count_as_unset() {
        let p = path_from(os(""), os(""), os("/home/ada"));
        assert_eq!(p, Path::new("/home/ada/.config/hof/hof.toml"));
    }

    #[test]
    fn themes_sit_beside_the_config_file() {
        let dir = themes_dir_for(Path::new("/home/ada/.config/hof/hof.toml"));
        assert_eq!(dir, Path::new("/home/ada/.config/hof/themes"));
    }
}
