//! Key chords and the actions they trigger.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// Something the user can bind a key to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// Open a tab.
    NewTab,
    /// Close the current tab and every pane in it.
    CloseTab,
    /// Go to the tab on the right.
    NextTab,
    /// Go to the tab on the left.
    PreviousTab,
    /// Split the current pane, the new one to the right.
    SplitRight,
    /// Split the current pane, the new one below.
    SplitDown,
    /// Close the current pane.
    ClosePane,
    /// Move focus to the pane on the left.
    FocusLeft,
    /// Move focus to the pane on the right.
    FocusRight,
    /// Move focus to the pane above.
    FocusUp,
    /// Move focus to the pane below.
    FocusDown,
    /// Move the current pane's left border left.
    GrowLeft,
    /// Move the current pane's right border right.
    GrowRight,
    /// Move the current pane's top border up.
    GrowUp,
    /// Move the current pane's bottom border down.
    GrowDown,
    /// Show the current pane alone, or put it back.
    ZoomPane,
    /// Copy the selection.
    Copy,
    /// Paste the clipboard.
    Paste,
    /// Select everything, scrollback too.
    SelectAll,
    /// Scroll back one page.
    ScrollPageUp,
    /// Scroll forward one page.
    ScrollPageDown,
    /// Scroll to the oldest line.
    ScrollTop,
    /// Scroll to the live screen.
    ScrollBottom,
    /// Scroll to the previous shell prompt.
    PreviousPrompt,
    /// Scroll to the next shell prompt.
    NextPrompt,
    /// Make the font bigger.
    FontBigger,
    /// Make the font smaller.
    FontSmaller,
    /// Put the font back to the configured size.
    FontReset,
    /// Search the scrollback.
    Search,
    /// Reset the terminal: modes, colours and screen, as a new one.
    Reset,
    /// Read the configuration file again.
    ReloadConfig,
    /// Hide the dropdown (in a dropdown); `hof --toggle` shows it again.
    ToggleDropdown,
    /// Enter or leave fullscreen.
    ToggleFullscreen,
    /// Go to the next pane whose agent is waiting for you, in any tab.
    NextWaiting,
    /// Pick a machine, this one or one of `[hosts]`, and open a tab there.
    NewTabOnHost,
    /// Pick a machine and split the pane to the right with a pane there.
    SplitRightOnHost,
    /// Pick a machine and split the pane downward with a pane there.
    SplitDownOnHost,
}

const ACTION_NAMES: [(Action, &str); 37] = [
    (Action::NewTab, "new-tab"),
    (Action::CloseTab, "close-tab"),
    (Action::NextTab, "next-tab"),
    (Action::PreviousTab, "previous-tab"),
    (Action::SplitRight, "split-right"),
    (Action::SplitDown, "split-down"),
    (Action::ClosePane, "close-pane"),
    (Action::FocusLeft, "focus-left"),
    (Action::FocusRight, "focus-right"),
    (Action::FocusUp, "focus-up"),
    (Action::FocusDown, "focus-down"),
    (Action::GrowLeft, "grow-left"),
    (Action::GrowRight, "grow-right"),
    (Action::GrowUp, "grow-up"),
    (Action::GrowDown, "grow-down"),
    (Action::ZoomPane, "zoom-pane"),
    (Action::Copy, "copy"),
    (Action::Paste, "paste"),
    (Action::SelectAll, "select-all"),
    (Action::ScrollPageUp, "scroll-page-up"),
    (Action::ScrollPageDown, "scroll-page-down"),
    (Action::ScrollTop, "scroll-top"),
    (Action::ScrollBottom, "scroll-bottom"),
    (Action::PreviousPrompt, "previous-prompt"),
    (Action::NextPrompt, "next-prompt"),
    (Action::FontBigger, "font-bigger"),
    (Action::FontSmaller, "font-smaller"),
    (Action::FontReset, "font-reset"),
    (Action::Search, "search"),
    (Action::Reset, "reset"),
    (Action::ReloadConfig, "reload-config"),
    (Action::ToggleDropdown, "toggle-dropdown"),
    (Action::ToggleFullscreen, "toggle-fullscreen"),
    (Action::NextWaiting, "next-waiting"),
    (Action::NewTabOnHost, "new-tab-on-host"),
    (Action::SplitRightOnHost, "split-right-on-host"),
    (Action::SplitDownOnHost, "split-down-on-host"),
];

impl Action {
    /// Every action, in the order they are documented.
    pub fn all() -> impl Iterator<Item = Action> {
        ACTION_NAMES.iter().map(|(a, _)| *a)
    }

    /// The kebab-case name used in `[keys]`.
    pub fn name(self) -> &'static str {
        ACTION_NAMES
            .iter()
            .find(|(a, _)| *a == self)
            .map(|(_, n)| *n)
            .expect("every action has a name")
    }
}

/// The name is not an action.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not an action")]
pub struct UnknownAction(pub String);

impl FromStr for Action {
    type Err = UnknownAction;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ACTION_NAMES
            .iter()
            .find(|(_, n)| *n == s)
            .map(|(a, _)| *a)
            .ok_or_else(|| UnknownAction(s.to_owned()))
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A key with the modifiers held down for it, e.g. `ctrl+shift+t`.
///
/// `key` is lowercase and normalised: a single character (`t`, `0`, `=`), or
/// a name from [`Chord::KEY_NAMES`]. `+` and `-` are spelled `plus` and
/// `minus` because `+` separates the parts of a chord. A shifted key is
/// written as its unshifted key plus `shift`, so `ctrl+shift+minus`, not
/// `ctrl+_`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    /// Control held.
    pub ctrl: bool,
    /// Shift held.
    pub shift: bool,
    /// Alt (Option on a Mac) held.
    pub alt: bool,
    /// Super (the logo key, Command on a Mac) held.
    pub super_: bool,
    /// The key itself, normalised.
    pub key: String,
}

impl Chord {
    /// Every multi-character key name a chord may use.
    pub const KEY_NAMES: &[&str] = &[
        "enter",
        "tab",
        "space",
        "escape",
        "backspace",
        "delete",
        "insert",
        "home",
        "end",
        "pageup",
        "pagedown",
        "up",
        "down",
        "left",
        "right",
        "plus",
        "minus",
        "menu",
        "print",
        "pause",
    ];

    /// Highest function key a chord may name.
    pub const MAX_FUNCTION_KEY: u8 = 35;
}

/// Why a chord could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BadChord {
    /// Nothing was written.
    #[error("the chord is empty")]
    Empty,
    /// Only modifiers, or a `+` with nothing after it.
    #[error("no key after the modifiers")]
    MissingKey,
    /// A modifier part that is not ctrl, shift, alt or super.
    #[error("`{0}` is not a modifier (use ctrl, shift, alt or super)")]
    UnknownModifier(String),
    /// The same modifier twice.
    #[error("`{0}` is given twice")]
    RepeatedModifier(String),
    /// A key name Hof does not know.
    #[error("`{0}` is not a key")]
    UnknownKey(String),
}

impl FromStr for Chord {
    type Err = BadChord;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(BadChord::Empty);
        }
        // `+` separates parts, so a `+` key can only be the last part, as in
        // `ctrl++`; peel it off before splitting.
        let (modifiers, key) = if s == "+" {
            ("", "+")
        } else if let Some(rest) = s.strip_suffix("++") {
            (rest, "+")
        } else {
            s.rsplit_once('+').unwrap_or(("", s))
        };

        let mut chord = Chord {
            ctrl: false,
            shift: false,
            alt: false,
            super_: false,
            key: normalise_key(key)?,
        };
        if !modifiers.is_empty() {
            for part in modifiers.split('+') {
                let part = part.trim().to_ascii_lowercase();
                let slot = match part.as_str() {
                    "ctrl" | "control" => &mut chord.ctrl,
                    "shift" => &mut chord.shift,
                    "alt" | "option" => &mut chord.alt,
                    "super" | "cmd" | "command" | "logo" | "win" => &mut chord.super_,
                    "" => return Err(BadChord::MissingKey),
                    _ => return Err(BadChord::UnknownModifier(part)),
                };
                if *slot {
                    return Err(BadChord::RepeatedModifier(part));
                }
                *slot = true;
            }
        }
        Ok(chord)
    }
}

fn normalise_key(key: &str) -> Result<String, BadChord> {
    let key = key.trim();
    if key.is_empty() {
        return Err(BadChord::MissingKey);
    }
    let lower = key.to_lowercase();
    let alias = match lower.as_str() {
        "+" => "plus",
        "-" => "minus",
        "return" => "enter",
        "esc" => "escape",
        "pgup" | "page-up" | "page_up" | "prior" => "pageup",
        "pgdn" | "pgdown" | "page-down" | "page_down" | "next" => "pagedown",
        "del" => "delete",
        "ins" => "insert",
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        other => other,
    };
    if Chord::KEY_NAMES.contains(&alias) || is_function_key(alias) {
        return Ok(alias.to_owned());
    }
    let mut chars = alias.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_whitespace() && !c.is_control() => Ok(alias.to_owned()),
        _ => Err(BadChord::UnknownKey(key.to_owned())),
    }
}

fn is_function_key(name: &str) -> bool {
    name.strip_prefix('f')
        .filter(|n| !n.starts_with('0'))
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=Chord::MAX_FUNCTION_KEY).contains(&n))
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
            (self.super_, "super+"),
        ] {
            if held {
                f.write_str(name)?;
            }
        }
        f.write_str(&self.key)
    }
}

// Ctrl+Shift for anything a shell or a TUI might want for itself, so plain
// Ctrl and Alt chords reach the program running in the terminal.
const DEFAULTS: &[(Action, &[&str])] = &[
    (Action::NewTab, &["ctrl+shift+t"]),
    (Action::NextTab, &["ctrl+tab", "ctrl+pagedown"]),
    (Action::PreviousTab, &["ctrl+shift+tab", "ctrl+pageup"]),
    (Action::SplitRight, &["ctrl+shift+enter"]),
    (Action::SplitDown, &["ctrl+shift+minus"]),
    (Action::ClosePane, &["ctrl+shift+w"]),
    (Action::FocusLeft, &["ctrl+shift+left"]),
    (Action::FocusRight, &["ctrl+shift+right"]),
    (Action::FocusUp, &["ctrl+shift+up"]),
    (Action::FocusDown, &["ctrl+shift+down"]),
    (Action::GrowLeft, &["ctrl+alt+shift+left"]),
    (Action::GrowRight, &["ctrl+alt+shift+right"]),
    (Action::GrowUp, &["ctrl+alt+shift+up"]),
    (Action::GrowDown, &["ctrl+alt+shift+down"]),
    (Action::ZoomPane, &["ctrl+shift+m"]),
    (Action::Copy, &["ctrl+shift+c"]),
    (Action::Paste, &["ctrl+shift+v", "shift+insert"]),
    (Action::ScrollPageUp, &["shift+pageup"]),
    (Action::ScrollPageDown, &["shift+pagedown"]),
    (Action::ScrollTop, &["shift+home"]),
    (Action::ScrollBottom, &["shift+end"]),
    (Action::PreviousPrompt, &["ctrl+shift+z"]),
    (Action::NextPrompt, &["ctrl+shift+x"]),
    // `=` is where `+` lives unshifted on most layouts.
    (Action::FontBigger, &["ctrl+plus", "ctrl+="]),
    (Action::FontSmaller, &["ctrl+minus"]),
    (Action::FontReset, &["ctrl+0"]),
    (Action::Search, &["ctrl+shift+f"]),
    (Action::ReloadConfig, &["ctrl+shift+r"]),
    (Action::ToggleFullscreen, &["f11"]),
    (Action::NextWaiting, &["ctrl+shift+a"]),
    (Action::NewTabOnHost, &["ctrl+shift+h"]),
];

/// The `[keys]` section: the user's changes to the default bindings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keys {
    /// Actions the user rebound, with their chords. An empty list means the
    /// action was unbound with `"none"`.
    pub overrides: BTreeMap<Action, Vec<Chord>>,
}

impl Keys {
    /// The bindings Hof has before any `[keys]` section.
    pub fn defaults() -> Vec<(Chord, Action)> {
        DEFAULTS
            .iter()
            .flat_map(|(action, chords)| {
                chords
                    .iter()
                    .map(|c| (c.parse().expect("default chords parse"), *action))
            })
            .collect()
    }

    /// The bindings in effect: the defaults with the user's entries applied.
    ///
    /// An action the user set loses all its default chords; a chord the user
    /// gave to some action is taken from whatever had it by default. Each
    /// chord appears at most once.
    pub fn bindings(&self) -> Vec<(Chord, Action)> {
        let mut out: Vec<(Chord, Action)> = Vec::new();
        for (action, chords) in &self.overrides {
            for chord in chords {
                if !out.iter().any(|(c, _)| c == chord) {
                    out.push((chord.clone(), *action));
                }
            }
        }
        let user_count = out.len();
        for (chord, action) in Self::defaults() {
            let overridden = self.overrides.contains_key(&action);
            let taken = out[..user_count].iter().any(|(c, _)| *c == chord);
            if !overridden && !taken {
                out.push((chord, action));
            }
        }
        out
    }

    /// The action bound to `chord`, if any.
    pub fn action_for(&self, chord: &Chord) -> Option<Action> {
        self.bindings()
            .into_iter()
            .find(|(c, _)| c == chord)
            .map(|(_, a)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn action_names_round_trip() {
        for action in Action::all() {
            assert_eq!(action.to_string().parse::<Action>(), Ok(action));
        }
        assert_eq!(Action::all().count(), ACTION_NAMES.len());
    }

    #[test]
    fn action_names_are_kebab_case() {
        assert_eq!(Action::ScrollPageUp.to_string(), "scroll-page-up");
        assert_eq!("toggle-fullscreen".parse(), Ok(Action::ToggleFullscreen));
    }

    #[test]
    fn an_unknown_action_name_is_rejected() {
        assert!("new_tab".parse::<Action>().is_err());
    }

    #[test]
    fn modifiers_are_read_in_any_order_and_case() {
        let c = chord("Shift+CTRL+T");
        assert!(c.ctrl && c.shift && !c.alt && !c.super_);
        assert_eq!(c.key, "t");
        assert_eq!(c, chord("ctrl+shift+t"));
    }

    #[test]
    fn chords_print_in_a_fixed_modifier_order() {
        assert_eq!(
            chord("super+shift+alt+ctrl+left").to_string(),
            "ctrl+alt+shift+super+left"
        );
    }

    #[test]
    fn plus_can_be_spelled_out_or_typed() {
        assert_eq!(chord("ctrl+plus").key, "plus");
        assert_eq!(chord("ctrl++"), chord("ctrl+plus"));
        assert_eq!(chord("+").key, "plus");
    }

    #[test]
    fn minus_is_normalised_to_its_name() {
        assert_eq!(chord("ctrl+-"), chord("ctrl+minus"));
    }

    #[test]
    fn key_aliases_are_normalised() {
        assert_eq!(chord("alt+Return").key, "enter");
        assert_eq!(chord("shift+PgUp").key, "pageup");
        assert_eq!(chord("esc").key, "escape");
        assert_eq!(chord("F1").key, "f1");
        assert_eq!(chord("cmd+left"), chord("super+left"));
    }

    #[test]
    fn single_characters_are_keys() {
        assert_eq!(chord("ctrl+0").key, "0");
        assert_eq!(chord("ctrl+=").key, "=");
        assert_eq!(chord("alt+Ö").key, "ö");
    }

    #[test]
    fn printed_chords_parse_back_to_themselves() {
        for (c, _) in Keys::defaults() {
            assert_eq!(c.to_string().parse::<Chord>(), Ok(c));
        }
    }

    #[test]
    fn malformed_chords_say_why() {
        assert_eq!("".parse::<Chord>(), Err(BadChord::Empty));
        assert_eq!("ctrl+".parse::<Chord>(), Err(BadChord::MissingKey));
        assert_eq!("ctrl++shift+t".parse::<Chord>(), Err(BadChord::MissingKey));
        assert_eq!(
            "hyper+t".parse::<Chord>(),
            Err(BadChord::UnknownModifier("hyper".into()))
        );
        assert_eq!(
            "ctrl+ctrl+t".parse::<Chord>(),
            Err(BadChord::RepeatedModifier("ctrl".into()))
        );
        assert_eq!(
            "ctrl+banana".parse::<Chord>(),
            Err(BadChord::UnknownKey("banana".into()))
        );
    }

    #[test]
    fn function_keys_stop_at_f35() {
        assert!("f35".parse::<Chord>().is_ok());
        assert!("f36".parse::<Chord>().is_err());
        assert!("f0".parse::<Chord>().is_err());
        assert!("f01".parse::<Chord>().is_err());
    }

    #[test]
    fn the_defaults_bind_the_promised_chords() {
        let keys = Keys::default();
        for (c, action) in [
            ("ctrl+shift+t", Action::NewTab),
            ("ctrl+shift+w", Action::ClosePane),
            ("ctrl+shift+enter", Action::SplitRight),
            ("ctrl+shift+minus", Action::SplitDown),
            ("ctrl+shift+left", Action::FocusLeft),
            ("ctrl+tab", Action::NextTab),
            ("ctrl+shift+tab", Action::PreviousTab),
            ("ctrl+plus", Action::FontBigger),
            ("ctrl+minus", Action::FontSmaller),
            ("ctrl+0", Action::FontReset),
            ("ctrl+shift+z", Action::PreviousPrompt),
            ("ctrl+shift+x", Action::NextPrompt),
            ("ctrl+shift+f", Action::Search),
            ("ctrl+shift+r", Action::ReloadConfig),
            ("f11", Action::ToggleFullscreen),
            ("ctrl+shift+c", Action::Copy),
            ("ctrl+shift+v", Action::Paste),
            ("shift+pageup", Action::ScrollPageUp),
            ("shift+pagedown", Action::ScrollPageDown),
            ("shift+home", Action::ScrollTop),
            ("shift+end", Action::ScrollBottom),
        ] {
            assert_eq!(keys.action_for(&chord(c)), Some(action), "{c}");
        }
    }

    #[test]
    fn no_chord_is_bound_twice_by_default() {
        let mut chords: Vec<_> = Keys::defaults().into_iter().map(|(c, _)| c).collect();
        let n = chords.len();
        chords.sort();
        chords.dedup();
        assert_eq!(chords.len(), n);
    }

    #[test]
    fn rebinding_an_action_drops_its_default_chord() {
        let mut keys = Keys::default();
        keys.overrides.insert(Action::NewTab, vec![chord("ctrl+t")]);
        assert_eq!(keys.action_for(&chord("ctrl+t")), Some(Action::NewTab));
        assert_eq!(keys.action_for(&chord("ctrl+shift+t")), None);
    }

    #[test]
    fn an_empty_override_unbinds_the_action() {
        let mut keys = Keys::default();
        keys.overrides.insert(Action::Search, vec![]);
        assert!(keys.bindings().iter().all(|(_, a)| *a != Action::Search));
    }

    #[test]
    fn a_user_chord_takes_over_from_a_default_action() {
        let mut keys = Keys::default();
        keys.overrides
            .insert(Action::NewTab, vec![chord("ctrl+shift+w")]);
        assert_eq!(
            keys.action_for(&chord("ctrl+shift+w")),
            Some(Action::NewTab)
        );
        let close_pane = keys
            .bindings()
            .into_iter()
            .filter(|(_, a)| *a == Action::ClosePane);
        assert_eq!(close_pane.count(), 0);
    }
}
