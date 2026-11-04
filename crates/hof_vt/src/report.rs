//! Hof's own escape sequence, by which a program says how its agent is:
//!
//! ```text
//! ESC ] 7701 ; <state> [; <name>] BEL      (or ST, ESC \, in place of BEL)
//! ```
//!
//! `state` is `working`, `waiting`, `idle` or `none`, and `name` the agent's,
//! for a program the terminal could not tell by its process. libghostty
//! ignores the sequence; this picks it out of the output beside it. As the
//! sequence travels in the output, it reaches Hof from a program on any
//! machine, through `hof agent` and back, with nothing more to set up.
//!
//! The output comes in chunks that can split a sequence anywhere, so the
//! scanner keeps where it was from one chunk to the next.

/// The OSC number. Not one any other terminal is known to use.
pub const OSC: u16 = 7701;

/// The most a sequence may hold between its number and its end: more, and
/// it is something else, never ending, and is let go.
const MOST: usize = 256;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// What a program said, as it said it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub state: String,
    /// Empty when it gave none.
    pub name: String,
}

#[derive(Debug, Default)]
enum At {
    /// Outside any sequence of ours.
    #[default]
    Ground,
    /// Just after an ESC.
    Escape,
    /// After `ESC ]`, this many bytes of `7701;` matched.
    Number(usize),
    /// In the body.
    Body,
    /// An ESC in the body: the start of ST, or the sequence is cut.
    BodyEscape,
}

/// Finds reports in output, across chunks.
#[derive(Debug, Default)]
pub(crate) struct Scanner {
    at: At,
    body: Vec<u8>,
}

/// How the bytes after `ESC ]` start, for a report.
const PREFIX: &[u8] = b"7701;";

impl Scanner {
    /// Looks through `bytes`, the output that follows what it saw last,
    /// and gives each report it finishes to `found`.
    pub fn scan(&mut self, bytes: &[u8], mut found: impl FnMut(Report)) {
        let mut i = 0;
        while i < bytes.len() {
            if matches!(self.at, At::Ground) {
                // Most output is text: skip to the next ESC at once.
                match bytes[i..].iter().position(|&b| b == ESC) {
                    Some(n) => {
                        i += n + 1;
                        self.at = At::Escape;
                    }
                    None => return,
                }
                continue;
            }
            let b = bytes[i];
            i += 1;
            self.at = match std::mem::take(&mut self.at) {
                At::Ground => unreachable!("handled above"),
                At::Escape if b == b']' => At::Number(0),
                At::Escape if b == ESC => At::Escape,
                At::Escape => At::Ground,
                At::Number(n) if b == PREFIX[n] => {
                    if n + 1 == PREFIX.len() {
                        self.body.clear();
                        At::Body
                    } else {
                        At::Number(n + 1)
                    }
                }
                At::Number(_) if b == ESC => At::Escape,
                At::Number(_) => At::Ground,
                At::Body if b == BEL => {
                    self.finish(&mut found);
                    At::Ground
                }
                At::Body if b == ESC => At::BodyEscape,
                At::Body if self.body.len() >= MOST => At::Ground,
                At::Body => {
                    self.body.push(b);
                    At::Body
                }
                At::BodyEscape if b == b'\\' => {
                    self.finish(&mut found);
                    At::Ground
                }
                // A new sequence began before this one ended.
                At::BodyEscape if b == b']' => At::Number(0),
                At::BodyEscape => At::Ground,
            };
        }
    }

    fn finish(&mut self, found: &mut impl FnMut(Report)) {
        let body = String::from_utf8_lossy(&self.body);
        let (state, name) = body.split_once(';').unwrap_or((&body, ""));
        found(Report {
            state: state.trim().to_owned(),
            name: name.trim().to_owned(),
        });
        self.body.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reports(chunks: &[&[u8]]) -> Vec<Report> {
        let mut scanner = Scanner::default();
        let mut out = Vec::new();
        for chunk in chunks {
            scanner.scan(chunk, |r| out.push(r));
        }
        out
    }

    fn report(state: &str, name: &str) -> Report {
        Report {
            state: state.into(),
            name: name.into(),
        }
    }

    #[test]
    fn a_report_amid_output_is_found() {
        let found = reports(&[b"hello \x1b[1mbold\x1b[0m \x1b]7701;waiting;claude\x07 more"]);
        assert_eq!(found, [report("waiting", "claude")]);
    }

    #[test]
    fn a_report_may_end_with_st_and_leave_out_the_name() {
        assert_eq!(reports(&[b"\x1b]7701;idle\x1b\\"]), [report("idle", "")]);
    }

    #[test]
    fn a_report_split_across_chunks_is_still_found() {
        let whole: &[u8] = b"x\x1b]7701;working;codex\x1b\\y";
        for cut in 1..whole.len() {
            let (a, b) = whole.split_at(cut);
            assert_eq!(
                reports(&[a, b]),
                [report("working", "codex")],
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn other_sequences_are_not_reports() {
        let found = reports(&[b"\x1b]0;title\x07\x1b]77010;x\x07\x1b]7;file:///\x07\x1b[7701;1m"]);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_sequence_that_never_ends_is_let_go() {
        let mut long = b"\x1b]7701;".to_vec();
        long.extend(std::iter::repeat_n(b'x', MOST + 10));
        long.extend(b"\x07\x1b]7701;idle\x07");
        assert_eq!(reports(&[&long]), [report("idle", "")]);
    }

    #[test]
    fn a_sequence_cut_off_by_another_gives_way_to_it() {
        let found = reports(&[b"\x1b]7701;work\x1b]7701;waiting\x07"]);
        assert_eq!(found, [report("waiting", "")]);
    }
}
