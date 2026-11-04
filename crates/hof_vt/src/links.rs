//! Links on the screen: a program's own (OSC 8), or a URL in the text.
//!
//! A program's link wins where it has one, since it says what it means.
//! Elsewhere Hof looks for a URL in the line under the pointer, a soft
//! wrapped line read as one, so a long URL that wraps opens whole.

use std::ops::Range;

/// The schemes Hof takes for a URL in plain text. Others need the program
/// to mark them as links.
const SCHEMES: &[&str] = &[
    "https://", "http://", "file://", "ftp://", "sftp://", "ssh://", "git://", "mailto:",
];

/// The most of one line Hof reads for a URL.
pub(crate) const MOST_LINE: usize = 4096;

/// A link on the screen, and the cells it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub url: String,
    /// The cells, one run a row, from the top.
    pub spans: Vec<LinkSpan>,
}

/// One row's run of a link's cells, in the view; `end` is past the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub row: u16,
    pub start: u16,
    pub end: u16,
}

/// The URLs in `text`, as ranges of its chars.
pub fn find_urls(text: &str) -> Vec<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let Some(scheme) = scheme_at(&chars, at) else {
            at += 1;
            continue;
        };
        let mut end = at + scheme;
        while end < chars.len() && in_url(chars[end]) {
            end += 1;
        }
        let end = trim_end(&chars[at..end]) + at;
        if end > at + scheme {
            found.push(at..end);
            at = end;
        } else {
            at += scheme;
        }
    }
    found
}

/// How long the scheme starting at `at` is, if one does and it starts a
/// word: `xhttps://` is not a URL.
fn scheme_at(chars: &[char], at: usize) -> Option<usize> {
    if at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_') {
        return None;
    }
    SCHEMES.iter().find_map(|s| {
        let n = s.chars().count();
        let here = chars.get(at..at + n)?;
        here.iter()
            .zip(s.chars())
            .all(|(a, b)| a.to_ascii_lowercase() == b)
            .then_some(n)
    })
}

/// What a URL may hold, short of trimming its end.
fn in_url(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !matches!(c, '<' | '>' | '"' | '`' | '\0')
}

/// Where a URL found up to the end of `url` really ends. Punctuation that
/// ends a sentence is the sentence's, and a closing bracket is the URL's
/// only when it opened one, as in a Wikipedia link.
fn trim_end(url: &[char]) -> usize {
    let mut end = url.len();
    while end > 0 {
        let c = url[end - 1];
        let opening = match c {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        };
        let trim = match opening {
            Some(open) => {
                let body = &url[..end];
                let opened = body.iter().filter(|&&x| x == open).count();
                let closed = body.iter().filter(|&&x| x == c).count();
                closed > opened
            }
            None => matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*'),
        };
        if !trim {
            break;
        }
        end -= 1;
    }
    end
}

/// A line of the screen as text, with the cell each char came from.
#[derive(Debug, Default)]
pub(crate) struct LineText {
    pub text: String,
    /// For each char of `text`: its row and column.
    pub cells: Vec<(u16, u16)>,
}

impl LineText {
    pub fn push(&mut self, c: char, row: u16, col: u16) {
        self.text.push(c);
        self.cells.push((row, col));
    }

    /// The URL in the line that covers the cell at `row`, `col`.
    pub fn url_at(&self, row: u16, col: u16) -> Option<Hyperlink> {
        // The char at the cell, or the wide one the cell is the right half of.
        let at = self
            .cells
            .iter()
            .rposition(|&cell| cell <= (row, col))
            .filter(|&i| self.cells[i].0 == row)?;
        let range = find_urls(&self.text)
            .into_iter()
            .find(|r| r.contains(&at))?;
        let url: String = self
            .text
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect();
        Some(Hyperlink {
            url,
            spans: spans_of(&self.cells[range]),
        })
    }
}

/// Runs of cells, a row at a time, for cells given in order. A wide char
/// covers the cell after its own.
pub(crate) fn spans_of(cells: &[(u16, u16)]) -> Vec<LinkSpan> {
    let mut spans: Vec<LinkSpan> = Vec::new();
    for &(row, col) in cells {
        match spans.last_mut() {
            Some(s) if s.row == row && col >= s.start => s.end = s.end.max(col + 1),
            _ => spans.push(LinkSpan {
                row,
                start: col,
                end: col + 1,
            }),
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(text: &str) -> Vec<String> {
        find_urls(text)
            .into_iter()
            .map(|r| text.chars().skip(r.start).take(r.len()).collect())
            .collect()
    }

    #[test]
    fn a_url_in_a_sentence_ends_before_its_full_stop() {
        assert_eq!(
            urls("See https://example.com/a?b=c#d. Then http://x.org, and"),
            ["https://example.com/a?b=c#d", "http://x.org"]
        );
    }

    #[test]
    fn brackets_are_the_urls_only_when_it_opened_them() {
        assert_eq!(
            urls("(see https://en.wikipedia.org/wiki/Rust_(language))"),
            ["https://en.wikipedia.org/wiki/Rust_(language)"]
        );
        assert_eq!(urls("[https://x.org/a]"), ["https://x.org/a"]);
        assert_eq!(urls("<https://x.org/a>"), ["https://x.org/a"]);
        assert_eq!(urls("\"https://x.org/a\""), ["https://x.org/a"]);
    }

    #[test]
    fn a_scheme_must_start_a_word_and_have_something_after_it() {
        assert_eq!(
            urls("xhttps://no https:// HTTPS://Yes.org"),
            ["HTTPS://Yes.org"]
        );
        assert_eq!(
            urls("mailto:me@example.com file:///etc/hosts"),
            ["mailto:me@example.com", "file:///etc/hosts"]
        );
        assert!(urls("no links here: just text.").is_empty());
    }

    #[test]
    fn a_url_that_wraps_is_found_whole_and_spans_both_rows() {
        let mut line = LineText::default();
        for (i, c) in "go https://example.com/long".chars().enumerate() {
            let (row, col) = if i < 10 {
                (3, i as u16)
            } else {
                (4, i as u16 - 10)
            };
            line.push(c, row, col);
        }
        let link = line.url_at(4, 2).unwrap();
        assert_eq!(link.url, "https://example.com/long");
        assert_eq!(
            link.spans,
            [
                LinkSpan {
                    row: 3,
                    start: 3,
                    end: 10
                },
                LinkSpan {
                    row: 4,
                    start: 0,
                    end: 17
                }
            ]
        );
        assert_eq!(line.url_at(3, 0), None, "the word before is no link");
    }

    #[test]
    fn a_wide_char_in_a_url_covers_two_cells() {
        let mut line = LineText::default();
        for (c, col) in [
            ('h', 0),
            ('t', 1),
            ('t', 2),
            ('p', 3),
            (':', 4),
            ('/', 5),
            ('/', 6),
            ('日', 7),
            ('x', 9),
        ] {
            line.push(c, 0, col);
        }
        let link = line.url_at(0, 8).unwrap();
        assert_eq!(link.url, "http://日x");
        assert_eq!(
            link.spans,
            [LinkSpan {
                row: 0,
                start: 0,
                end: 10
            }]
        );
    }
}
