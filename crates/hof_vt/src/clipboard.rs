//! What programs do with the user's clipboard through the terminal: OSC 52
//! (and the protocols libghostty folds into it) to write it, and OSC 52
//! with `?` to read it.
//!
//! A write libghostty hands over at once, and Hof puts it on the clipboard.
//! A read is different: answering gives the program whatever the user
//! copied last, a password as likely as not, so Hof asks first. libghostty
//! wants a read answered before its callback returns, while the parser is
//! locked and the screen can't be drawn, which is no time to ask. So Hof
//! leaves libghostty's read callback unset, which makes it pass reads over,
//! picks them out of the output itself, and answers through the program's
//! input once the user has said yes.

/// Which clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clipboard {
    /// The one Ctrl+C and Ctrl+V use.
    Standard,
    /// The selection: what the mouse selects, and the middle button
    /// pastes.
    Selection,
}

impl Clipboard {
    /// The clipboard an OSC 52 selector names: `p` and `s` the selection,
    /// `c`, the cut buffers and no selector the standard one, as most
    /// programs mean by it.
    pub(crate) fn of_selector(selector: &[u8]) -> Self {
        match selector.first() {
            Some(b'p' | b's') => Clipboard::Selection,
            _ => Clipboard::Standard,
        }
    }

    fn selector(self) -> char {
        match self {
            Clipboard::Standard => 'c',
            Clipboard::Selection => 'p',
        }
    }
}

/// The clipboard an OSC 52 body (what follows `52;`) asks to read, if it is
/// a read.
pub(crate) fn read_of(body: &[u8]) -> Option<Clipboard> {
    let (selector, data) = body.split_at(body.iter().position(|&b| b == b';')?);
    (&data[1..] == b"?").then(|| Clipboard::of_selector(selector))
}

/// The answer to a program that asked to read `from`: `text`, which is
/// empty when the user said no, so the program does not wait for ever.
pub fn read_answer(from: Clipboard, text: &str) -> Vec<u8> {
    let mut out = format!("\x1b]52;{};", from.selector()).into_bytes();
    base64(text.as_bytes(), &mut out);
    out.extend(b"\x1b\\");
    out
}

fn base64(bytes: &[u8], out: &mut Vec<u8>) {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                DIGITS[(n >> (18 - 6 * i) & 63) as usize]
            } else {
                b'='
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_names_its_clipboard_and_a_write_is_not_a_read() {
        assert_eq!(read_of(b"c;?"), Some(Clipboard::Standard));
        assert_eq!(read_of(b";?"), Some(Clipboard::Standard));
        assert_eq!(read_of(b"p;?"), Some(Clipboard::Selection));
        assert_eq!(read_of(b"c;aGk="), None);
        assert_eq!(read_of(b"?"), None);
    }

    #[test]
    fn an_answer_carries_the_text_in_base64() {
        assert_eq!(
            read_answer(Clipboard::Standard, "hi"),
            b"\x1b]52;c;aGk=\x1b\\"
        );
        assert_eq!(
            read_answer(Clipboard::Selection, "ljós"),
            b"\x1b]52;p;bGrDs3M=\x1b\\"
        );
        assert_eq!(read_answer(Clipboard::Standard, ""), b"\x1b]52;c;\x1b\\");
        let mut out = Vec::new();
        base64(b"abc", &mut out);
        assert_eq!(out, b"YWJj");
    }
}
