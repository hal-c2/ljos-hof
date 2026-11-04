//! Finding text on the screen and in the scrollback, and which cells in view
//! to highlight for it. libghostty does the finding; this keeps one search
//! per session and turns its matches into rows of cells.

use libghostty_vt::Terminal;
use libghostty_vt::search::{MatchBuffer, Search};
use libghostty_vt::selection::Selection;
use libghostty_vt::terminal::PointSpace;

use crate::Error;

/// How a search stands: how many matches there are, and which one is the
/// current one, counted from the newest (0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub total: usize,
    pub current: Option<usize>,
}

/// One row's share of a match in view: columns `start..=end` of `row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub row: u16,
    pub start: u16,
    pub end: u16,
    /// Part of the current match, which is drawn apart from the others.
    pub current: bool,
}

pub(crate) struct Finder {
    search: Search<'static, 'static>,
    /// Kept between reads so a frame with matches allocates nothing.
    storage: MatchBuffer,
}

impl Finder {
    pub fn new(term: &mut Terminal<'static, 'static>) -> Result<Self, Error> {
        Ok(Self {
            search: Search::new(term)?,
            storage: MatchBuffer::new(),
        })
    }

    /// Look for `needle`, and make the newest match current. The same needle
    /// again keeps the current match, so a find bar can resubmit as it likes.
    pub fn set(
        &mut self,
        term: &mut Terminal<'static, 'static>,
        needle: &str,
    ) -> Result<Found, Error> {
        self.search.set_needle(term, needle.as_bytes())?;
        // All of it at once: a find bar wants the count as the user types,
        // and even a long scrollback is searched in a few milliseconds.
        self.search.run(term)?;
        if self.search.selected_index()?.is_none() {
            self.search.select_next(term)?;
        }
        self.found()
    }

    /// Make the next older match current, wrapping to the newest, and
    /// scroll it into view.
    pub fn older(&mut self, term: &mut Terminal<'static, 'static>) -> Result<Found, Error> {
        self.search.select_next(term)?;
        self.found()
    }

    /// Make the next newer match current, wrapping to the oldest.
    pub fn newer(&mut self, term: &mut Terminal<'static, 'static>) -> Result<Found, Error> {
        self.search.select_prev(term)?;
        self.found()
    }

    /// How the search stood as of its last look at the terminal.
    pub fn found(&self) -> Result<Found, Error> {
        Ok(Found {
            total: self.search.total_matches()?,
            current: self.search.selected_index()?,
        })
    }

    /// Catch up with the terminal and say how the search stands.
    pub fn catch_up(&mut self, term: &mut Terminal<'static, 'static>) -> Result<Found, Error> {
        self.search.feed(term)?;
        self.found()
    }

    /// Catch up with the terminal and write what to highlight in a view of
    /// `cols` by `rows` into `out`.
    pub fn highlights(
        &mut self,
        term: &mut Terminal<'static, 'static>,
        cols: u16,
        rows: u16,
        out: &mut Vec<Highlight>,
    ) -> Result<Found, Error> {
        out.clear();
        {
            let Self { search, storage } = self;
            let snapshot = search.snapshot(term)?;
            let term = snapshot.terminal();
            let current = snapshot
                .selected_match()?
                .and_then(|m| in_view(term, &m, cols, rows));
            for m in snapshot.viewport_matches(storage)? {
                let Some(span) = in_view(term, &m, cols, rows) else {
                    continue;
                };
                let ((x0, y0), (x1, y1)) = span;
                let current = current == Some(span);
                for row in y0..=y1 {
                    out.push(Highlight {
                        row,
                        start: if row == y0 { x0 } else { 0 },
                        end: if row == y1 {
                            x1
                        } else {
                            cols.saturating_sub(1)
                        },
                        current,
                    });
                }
            }
        }
        // libghostty lists them newest first; drawing wants reading order.
        out.sort_unstable_by_key(|h| (h.row, h.start));
        self.found()
    }
}

/// Where a match starts and ends in a view of `cols` by `rows`, cut to the
/// view; `None` when none of it shows.
fn in_view(
    term: &Terminal<'_, '_>,
    m: &Selection<'_>,
    cols: u16,
    rows: u16,
) -> Option<((u16, u16), (u16, u16))> {
    let at = |g| {
        term.point_from_grid_ref(&g, PointSpace::Viewport)
            .ok()
            .flatten()
            .and_then(|p| Some((p.x, u16::try_from(p.y).ok()?)))
            .filter(|&(_, y)| y < rows)
    };
    // A match is in reading order, so an end in view with its start not
    // began above the view, and a start in view with its end not runs on
    // below it.
    match (at(m.start()), at(m.end())) {
        (Some(a), Some(b)) => Some((a, b)),
        (None, Some(b)) => Some(((0, 0), b)),
        (Some(a), None) => Some((a, (cols.saturating_sub(1), rows - 1))),
        (None, None) => None,
    }
}
