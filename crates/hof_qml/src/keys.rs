//! Qt's key events as libghostty's keys.
//!
//! Qt names a key by what it types (Shift+1 is `Key_Exclam`), libghostty by
//! where it is on the keyboard (`Digit1`), because the Kitty keyboard
//! protocol reports physical keys. Shifted symbols are folded back onto the
//! key that types them on a US layout; for other layouts the typed text
//! still goes along, and that is what legacy encodings send.

use hof_vt::{Key, KeyInput, Mods};
use ljos::item::{KeyEvent, KeyKind, Modifiers, key};

/// The key `ev` stands for, or `None` for one a terminal has no use for
/// (volume keys, say). `mac` swaps Control and Command back: Qt on macOS
/// reports Command as Control, and a terminal wants the real Control.
pub fn translate(ev: &KeyEvent, mac: bool) -> Option<KeyInput> {
    let keypad = ev.modifiers.contains(Modifiers::KEYPAD);
    let (code, unshifted) = physical(ev.key, keypad)?;
    let mods = mods(ev.modifiers, mac) | backtab_shift(ev.key);
    let text: String = ev.text.chars().filter(|c| !c.is_control()).collect();
    let mut input = KeyInput::press(code).with_mods(mods);
    if !text.is_empty() {
        input = input.with_text(text);
        // The layout spent Shift on the character: legacy encodings must not
        // report it again.
        input.consumed = mods & Mods::SHIFT;
    }
    input.unshifted = unshifted.or(input.unshifted);
    input.release = ev.kind == KeyKind::Release;
    input.repeat = ev.auto_repeat;
    Some(input)
}

/// Qt's modifiers as the terminal's. On a Mac Qt calls Command Control and
/// Control Meta; the terminal wants them the other way round.
pub(crate) fn mods(m: Modifiers, mac: bool) -> Mods {
    let mut out = Mods::empty();
    out.set(Mods::SHIFT, m.contains(Modifiers::SHIFT));
    out.set(Mods::ALT, m.contains(Modifiers::ALT));
    let (ctrl, sup) = if mac {
        (Modifiers::META, Modifiers::CONTROL)
    } else {
        (Modifiers::CONTROL, Modifiers::META)
    };
    out.set(Mods::CTRL, m.contains(ctrl));
    out.set(Mods::SUPER, m.contains(sup));
    out
}

/// Qt folds Shift+Tab into a key of its own; the terminal wants Tab, shifted.
fn backtab_shift(k: i32) -> Mods {
    if k == key::BACKTAB {
        Mods::SHIFT
    } else {
        Mods::empty()
    }
}

/// The physical key for Qt key `k`, and the character it types unshifted.
fn physical(k: i32, keypad: bool) -> Option<(Key, Option<char>)> {
    if (key::F1..key::F1 + 25).contains(&k) {
        return Some((FUNCTION[(k - key::F1) as usize], None));
    }
    if let Some(found) = special(k, keypad) {
        return Some((found, None));
    }
    let c = u32::try_from(k).ok().and_then(char::from_u32)?;
    if keypad && let Some(found) = numpad(c) {
        return Some((found, Some(c)));
    }
    if c.is_ascii_alphabetic() {
        let i = (c.to_ascii_uppercase() as u8 - b'A') as usize;
        return Some((LETTERS[i], Some(c.to_ascii_lowercase())));
    }
    if let Some(&(_, base, found)) = SYMBOLS.iter().find(|(s, b, _)| *s == c || *b == c) {
        return Some((found, Some(base)));
    }
    // A key of another layout (é, ß): the text says what it types.
    (!c.is_control()).then_some((Key::Unidentified, None))
}

fn special(k: i32, keypad: bool) -> Option<Key> {
    Some(match k {
        key::ESCAPE => Key::Escape,
        key::TAB | key::BACKTAB => Key::Tab,
        key::BACKSPACE => Key::Backspace,
        key::RETURN => Key::Enter,
        key::ENTER => Key::NumpadEnter,
        key::INSERT if keypad => Key::NumpadInsert,
        key::INSERT => Key::Insert,
        key::DELETE if keypad => Key::NumpadDelete,
        key::DELETE => Key::Delete,
        key::HOME if keypad => Key::NumpadHome,
        key::HOME => Key::Home,
        key::END if keypad => Key::NumpadEnd,
        key::END => Key::End,
        key::LEFT if keypad => Key::NumpadLeft,
        key::LEFT => Key::ArrowLeft,
        key::RIGHT if keypad => Key::NumpadRight,
        key::RIGHT => Key::ArrowRight,
        key::UP if keypad => Key::NumpadUp,
        key::UP => Key::ArrowUp,
        key::DOWN if keypad => Key::NumpadDown,
        key::DOWN => Key::ArrowDown,
        key::PAGE_UP if keypad => Key::NumpadPageUp,
        key::PAGE_UP => Key::PageUp,
        key::PAGE_DOWN if keypad => Key::NumpadPageDown,
        key::PAGE_DOWN => Key::PageDown,
        key::CLEAR if keypad => Key::NumpadBegin,
        key::SPACE => Key::Space,
        key::MENU => Key::ContextMenu,
        key::SHIFT => Key::ShiftLeft,
        key::CONTROL => Key::ControlLeft,
        key::ALT | key::ALT_GR => Key::AltLeft,
        key::META | key::SUPER_L => Key::MetaLeft,
        key::SUPER_R => Key::MetaRight,
        key::CAPS_LOCK => Key::CapsLock,
        key::NUM_LOCK => Key::NumLock,
        key::SCROLL_LOCK => Key::ScrollLock,
        key::PAUSE => Key::Pause,
        key::PRINT => Key::PrintScreen,
        key::COPY => Key::Copy,
        key::CUT => Key::Cut,
        key::PASTE => Key::Paste,
        _ => return None,
    })
}

fn numpad(c: char) -> Option<Key> {
    Some(match c {
        '0'..='9' => NUMPAD[(c as u8 - b'0') as usize],
        '+' => Key::NumpadAdd,
        '-' => Key::NumpadSubtract,
        '*' => Key::NumpadMultiply,
        '/' => Key::NumpadDivide,
        '.' => Key::NumpadDecimal,
        ',' => Key::NumpadComma,
        '=' => Key::NumpadEqual,
        _ => return None,
    })
}

const LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];

const NUMPAD: [Key; 10] = [
    Key::Numpad0,
    Key::Numpad1,
    Key::Numpad2,
    Key::Numpad3,
    Key::Numpad4,
    Key::Numpad5,
    Key::Numpad6,
    Key::Numpad7,
    Key::Numpad8,
    Key::Numpad9,
];

const FUNCTION: [Key; 25] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
    Key::F13,
    Key::F14,
    Key::F15,
    Key::F16,
    Key::F17,
    Key::F18,
    Key::F19,
    Key::F20,
    Key::F21,
    Key::F22,
    Key::F23,
    Key::F24,
    Key::F25,
];

/// (shifted, unshifted, key) on a US layout.
const SYMBOLS: [(char, char, Key); 21] = [
    (')', '0', Key::Digit0),
    ('!', '1', Key::Digit1),
    ('@', '2', Key::Digit2),
    ('#', '3', Key::Digit3),
    ('$', '4', Key::Digit4),
    ('%', '5', Key::Digit5),
    ('^', '6', Key::Digit6),
    ('&', '7', Key::Digit7),
    ('*', '8', Key::Digit8),
    ('(', '9', Key::Digit9),
    ('~', '`', Key::Backquote),
    ('_', '-', Key::Minus),
    ('+', '=', Key::Equal),
    ('{', '[', Key::BracketLeft),
    ('}', ']', Key::BracketRight),
    ('|', '\\', Key::Backslash),
    (':', ';', Key::Semicolon),
    ('"', '\'', Key::Quote),
    ('<', ',', Key::Comma),
    ('>', '.', Key::Period),
    ('?', '/', Key::Slash),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn press(k: i32, m: Modifiers, text: &str) -> KeyInput {
        let ev = KeyEvent::press(k).with_modifiers(m).with_text(text);
        translate(&ev, false).expect("a terminal key")
    }

    #[test]
    fn shifted_symbols_are_the_keys_that_type_them() {
        let bang = press('!' as i32, Modifiers::SHIFT, "!");
        assert_eq!(bang.key, Key::Digit1);
        assert_eq!(bang.unshifted, Some('1'));
        assert_eq!(bang.consumed, Mods::SHIFT);
        assert_eq!(bang.text.as_deref(), Some("!"));
    }

    #[test]
    fn control_characters_are_not_text() {
        let c = press(key::A + 2, Modifiers::CONTROL, "\u{3}");
        assert_eq!(c.key, Key::C);
        assert_eq!(c.mods, Mods::CTRL);
        assert_eq!(c.text, None);
        assert_eq!(c.unshifted, Some('c'));
    }

    #[test]
    fn backtab_is_shift_tab() {
        let t = press(key::BACKTAB, Modifiers::SHIFT, "");
        assert_eq!((t.key, t.mods), (Key::Tab, Mods::SHIFT));
    }

    #[test]
    fn mac_swaps_command_and_control() {
        let ev = KeyEvent::press(key::A).with_modifiers(Modifiers::META);
        assert_eq!(translate(&ev, true).unwrap().mods, Mods::CTRL);
        let ev = KeyEvent::press(key::A).with_modifiers(Modifiers::CONTROL);
        assert_eq!(translate(&ev, true).unwrap().mods, Mods::SUPER);
    }

    #[test]
    fn keypad_keys_are_the_numpad() {
        let ev = KeyEvent::press('5' as i32)
            .with_modifiers(Modifiers::KEYPAD)
            .with_text("5");
        assert_eq!(translate(&ev, false).unwrap().key, Key::Numpad5);
        let f5 = press(key::F1 + 4, Modifiers::empty(), "");
        assert_eq!(f5.key, Key::F5);
    }
}
