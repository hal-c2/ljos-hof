//! Terminal sessions for Ljós Hof.
//!
//! A [`Session`] is one terminal: a program on a pseudo-terminal, and
//! libghostty-vt's model of the screen it draws. The program's output is read
//! and parsed on a thread of the session's own, so a program that floods the
//! terminal never stalls the UI. The UI hears about it through a waker that
//! fires at most once until the UI drains the [`Changes`]; however fast the
//! program writes, the UI draws at its own pace and always draws the latest
//! screen.
//!
//! The UI reads the screen through a [`Grid`], which it owns. Updating it holds
//! the session's lock only for libghostty's copy of the dirty rows; turning
//! those into lines of cells happens after the lock is released.
//!
//! A session can also be [detached](Session::detached): no program, the output
//! fed in by hand and the replies collected. Tests and session restore use
//! that. A [remote](Session::remote) one has its program on another
//! machine: the connection writes the output in through a [`Feed`], and
//! takes what the user types from an [`Outlet`].

mod clipboard;
mod error;
mod find;
mod grid;
mod input;
mod links;
mod process;
mod pty;
mod remote;
mod report;
mod session;
mod signals;
mod vt;

pub use clipboard::{Clipboard, answer_of, carrying, read_request};
pub use error::Error;
pub use find::{Found, Highlight};
pub use grid::{Attrs, Cell, Colors, Cursor, CursorShape, Damage, Grid, Line, Rgb};
pub use input::{KeyInput, MouseAction, MouseButton, MouseInput};
pub use links::{Hyperlink, LinkSpan, find_urls};
pub use process::Process;
pub use pty::Spawn;
pub use remote::{Feed, Link, Outlet};
pub use report::{OPEN_OSC, OSC as AGENT_OSC, Report, open_request};
pub use session::{Options, PasteError, Scroll, Scrollbar, Selecting, Session, Tap, Theme};
pub use signals::{Changes, Event, Progress, PromptMark};

/// libghostty-vt itself, for the key names and modifiers in [`KeyInput`].
pub use libghostty_vt;
pub use libghostty_vt::key::{Key, Mods};

/// The size of the grid, and of one cell in device pixels.
///
/// Programs read the pixel size from the pty for image protocols and size
/// reports; the grid only needs the cell counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u32,
    pub cell_height: u32,
}

impl Size {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            cell_width: 8,
            cell_height: 16,
        }
    }

    pub const fn with_cell(self, cell_width: u32, cell_height: u32) -> Self {
        Self {
            cell_width,
            cell_height,
            ..self
        }
    }

    /// A terminal is never smaller than one cell.
    pub(crate) fn clamped(self) -> Self {
        Self {
            cols: self.cols.max(1),
            rows: self.rows.max(1),
            cell_width: self.cell_width.max(1),
            cell_height: self.cell_height.max(1),
        }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self::new(80, 24)
    }
}
