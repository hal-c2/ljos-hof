//! What can go wrong reading a configuration, and what is only worth
//! mentioning.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// A configuration that Hof cannot use at all.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file exists but could not be read (permissions, not UTF-8, ...).
    #[error("could not read {}: {source}", path.display())]
    Read {
        /// The file Hof tried to read.
        path: PathBuf,
        /// What the operating system said.
        #[source]
        source: io::Error,
    },
    /// The file is not valid TOML, or a value has the wrong type or form.
    #[error("{}:{line}:{column}: {message}", Origin(path.as_deref()))]
    Parse {
        /// The file, or `None` when the text came from [`crate::parse`].
        path: Option<PathBuf>,
        /// 1-based line of the offending text.
        line: usize,
        /// 1-based column, counted in characters.
        column: usize,
        /// What is wrong, naming the key when one is involved.
        message: String,
    },
}

/// Something in the configuration Hof ignored or adjusted so that it could
/// still start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// The file, or `None` when the text came from [`crate::parse`].
    pub path: Option<PathBuf>,
    /// 1-based line, when the warning is about a particular place in a file.
    pub line: Option<usize>,
    /// The dotted key path, e.g. `font.sise` or `keys.new-tab`.
    pub key: String,
    /// What was wrong.
    pub kind: WarningKind,
}

/// The reason for a [`Warning`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningKind {
    /// A key this version of Hof does not know; it is ignored, which lets an
    /// older Hof start with a file written for a newer one.
    UnknownKey,
    /// A `[keys]` entry names an action that does not exist.
    UnknownAction,
    /// A `[keys]` chord could not be read; the action keeps its default.
    BadChord {
        /// The chord as written.
        chord: String,
        /// Why it was rejected.
        reason: String,
    },
    /// A `[keys]` value is neither a string nor an array of strings.
    BadBinding,
    /// The same chord is bound to two actions; the one earlier in
    /// [`crate::Action::all`] keeps it.
    ChordConflict {
        /// The chord, normalised.
        chord: String,
        /// The action that keeps the chord.
        kept: String,
    },
    /// A number outside what makes sense, replaced by `used`.
    OutOfRange {
        /// What the file said.
        value: String,
        /// What Hof uses instead.
        used: String,
    },
    /// `theme.palette` has more than 16 entries; the rest are ignored.
    PaletteTooLong {
        /// How many entries the file gave.
        len: usize,
    },
    /// A `[commands.<name>]` entry that cannot run; it is left out.
    BadCommand {
        /// What is wrong with it.
        reason: String,
    },
    /// A plugin defines a command someone else already did; theirs stays.
    CommandConflict {
        /// Who keeps it: the config, or a plugin by name.
        kept: String,
    },
    /// A plugin file that could not be read; Hof starts without it.
    BadPlugin {
        /// Why.
        message: String,
    },
    /// `theme.name` is neither a built-in theme nor a theme file; Hof uses
    /// `hof-dark` instead.
    UnknownTheme {
        /// The name as written.
        name: String,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Origin(self.path.as_deref()))?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        let key = &self.key;
        match &self.kind {
            WarningKind::UnknownKey => write!(f, ": unknown key `{key}` ignored"),
            WarningKind::UnknownAction => write!(f, ": `{key}` is not an action"),
            WarningKind::BadChord { chord, reason } => {
                write!(f, ": `{key}`: cannot read chord `{chord}`: {reason}")
            }
            WarningKind::BadBinding => {
                write!(f, ": `{key}` must be a chord string or an array of them")
            }
            WarningKind::ChordConflict { chord, kept } => {
                write!(f, ": `{key}`: `{chord}` is already bound to `{kept}`")
            }
            WarningKind::OutOfRange { value, used } => {
                write!(f, ": `{key}` = {value} is out of range, using {used}")
            }
            WarningKind::PaletteTooLong { len } => {
                write!(f, ": `{key}` has {len} colours, only the first 16 are used")
            }
            WarningKind::BadCommand { reason } => {
                write!(f, ": `{key}` {reason}, so it is left out")
            }
            WarningKind::CommandConflict { kept } => {
                write!(f, ": `{key}` is already defined by {kept}, which keeps it")
            }
            WarningKind::BadPlugin { message } => write!(f, ": plugin not loaded: {message}"),
            WarningKind::UnknownTheme { name } => {
                write!(f, ": no theme called `{name}`, using hof-dark")
            }
        }
    }
}

/// Where a message comes from, for the `file:line` prefix editors and
/// terminals can jump to.
struct Origin<'a>(Option<&'a Path>);

impl fmt::Display for Origin<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(path) => write!(f, "{}", path.display()),
            None => f.write_str("<config>"),
        }
    }
}
