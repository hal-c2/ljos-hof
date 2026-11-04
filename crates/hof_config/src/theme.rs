//! Colours and the themes Hof ships with.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

/// A colour with alpha, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha; 255 is opaque.
    pub a: u8,
}

impl Rgba {
    /// An opaque colour from `0xrrggbb`, so theme tables read like the hex
    /// values their authors publish.
    pub const fn rgb(hex: u32) -> Self {
        Self {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
            a: 0xff,
        }
    }
}

/// Why a string is not a colour.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a colour; write #rrggbb or #rrggbbaa")]
pub struct BadColour(pub String);

impl FromStr for Rgba {
    type Err = BadColour;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || BadColour(s.to_owned());
        let hex = s.trim().strip_prefix('#').ok_or_else(bad)?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        Ok(Self {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
            a: if hex.len() == 8 { channel(6)? } else { 0xff },
        })
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 0xff {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The colours a terminal draws with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Default text colour.
    pub foreground: Rgba,
    /// Default background colour.
    pub background: Rgba,
    /// Cursor colour; `None` lets the UI invert the cell under it.
    pub cursor: Option<Rgba>,
    /// Selection background; `None` lets the UI invert the selected cells.
    pub selection: Option<Rgba>,
    /// The 16 ANSI colours: black, red, green, yellow, blue, magenta, cyan,
    /// white, then their bright forms in the same order.
    pub palette: [Rgba; 16],
}

impl Default for Theme {
    fn default() -> Self {
        HOF_DARK
    }
}

/// The name of the theme used when none is given or the given one is missing.
pub const DEFAULT_THEME: &str = "hof-dark";

const fn palette(hex: [u32; 16]) -> [Rgba; 16] {
    let mut out = [Rgba::rgb(0); 16];
    let mut i = 0;
    while i < 16 {
        out[i] = Rgba::rgb(hex[i]);
        i += 1;
    }
    out
}

// Hof is a shrine and Ljós is light: a lamp-lit room at night. The background
// is a deep blue-black, the text a warm off-white like candle-lit paper, and
// the cursor the amber of the flame.
const HOF_DARK: Theme = Theme {
    foreground: Rgba::rgb(0xe8e2d4),
    background: Rgba::rgb(0x0e1219),
    cursor: Some(Rgba::rgb(0xf2a93b)),
    selection: Some(Rgba::rgb(0x2a3446)),
    palette: palette([
        0x1b212c, 0xe0625a, 0x8fbf6a, 0xe6b85c, 0x6b9be0, 0xc48bd6, 0x5fbfb8, 0xcfc8b8, //
        0x4a5363, 0xf2837a, 0xaad889, 0xf5cf7e, 0x8fb6f0, 0xd9a8e8, 0x82d6cf, 0xf6f1e6,
    ]),
};

// The same room by day: warm paper, ink-dark text and a deeper amber cursor
// that still reads on a light page. Colours are darkened so they keep their
// contrast against the paper.
const HOF_LIGHT: Theme = Theme {
    foreground: Rgba::rgb(0x262a33),
    background: Rgba::rgb(0xf7f3ea),
    cursor: Some(Rgba::rgb(0xc47a12)),
    selection: Some(Rgba::rgb(0xd9e2f0)),
    palette: palette([
        0x262a33, 0xb53a33, 0x4f7f2c, 0x9a6a00, 0x2f5fa8, 0x8a4a9e, 0x1f7f78, 0xc8c1b3, //
        0x5c6370, 0xd0473f, 0x5f9636, 0xb98400, 0x3b72c4, 0xa35cb8, 0x279690, 0xe6dfd1,
    ]),
};

// Solarized, by Ethan Schoonover. Both variants share one ANSI palette; the
// bright slots hold the base tones, as in the published terminal files.
const SOLARIZED_PALETTE: [Rgba; 16] = palette([
    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, //
    0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
]);

const SOLARIZED_DARK: Theme = Theme {
    foreground: Rgba::rgb(0x839496),
    background: Rgba::rgb(0x002b36),
    cursor: Some(Rgba::rgb(0x93a1a1)),
    selection: Some(Rgba::rgb(0x073642)),
    palette: SOLARIZED_PALETTE,
};

const SOLARIZED_LIGHT: Theme = Theme {
    foreground: Rgba::rgb(0x657b83),
    background: Rgba::rgb(0xfdf6e3),
    cursor: Some(Rgba::rgb(0x586e75)),
    selection: Some(Rgba::rgb(0xeee8d5)),
    palette: SOLARIZED_PALETTE,
};

// Gruvbox, by Pavel Pertsev, dark variant at medium contrast.
const GRUVBOX_DARK: Theme = Theme {
    foreground: Rgba::rgb(0xebdbb2),
    background: Rgba::rgb(0x282828),
    cursor: Some(Rgba::rgb(0xebdbb2)),
    selection: Some(Rgba::rgb(0x504945)),
    palette: palette([
        0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, //
        0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
    ]),
};

// Nord, by Arctic Ice Studio: Polar Night, Snow Storm, Frost and Aurora.
const NORD: Theme = Theme {
    foreground: Rgba::rgb(0xd8dee9),
    background: Rgba::rgb(0x2e3440),
    cursor: Some(Rgba::rgb(0xd8dee9)),
    selection: Some(Rgba::rgb(0x434c5e)),
    palette: palette([
        0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, //
        0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
    ]),
};

// Dracula, by Zeno Rocha.
const DRACULA: Theme = Theme {
    foreground: Rgba::rgb(0xf8f8f2),
    background: Rgba::rgb(0x282a36),
    cursor: Some(Rgba::rgb(0xf8f8f2)),
    selection: Some(Rgba::rgb(0x44475a)),
    palette: palette([
        0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, //
        0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
    ]),
};

// Tokyo Night, by enkia, "night" variant.
const TOKYO_NIGHT: Theme = Theme {
    foreground: Rgba::rgb(0xc0caf5),
    background: Rgba::rgb(0x1a1b26),
    cursor: Some(Rgba::rgb(0xc0caf5)),
    selection: Some(Rgba::rgb(0x283457)),
    palette: palette([
        0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, //
        0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
    ]),
};

// Catppuccin, Mocha flavour.
const CATPPUCCIN_MOCHA: Theme = Theme {
    foreground: Rgba::rgb(0xcdd6f4),
    background: Rgba::rgb(0x1e1e2e),
    cursor: Some(Rgba::rgb(0xf5e0dc)),
    selection: Some(Rgba::rgb(0x585b70)),
    palette: palette([
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
        0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
    ]),
};

static THEMES: [(&str, Theme); 9] = [
    (DEFAULT_THEME, HOF_DARK),
    ("hof-light", HOF_LIGHT),
    ("solarized-dark", SOLARIZED_DARK),
    ("solarized-light", SOLARIZED_LIGHT),
    ("gruvbox-dark", GRUVBOX_DARK),
    ("nord", NORD),
    ("dracula", DRACULA),
    ("tokyo-night", TOKYO_NIGHT),
    ("catppuccin-mocha", CATPPUCCIN_MOCHA),
];

/// The built-in themes, by name, `hof-dark` first.
pub fn themes() -> &'static [(&'static str, Theme)] {
    &THEMES
}

/// Every theme `theme.name` may name: the built-in ones in their order,
/// then the files in `themes_dir` (`<name>.toml`) alphabetically. A file
/// named as a built-in theme takes its place, as it does when loaded.
pub fn theme_names(themes_dir: &std::path::Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(themes_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?;
            (path.extension()? == "toml" && path.is_file() && !stem.starts_with('.'))
                .then(|| stem.to_owned())
        })
        .filter(|name| builtin(name).is_none())
        .collect();
    files.sort();
    THEMES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .chain(files)
        .collect()
}

/// The built-in theme called `name`, if there is one.
pub fn builtin(name: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|(n, _)| *n == name).map(|(_, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_names_are_the_built_in_ones_then_the_files() {
        let dir = std::env::temp_dir().join(format!("hof-theme-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["zebra.toml", "alpha.toml", "nord.toml", "notes.txt"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let names = theme_names(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        let builtins: Vec<&str> = THEMES.iter().map(|(n, _)| *n).collect();
        assert_eq!(names[..builtins.len()], builtins[..]);
        assert_eq!(names[builtins.len()..], ["alpha", "zebra"]);
        assert_eq!(
            theme_names(std::path::Path::new("/nonexistent/hof")).len(),
            builtins.len()
        );
    }

    #[test]
    fn a_six_digit_hex_is_an_opaque_colour() {
        let c: Rgba = "#0E1219".parse().unwrap();
        assert_eq!(
            c,
            Rgba {
                r: 0x0e,
                g: 0x12,
                b: 0x19,
                a: 0xff
            }
        );
    }

    #[test]
    fn an_eight_digit_hex_carries_alpha() {
        let c: Rgba = "#11223380".parse().unwrap();
        assert_eq!(c.a, 0x80);
    }

    #[test]
    fn colours_print_back_as_they_were_written() {
        for text in ["#0e1219", "#11223380"] {
            assert_eq!(text.parse::<Rgba>().unwrap().to_string(), text);
        }
    }

    #[test]
    fn opaque_alpha_is_left_out_when_printing() {
        assert_eq!("#abcdefff".parse::<Rgba>().unwrap().to_string(), "#abcdef");
    }

    #[test]
    fn colours_without_a_hash_or_with_the_wrong_length_are_rejected() {
        for text in ["0e1219", "#fff", "#12345", "#1234567", "#ggggggg", "", "#"] {
            assert!(text.parse::<Rgba>().is_err(), "{text:?} parsed");
        }
    }

    #[test]
    fn multibyte_text_is_not_a_colour_and_does_not_panic() {
        assert!("#ééé".parse::<Rgba>().is_err());
    }

    #[test]
    fn every_promised_theme_is_built_in() {
        let names: Vec<_> = themes().iter().map(|(n, _)| *n).collect();
        for name in [
            "hof-dark",
            "hof-light",
            "solarized-dark",
            "solarized-light",
            "gruvbox-dark",
            "nord",
            "dracula",
            "tokyo-night",
            "catppuccin-mocha",
        ] {
            assert!(names.contains(&name), "{name} missing");
        }
    }

    #[test]
    fn the_default_theme_is_hof_dark() {
        assert_eq!(&Theme::default(), builtin(DEFAULT_THEME).unwrap());
        assert_eq!(themes()[0].0, DEFAULT_THEME);
    }

    #[test]
    fn hof_dark_has_an_amber_cursor_on_a_dark_blue_background() {
        let t = builtin("hof-dark").unwrap();
        let cursor = t.cursor.unwrap();
        assert!(
            cursor.r > cursor.g && cursor.g > cursor.b,
            "amber is red over green over blue"
        );
        assert!(t.background.b > t.background.r && t.background.r < 0x20);
    }

    #[test]
    fn built_in_theme_names_are_unique() {
        let mut names: Vec<_> = themes().iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), themes().len());
    }
}
