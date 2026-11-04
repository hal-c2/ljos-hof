//! The screen as the UI draws it: lines of cells with their colours resolved.
//!
//! A [`Grid`] belongs to the UI thread. [`Grid::update`] asks libghostty for
//! the rows that changed, holding the session's lock only for libghostty's own
//! copy, and rebuilds just those lines. Each line carries a generation that
//! moves whenever its contents change, so a renderer can keep its shaped text
//! per line and redo only what moved.

use libghostty_vt::RenderState;
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RowId, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::{PaletteIndex, RgbColor, Style, StyleColor, Underline};

use crate::Error;
use crate::find::Highlight;
use crate::session::{Scrollbar, Session};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl From<RgbColor> for Rgb {
    fn from(c: RgbColor) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

impl From<Rgb> for RgbColor {
    fn from(c: Rgb) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Attrs: u16 {
        const BOLD = 1 << 0;
        const ITALIC = 1 << 1;
        const FAINT = 1 << 2;
        const BLINK = 1 << 3;
        /// Drawn as blank (SGR 8); the text is still there to copy.
        const INVISIBLE = 1 << 4;
        const STRIKETHROUGH = 1 << 5;
        const OVERLINE = 1 << 6;
        const SELECTED = 1 << 7;
        const UNDERLINE = 1 << 8;
        const DOUBLE_UNDERLINE = 1 << 9;
        const CURLY_UNDERLINE = 1 << 10;
        const DOTTED_UNDERLINE = 1 << 11;
        const DASHED_UNDERLINE = 1 << 12;
        /// Any kind of underline.
        const UNDERLINES = Self::UNDERLINE.bits() | Self::DOUBLE_UNDERLINE.bits()
            | Self::CURLY_UNDERLINE.bits() | Self::DOTTED_UNDERLINE.bits()
            | Self::DASHED_UNDERLINE.bits();
    }
}

/// One cell. Its text is `line.text[start..start + len]`, empty for a blank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub start: u32,
    pub len: u16,
    /// Columns the cell's text covers: 1, 2 for a wide character, or 0 for
    /// the spacer after one, which draws nothing of its own.
    pub width: u8,
    pub fg: Rgb,
    /// `None` for the terminal's background, so the renderer can skip it.
    pub bg: Option<Rgb>,
    /// `None` for the foreground colour.
    pub underline: Option<Rgb>,
    pub attrs: Attrs,
}

#[derive(Clone, Debug)]
pub struct Line {
    /// The row's identity in libghostty, stable while it scrolls.
    pub id: Option<RowId>,
    /// Moves whenever the line's text, cells or colours change. libghostty
    /// also marks rows dirty for things drawn on top of them, such as the
    /// cursor leaving; those keep their generation.
    pub generation: u64,
    pub text: String,
    pub cells: Vec<Cell>,
    /// The line continues on the next (a soft wrap).
    pub wrapped: bool,
}

impl Line {
    fn empty() -> Self {
        Self {
            id: None,
            generation: 0,
            text: String::new(),
            cells: Vec::new(),
            wrapped: false,
        }
    }

    pub fn cell_text(&self, cell: &Cell) -> &str {
        let start = cell.start as usize;
        &self.text[start..start + cell.len as usize]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    pub foreground: Rgb,
    pub background: Rgb,
    /// `None` when neither the theme nor the program set one; draw the cursor
    /// in the text colour then.
    pub cursor: Option<Rgb>,
    pub palette: Box<[Rgb; 256]>,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            foreground: Rgb::new(0xdd, 0xdd, 0xdd),
            background: Rgb::new(0x10, 0x10, 0x10),
            cursor: None,
            palette: Box::new([Rgb::default(); 256]),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    BlockHollow,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// `None` when the cursor is hidden or out of view.
    pub at: Option<(u16, u16)>,
    /// The cursor sits on the right half of a wide character.
    pub wide_tail: bool,
    pub blinking: bool,
    pub shape: CursorShape,
    /// The program is reading a password (echo is off).
    pub password: bool,
}

/// What one update changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    /// Everything should be drawn again (size, colours or a full redraw).
    pub full: bool,
    /// How many lines changed.
    pub lines: usize,
    /// The cursor moved or changed its look.
    pub cursor: bool,
    /// The search's highlights changed.
    pub found: bool,
}

impl Damage {
    pub fn is_empty(&self) -> bool {
        !self.full && self.lines == 0 && !self.cursor && !self.found
    }
}

pub struct Grid {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    plain: Style,
    scratch: String,
    /// Where a dirty line is rebuilt, so it can be compared with what is
    /// already drawn before replacing it.
    spare: Line,
    generation: u64,
    /// Draw bold text in the bright colours, as many themes expect.
    pub bold_is_bright: bool,
    pub cols: u16,
    pub lines: Vec<Line>,
    pub colors: Colors,
    pub cursor: Cursor,
    pub scrollbar: Scrollbar,
    /// What the session's search has in view, row by row in reading order.
    pub found: Vec<Highlight>,
    /// The highlights before this update, to tell whether they moved.
    found_before: Vec<Highlight>,
}

impl Grid {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            state: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            plain: Style::default(),
            scratch: String::new(),
            spare: Line::empty(),
            generation: 0,
            bold_is_bright: false,
            cols: 0,
            lines: Vec::new(),
            colors: Colors::default(),
            cursor: Cursor::default(),
            scrollbar: Scrollbar::default(),
            found: Vec::new(),
            found_before: Vec::new(),
        })
    }

    pub fn rows(&self) -> u16 {
        u16::try_from(self.lines.len()).unwrap_or(u16::MAX)
    }

    /// Bring the grid up to date with the session.
    pub fn update(&mut self, session: &Session) -> Result<Damage, Error> {
        let update = {
            let mut guard = session.shared_state().lock();
            let vt = &mut *guard;
            std::mem::swap(&mut self.found, &mut self.found_before);
            self.found.clear();
            // Only while searching, so an idle terminal reads nothing more.
            if let Some(finder) = &mut vt.finder {
                let _ =
                    finder.highlights(&mut vt.term, vt.size.cols, vt.size.rows, &mut self.found);
            }
            let scrollbar = vt.term.scrollbar()?;
            self.scrollbar = Scrollbar {
                total: scrollbar.total,
                offset: scrollbar.offset,
                visible: scrollbar.len,
            };
            self.state.begin_update(&vt.term)?
        };
        // The rest reads only the render state's own copy; the program's
        // output is already being parsed again.
        let snapshot = update.end()?;

        let dirty = snapshot.dirty()?;
        let (cols, rows) = (snapshot.cols()?, snapshot.rows()?);
        let resized = cols != self.cols || usize::from(rows) != self.lines.len();
        let found = self.found != self.found_before;
        // Read whatever the screen says: a program changing only the
        // cursor's look (DECSCUSR) leaves the screen clean.
        let cursor = snapshot.cursor()?;
        let cursor = Cursor {
            at: cursor
                .viewport
                .filter(|_| cursor.visible)
                .map(|v| (v.x, v.y)),
            wide_tail: cursor.viewport.is_some_and(|v| v.at_wide_tail),
            blinking: cursor.blinking,
            shape: match cursor.visual_style {
                CursorVisualStyle::Bar => CursorShape::Bar,
                CursorVisualStyle::Underline => CursorShape::Underline,
                CursorVisualStyle::BlockHollow => CursorShape::BlockHollow,
                _ => CursorShape::Block,
            },
            password: cursor.password_input,
        };
        let cursor_changed = cursor != self.cursor;
        self.cursor = cursor;
        if dirty == Dirty::Clean && !resized {
            return Ok(Damage {
                found,
                cursor: cursor_changed,
                ..Damage::default()
            });
        }

        let colors = snapshot.colors()?;
        let colors = Colors {
            foreground: colors.foreground.into(),
            background: colors.background.into(),
            cursor: colors.cursor.map(Into::into),
            palette: Box::new(colors.palette.map(Into::into)),
        };
        let recoloured = colors != self.colors;
        self.colors = colors;

        let full = dirty == Dirty::Full || resized || recoloured;
        self.cols = cols;
        self.lines.resize_with(usize::from(rows), Line::empty);

        let mut rebuilt = 0;
        let mut iteration = self.rows.update(&snapshot)?;
        let mut y = 0usize;
        while let Some(row) = iteration.next() {
            let index = y;
            y += 1;
            let Some(line) = self.lines.get_mut(index) else {
                break;
            };
            let id = row.id().ok();
            if !full && !row.dirty()? && line.id == id {
                continue;
            }
            let raw = row.raw_row()?;
            line.id = id;
            let clusters = raw.has_grapheme_cluster().unwrap_or(true);
            let selected = row.selection()?.map(|s| (s.start_x, s.end_x));
            let mut cells = self.cells.update(row)?;
            build_line(
                &mut self.spare,
                &mut cells,
                LineContext {
                    colors: &self.colors,
                    plain: self.plain,
                    clusters,
                    selected,
                    bold_is_bright: self.bold_is_bright,
                },
                &mut self.scratch,
            )?;
            self.spare.wrapped = raw.is_wrapped().unwrap_or(false);
            let same = self.spare.wrapped == line.wrapped
                && self.spare.text == line.text
                && self.spare.cells == line.cells
                && line.generation != 0;
            if !same {
                std::mem::swap(&mut line.text, &mut self.spare.text);
                std::mem::swap(&mut line.cells, &mut self.spare.cells);
                line.wrapped = self.spare.wrapped;
                self.generation += 1;
                line.generation = self.generation;
                rebuilt += 1;
            }
        }
        snapshot.clean()?;
        Ok(Damage {
            full,
            lines: rebuilt,
            cursor: cursor_changed,
            found,
        })
    }
}

struct LineContext<'a> {
    colors: &'a Colors,
    plain: Style,
    /// The row has multi-codepoint clusters, so the one-codepoint shortcut
    /// won't do.
    clusters: bool,
    /// Selected columns, inclusive.
    selected: Option<(u16, u16)>,
    bold_is_bright: bool,
}

fn build_line(
    line: &mut Line,
    cells: &mut libghostty_vt::render::CellIteration<'_, '_>,
    cx: LineContext<'_>,
    scratch: &mut String,
) -> Result<(), Error> {
    line.text.clear();
    line.cells.clear();
    let mut x: u16 = 0;
    while let Some(cell) = cells.next() {
        let raw = cell.raw_cell()?;
        let width = match raw.wide()? {
            CellWide::Wide => 2,
            CellWide::SpacerTail | CellWide::SpacerHead => 0,
            _ => 1,
        };

        let start = line.text.len();
        if raw.has_text()? {
            if cx.clusters {
                cell.graphemes_utf8(scratch)?;
                line.text.push_str(scratch);
            } else if let Some(c) = char::from_u32(raw.codepoint()?) {
                line.text.push(c);
            }
        }

        let style = if raw.has_styling()? {
            cell.style()?
        } else {
            cx.plain
        };
        let mut fg = cell.fg_color()?.map_or(cx.colors.foreground, Rgb::from);
        if cx.bold_is_bright
            && style.bold
            && let StyleColor::Palette(PaletteIndex(i)) = style.fg_color
            && i < 8
        {
            fg = cx.colors.palette[usize::from(i) + 8];
        }
        let mut bg = cell.bg_color()?.map(Rgb::from);
        if style.inverse {
            let behind = bg.unwrap_or(cx.colors.background);
            bg = Some(fg);
            fg = behind;
        }
        let underline = match style.underline_color {
            StyleColor::None => None,
            StyleColor::Palette(PaletteIndex(i)) => Some(cx.colors.palette[usize::from(i)]),
            StyleColor::Rgb(c) => Some(c.into()),
        };

        let mut attrs = Attrs::empty();
        attrs.set(Attrs::BOLD, style.bold);
        attrs.set(Attrs::ITALIC, style.italic);
        attrs.set(Attrs::FAINT, style.faint);
        attrs.set(Attrs::BLINK, style.blink);
        attrs.set(Attrs::INVISIBLE, style.invisible);
        attrs.set(Attrs::STRIKETHROUGH, style.strikethrough);
        attrs.set(Attrs::OVERLINE, style.overline);
        attrs |= match style.underline {
            Underline::None => Attrs::empty(),
            Underline::Single => Attrs::UNDERLINE,
            Underline::Double => Attrs::DOUBLE_UNDERLINE,
            Underline::Curly => Attrs::CURLY_UNDERLINE,
            Underline::Dotted => Attrs::DOTTED_UNDERLINE,
            Underline::Dashed => Attrs::DASHED_UNDERLINE,
            _ => Attrs::UNDERLINE,
        };
        if cx.selected.is_some_and(|(a, b)| (a..=b).contains(&x)) {
            attrs |= Attrs::SELECTED;
        }

        line.cells.push(Cell {
            start: start as u32,
            len: (line.text.len() - start) as u16,
            width,
            fg,
            bg,
            underline,
            attrs,
        });
        x = x.saturating_add(1);
    }
    Ok(())
}
