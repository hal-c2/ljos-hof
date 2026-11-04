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
//!
//! The same sequences serve the other end, a program asking its terminal,
//! which is what `hof copy` and `hof paste` are: [`carrying`] text to put
//! on a clipboard, [`read_request`], and [`answer_of`] to make out the
//! answer.

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

/// OSC 52 with `text` for `clipboard`: a program's write, and the answer
/// to its read, which is empty when the user said no, so the program does
/// not wait for ever.
pub fn carrying(clipboard: Clipboard, text: &str) -> Vec<u8> {
    let mut out = format!("\x1b]52;{};", clipboard.selector()).into_bytes();
    base64(text.as_bytes(), &mut out);
    out.extend(b"\x1b\\");
    out
}

/// What a program sends to read `from`.
pub fn read_request(from: Clipboard) -> Vec<u8> {
    format!("\x1b]52;{};?\x1b\\", from.selector()).into_bytes()
}

/// The text a terminal's answer to a read carries, from the first answer
/// in `bytes`, or `None` while `bytes` holds no whole answer. Whatever
/// else is in `bytes`, such as keys the user typed meanwhile, is passed
/// over. Text that is not UTF-8 is made so.
pub fn answer_of(bytes: &[u8]) -> Option<String> {
    const START: &[u8] = b"\x1b]52;";
    let at = bytes.windows(START.len()).position(|w| w == START)?;
    let body = &bytes[at + START.len()..];
    let end = body.iter().position(|&b| b == 0x07 || b == 0x1b)?;
    if body[end] == 0x1b && body.get(end + 1) != Some(&b'\\') {
        return None;
    }
    let body = &body[..end];
    let data = &body[body.iter().position(|&b| b == b';')? + 1..];
    Some(String::from_utf8_lossy(&unbase64(data)).into_owned())
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

/// Decodes base64, passing over what is not of it, as terminals send it
/// with no line breaks but some programs don't.
fn unbase64(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut n, mut bits) = (0u32, 0);
    for &c in text {
        let digit = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        n = n << 6 | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
            n &= (1 << bits) - 1;
        }
    }
    out
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
        assert_eq!(carrying(Clipboard::Standard, "hi"), b"\x1b]52;c;aGk=\x1b\\");
        assert_eq!(
            carrying(Clipboard::Selection, "ljós"),
            b"\x1b]52;p;bGrDs3M=\x1b\\"
        );
        assert_eq!(carrying(Clipboard::Standard, ""), b"\x1b]52;c;\x1b\\");
        let mut out = Vec::new();
        base64(b"abc", &mut out);
        assert_eq!(out, b"YWJj");
    }

    #[test]
    fn an_answer_is_made_out_amid_keys_and_only_once_whole() {
        let answer = carrying(Clipboard::Standard, "ljós and more");
        let mut typed = b"x\x1b[A".to_vec();
        typed.extend(&answer);
        assert_eq!(answer_of(&typed).as_deref(), Some("ljós and more"));
        assert_eq!(answer_of(&typed[..typed.len() - 1]), None, "not yet ended");
        assert_eq!(answer_of(b"\x1b]52;c;aGk=\x07").as_deref(), Some("hi"));
        assert_eq!(answer_of(b"\x1b]52;c;\x07").as_deref(), Some(""));
        for text in ["", "a", "ab", "abc", "abcd", "ljós"] {
            let mut out = Vec::new();
            base64(text.as_bytes(), &mut out);
            assert_eq!(unbase64(&out), text.as_bytes(), "{text}");
        }
    }

    #[test]
    fn a_read_request_is_a_read() {
        let request = read_request(Clipboard::Selection);
        assert_eq!(request, b"\x1b]52;p;?\x1b\\");
        assert_eq!(
            read_of(&request[5..request.len() - 2]),
            Some(Clipboard::Selection)
        );
    }
}
