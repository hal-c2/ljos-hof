//! Input as the UI describes it, before the session encodes it for the
//! program in whatever keyboard and mouse protocols the program turned on.

use libghostty_vt::key::{Key, Mods};

/// One key press, repeat or release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    /// The physical key, by its W3C code name.
    pub key: Key,
    pub mods: Mods,
    /// Modifiers the keyboard layout used up producing `text` (Shift for a
    /// capital letter, say), which legacy encodings must not report again.
    pub consumed: Mods,
    pub release: bool,
    pub repeat: bool,
    /// The text the key produces in the current layout, if any.
    pub text: Option<String>,
    /// The character the key produces without modifiers, which the Kitty
    /// keyboard protocol reports alongside.
    pub unshifted: Option<char>,
    /// The key is part of an unfinished input-method composition.
    pub composing: bool,
}

impl KeyInput {
    pub fn press(key: Key) -> Self {
        Self {
            key,
            mods: Mods::empty(),
            consumed: Mods::empty(),
            release: false,
            repeat: false,
            text: None,
            unshifted: None,
            composing: false,
        }
    }

    #[must_use]
    pub fn with_mods(mut self, mods: Mods) -> Self {
        self.mods = mods;
        self
    }

    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.unshifted = self.unshifted.or_else(|| {
            text.chars()
                .next()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
        });
        self.text = Some(text);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// Wheel up.
    WheelUp,
    /// Wheel down.
    WheelDown,
    WheelLeft,
    WheelRight,
    Back,
    Forward,
}

/// A mouse event over the grid, in device pixels from the grid's top-left
/// (inside any padding).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseInput {
    pub action: MouseAction,
    /// `None` for motion with no button down.
    pub button: Option<MouseButton>,
    pub mods: Mods,
    pub x: f32,
    pub y: f32,
}
