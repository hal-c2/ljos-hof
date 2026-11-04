//! Changing one setting in `hof.toml`, as the settings window does, and
//! leaving the rest of the file as the user wrote it: comments, order and
//! spacing, and the comment after the value changed.

use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, Value};

/// A value for a setting.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    /// `true` or `false`.
    Bool(bool),
    /// A number: written as an integer when it is a whole one, which a
    /// setting that wants a fraction reads as well.
    Number(f64),
    /// A string.
    Text(String),
}

/// Why a setting could not be written.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    /// The key is not one of Hof's settings.
    #[error("`{0}` is not a setting")]
    Unknown(String),
    /// The number is not a number TOML can hold.
    #[error("`{0}` cannot be set to {1}")]
    BadNumber(String, f64),
    /// The file is not TOML, so changing it would lose what it says.
    #[error("{path} is not TOML, so it was left alone: {message}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// What the parser said.
        message: String,
    },
    /// The file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The error.
        source: io::Error,
    },
}

/// Sets `section.key` (`font`, `size`) in the file at `path` to `value`,
/// or with `None` takes the key out, back to its default. A file that does
/// not exist yet is started from the example ([`crate::EXAMPLE`]), as
/// `edit-config` starts it. The file is replaced whole, so a reader never
/// sees half of it.
pub fn set(path: &Path, section: &str, key: &str, value: Option<Setting>) -> Result<(), EditError> {
    let name = format!("{section}.{key}");
    if !crate::parse::is_setting(section, key) {
        return Err(EditError::Unknown(name));
    }
    let io = |source| EditError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = crate::parse::read_file(path)
        .map_err(|e| match e {
            crate::Error::Read { source, .. } => io(source),
            other => io(io::Error::other(other.to_string())),
        })?
        .unwrap_or_else(|| crate::EXAMPLE.to_owned());
    let mut doc: DocumentMut =
        text.parse()
            .map_err(|e: toml_edit::TomlError| EditError::Parse {
                path: path.to_path_buf(),
                message: e.message().to_owned(),
            })?;
    let value = match value {
        None => None,
        Some(v) => Some(to_value(&name, v)?),
    };
    apply(&mut doc, section, key, value);
    write(path, &doc.to_string()).map_err(io)
}

fn to_value(name: &str, value: Setting) -> Result<Value, EditError> {
    Ok(match value {
        Setting::Bool(b) => Value::from(b),
        Setting::Text(s) => Value::from(s),
        Setting::Number(n) if !n.is_finite() => {
            return Err(EditError::BadNumber(name.to_owned(), n));
        }
        // Within the range where every whole f64 is exact.
        Setting::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Value::from(n as i64),
        Setting::Number(n) => Value::from(n),
    })
}

fn apply(doc: &mut DocumentMut, section: &str, key: &str, value: Option<Value>) {
    let Some(value) = value else {
        if let Some(table) = doc.get_mut(section).and_then(Item::as_table_like_mut) {
            table.remove(key);
        }
        return;
    };
    if !doc.contains_key(section) {
        doc.insert(section, Item::Table(Table::new()));
    }
    let Some(table) = doc[section].as_table_like_mut() else {
        // `section = 3`: not a table, and the load says so; make it one.
        doc[section] = Item::Table(Table::new());
        return apply(doc, section, key, Some(value));
    };
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            // Its spacing and the comment after it stay, the comment in
            // its column.
            let mut decor = old.decor().clone();
            let suffix = decor.suffix().and_then(|s| s.as_str()).unwrap_or("");
            let comment = suffix.trim_start_matches([' ', '\t']).to_owned();
            let gap = suffix.len() - comment.len();
            if gap > 0 && comment.starts_with('#') {
                let pad = (gap + bare_len(old))
                    .saturating_sub(bare_len(&value))
                    .max(1);
                decor.set_suffix(format!("{}{comment}", " ".repeat(pad)));
            }
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}

/// How long `value` is written, without what is around it.
fn bare_len(value: &Value) -> usize {
    let mut bare = value.clone();
    *bare.decor_mut() = toml_edit::Decor::new("", "");
    bare.to_string().chars().count()
}

fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_owned());
    let mut temp = name;
    temp.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp);
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "hof-edit-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_value_changes_and_its_comments_stay() {
        let d = dir();
        let path = d.join("hof.toml");
        std::fs::write(
            &path,
            "# mine\n[font]\nfamily = \"Iosevka\"   # the best\nsize = 11.0 # points\n\n[window]\npadding = 2\n",
        )
        .unwrap();
        set(&path, "font", "size", Some(Setting::Number(14.0))).unwrap();
        set(
            &path,
            "font",
            "family",
            Some(Setting::Text("Fira Code".into())),
        )
        .unwrap();
        set(&path, "window", "opacity", Some(Setting::Number(0.9))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(
            text,
            "# mine\n[font]\nfamily = \"Fira Code\" # the best\nsize = 14   # points\n\n[window]\npadding = 2\nopacity = 0.9\n"
        );
        let loaded = crate::parse(&text).unwrap();
        assert_eq!(loaded.config.font.size, 14.0);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn a_section_the_file_lacks_is_added() {
        let d = dir();
        let path = d.join("hof.toml");
        std::fs::write(&path, "[font]\nsize = 12\n").unwrap();
        set(
            &path,
            "terminal",
            "cursor-shape",
            Some(Setting::Text("bar".into())),
        )
        .unwrap();
        set(&path, "session", "restore", Some(Setting::Bool(true))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        let c = crate::parse(&text).unwrap().config;
        assert_eq!(c.terminal.cursor_shape, crate::CursorShape::Bar);
        assert!(c.session.restore);
        assert_eq!(c.font.size, 12.0);
    }

    #[test]
    fn none_takes_the_key_out() {
        let d = dir();
        let path = d.join("hof.toml");
        std::fs::write(&path, "[theme]\nname = \"nord\"\n").unwrap();
        set(&path, "theme", "name", None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(text, "[theme]\n");
    }

    #[test]
    fn a_new_file_starts_as_the_example() {
        let d = dir();
        let path = d.join("sub").join("hof.toml");
        set(
            &path,
            "theme",
            "name",
            Some(Setting::Text("tokyo-night".into())),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(text.starts_with("# Ljós Hof configuration."), "{text}");
        assert!(
            text.contains("name = \"tokyo-night\"          # hof-dark"),
            "{text}"
        );
        let loaded = crate::parse(&text).unwrap();
        assert_eq!(loaded.config.theme.name, "tokyo-night");
    }

    #[test]
    fn what_is_not_a_setting_is_refused() {
        let d = dir();
        let path = d.join("hof.toml");
        for (section, key) in [("font", "sizee"), ("keys", "new-tab"), ("terminal", "env")] {
            assert!(
                matches!(
                    set(&path, section, key, Some(Setting::Bool(true))),
                    Err(EditError::Unknown(_))
                ),
                "{section}.{key}"
            );
        }
        assert!(!path.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_file_that_is_not_toml_is_left_alone() {
        let d = dir();
        let path = d.join("hof.toml");
        std::fs::write(&path, "[font\nsize = 1\n").unwrap();
        let err = set(&path, "font", "size", Some(Setting::Number(3.0))).unwrap_err();
        assert!(matches!(err, EditError::Parse { .. }), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[font\nsize = 1\n");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
