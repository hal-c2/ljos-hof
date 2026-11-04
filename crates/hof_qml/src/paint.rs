//! Painting the grid.
//!
//! Text is shaped once per distinct cluster and style (a glyph template),
//! and a line's glyphs are put together once per distinct line content: a
//! line that scrolls up keeps its glyphs, only its place changes. Each line
//! becomes a few glyph runs, one per font and colour, placed on the cell
//! grid rather than where the shaper's advances would put them, so a
//! fallback glyph never shifts the columns after it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use hof_vt::{Attrs, Cell, CursorShape, Grid, Line, Rgb, Session};
use ljos::item::{self, Decorations, DisplayListBuilder, GlyphRun, ItemPaint, LayoutParams};
use ljos::item::{PositionedGlyph, Rgba, TextEngine, TextStyle, kurbo, peniko};
use ljos::prelude::*;

use crate::terminal::{Terminal, shared_of};

/// One cell and where things go in it, in logical pixels, snapped so that
/// every edge falls on a device pixel.
#[derive(Clone, Debug)]
pub struct Metrics {
    /// Regular, bold, italic, bold italic: indexed by [`style_index`].
    pub styles: Option<Rc<[TextStyle; 4]>>,
    pub cell: kurbo::Size,
    /// The cell in device pixels, which programs read from the pty.
    pub device_cell: (u32, u32),
    /// From the top of the cell.
    pub baseline: f64,
    /// Tops of the underline and the strike-through, from the top of the cell.
    pub underline: f64,
    pub strike: f64,
    /// Of the lines under and through text.
    pub thickness: f64,
    /// One device pixel.
    pub pixel: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            styles: None,
            cell: kurbo::Size::new(8.0, 16.0),
            device_cell: (8, 16),
            baseline: 12.0,
            underline: 13.0,
            strike: 8.0,
            thickness: 1.0,
            pixel: 1.0,
        }
    }
}

impl Metrics {
    pub fn measure(te: &mut TextEngine, style: &TextStyle, dpr: f64) -> Self {
        let fm = te.font_metrics(style);
        let advance = te.advance_width("M", style);
        let device = |v: f64| (v * dpr).round().max(1.0);
        let (cw, ch) = (device(advance), (fm.line_height * dpr).ceil().max(1.0));
        let cell = kurbo::Size::new(cw / dpr, ch / dpr);
        // Any spare height is shared above and below the text.
        let baseline = ((ch / dpr - fm.height) / 2.0 + fm.ascent) * dpr;
        let baseline = baseline.round() / dpr;
        let thickness = device(fm.line_width) / dpr;
        let underline = ((baseline + fm.underline_position) * dpr).round() / dpr;
        let underline = underline.min(cell.height - thickness);
        let strike = ((baseline - fm.strikeout_position) * dpr).round() / dpr;
        let styled = |weight: f32, italic: bool| TextStyle {
            weight,
            italic,
            ..style.clone()
        };
        let bold = style.weight.max(700.0);
        Self {
            styles: Some(Rc::new([
                style.clone(),
                styled(bold, style.italic),
                styled(style.weight, true),
                styled(bold, true),
            ])),
            cell,
            device_cell: (cw as u32, ch as u32),
            baseline,
            underline,
            strike,
            thickness,
            pixel: 1.0 / dpr,
        }
    }

    /// Whether glyphs shaped for `self` can be reused for `other`.
    fn same(&self, other: &Self) -> bool {
        let style = match (&self.styles, &other.styles) {
            (Some(a), Some(b)) => {
                let (a, b) = (&a[0], &b[0]);
                a.family == b.family
                    && a.pixel_size == b.pixel_size
                    && a.weight == b.weight
                    && a.italic == b.italic
                    && a.letter_spacing == b.letter_spacing
            }
            (None, None) => true,
            _ => false,
        };
        style && self.cell == other.cell && self.baseline == other.baseline
    }
}

fn style_index(attrs: Attrs) -> usize {
    usize::from(attrs.contains(Attrs::BOLD)) | usize::from(attrs.contains(Attrs::ITALIC)) << 1
}

/// What the painter keeps between frames.
#[derive(Default)]
pub struct View {
    /// Made with the first update: making one can fail, and a terminal
    /// that never gets a session never needs one.
    pub grid: Option<Grid>,
    pub bold_is_bright: bool,
    pub metrics: Metrics,
    glyphs: Glyphs,
    /// The link under the pointer, underlined.
    pub link: Option<hof_vt::Hyperlink>,
}

impl fmt::Debug for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View")
            .field("metrics", &self.metrics)
            .finish_non_exhaustive()
    }
}

impl View {
    /// Forgets the old session's screen.
    pub fn reset(&mut self) {
        self.grid = None;
        self.glyphs.lines.clear();
        self.glyphs.last_frame.clear();
    }

    pub fn set_metrics(&mut self, metrics: Metrics) {
        if !self.metrics.same(&metrics) {
            self.glyphs = Glyphs::default();
        }
        self.metrics = metrics;
    }

    /// Returns whether the cursor moved or changed its look.
    fn update(&mut self, session: &Session) -> bool {
        if self.grid.is_none() {
            self.grid = Grid::new().ok();
        }
        let Some(grid) = &mut self.grid else {
            return false;
        };
        grid.bold_is_bright = self.bold_is_bright;
        grid.update(session).is_ok_and(|d| d.cursor)
    }

    /// The cursor is there to be seen, and blinks.
    fn blinks(&self) -> bool {
        self.grid
            .as_ref()
            .is_some_and(|g| g.cursor.at.is_some() && g.cursor.blinking)
    }

    /// The cursor's cell, in the item's coordinates.
    pub fn cursor_rect(&self, padding: f64) -> kurbo::Rect {
        let (x, y) = self
            .grid
            .as_ref()
            .and_then(|g| g.cursor.at)
            .unwrap_or((0, 0));
        let cell = self.metrics.cell;
        kurbo::Rect::from_origin_size(
            (
                padding + f64::from(x) * cell.width,
                padding + f64::from(y) * cell.height,
            ),
            cell,
        )
    }
}

/// Shaped text, kept while the fonts and the cell stay the same.
#[derive(Default)]
struct Glyphs {
    /// The text engine's font generation they were shaped with.
    generation: u64,
    templates: [HashMap<String, Template>; 4],
    /// Lines drawn this frame and the one before, by content: a line not
    /// seen for a whole frame is dropped, so this never outgrows two
    /// screens.
    lines: HashMap<u64, Rc<[Placed]>>,
    last_frame: HashMap<u64, Rc<[Placed]>>,
}

/// A cluster as shaped: its glyphs from the cell's origin on the baseline,
/// one piece per font the shaper used.
type Template = Rc<[Piece]>;

struct Piece {
    font: peniko::FontData,
    size: f32,
    coords: Vec<i16>,
    embolden: bool,
    skew: f32,
    glyphs: Vec<(u32, f32, f32)>,
}

impl Piece {
    fn same_font(&self, run: &GlyphRun) -> bool {
        self.font.data.id() == run.font.data.id()
            && self.font.index == run.font.index
            && self.size == run.font_size
            && self.embolden == run.embolden
            && self.skew == run.skew
            && self.coords == run.normalized_coords
    }

    fn run(&self) -> GlyphRun {
        let mut run = GlyphRun::new(
            self.font.clone(),
            self.size,
            Vec::new(),
            0.0,
            kurbo::Rect::ZERO,
        );
        run.normalized_coords.clone_from(&self.coords);
        run.embolden = self.embolden;
        run.skew = self.skew;
        run
    }
}

/// A run of a line's glyphs, ready to draw at the line's origin.
type Placed = (Arc<GlyphRun>, Rgba);

impl Glyphs {
    fn begin_frame(&mut self, te: &TextEngine) {
        if self.generation != te.generation() {
            *self = Self {
                generation: te.generation(),
                ..Self::default()
            };
        }
        self.last_frame = std::mem::take(&mut self.lines);
    }

    fn template(
        &mut self,
        te: &mut TextEngine,
        styles: &[TextStyle; 4],
        text: &str,
        i: usize,
    ) -> Template {
        if let Some(t) = self.templates[i].get(text) {
            return Rc::clone(t);
        }
        let layout = te.layout(text, &styles[i], &LayoutParams::default());
        let pieces: Vec<Piece> = layout
            .lines
            .iter()
            .flat_map(|line| line.runs.iter().map(move |r| (line.baseline, &r.glyphs)))
            .map(|(baseline, run)| Piece {
                font: run.font.clone(),
                size: run.font_size,
                coords: run.normalized_coords.clone(),
                embolden: run.embolden,
                skew: run.skew,
                glyphs: run
                    .glyphs
                    .iter()
                    .map(|g| (g.id, g.x, g.y - baseline as f32))
                    .collect(),
            })
            .collect();
        let template: Template = pieces.into();
        self.templates[i].insert(text.to_owned(), Rc::clone(&template));
        template
    }

    /// The glyph runs of `line`, from the cache when its content was drawn
    /// in the last two frames.
    fn line(&mut self, te: &mut TextEngine, m: &Metrics, line: &Line, bg: Rgb) -> Rc<[Placed]> {
        let key = content_key(line, bg);
        if let Some(found) = self.lines.get(&key) {
            return Rc::clone(found);
        }
        if let Some(found) = self.last_frame.remove(&key) {
            self.lines.insert(key, Rc::clone(&found));
            return found;
        }
        let Some(styles) = m.styles.clone() else {
            return Rc::from([]);
        };
        let baseline = m.baseline as f32;
        let mut groups: Vec<(GlyphRun, Rgba)> = Vec::new();
        for (col, cell) in line.cells.iter().enumerate() {
            let text = line.cell_text(cell);
            if !draws(cell, text) {
                continue;
            }
            let colour = ink(cell, bg);
            let x0 = (col as f64 * m.cell.width) as f32;
            let template = self.template(te, &styles, text, style_index(cell.attrs));
            for piece in template.iter() {
                let at = groups
                    .iter()
                    .position(|(run, c)| *c == colour && piece.same_font(run));
                let at = at.unwrap_or_else(|| {
                    groups.push((piece.run(), colour));
                    groups.len() - 1
                });
                groups[at]
                    .0
                    .glyphs
                    .extend(piece.glyphs.iter().map(|&(id, x, y)| PositionedGlyph {
                        id,
                        x: x0 + x,
                        y: baseline + y,
                    }));
            }
        }
        let bounds = kurbo::Rect::new(
            -m.cell.width,
            0.0,
            (line.cells.len() + 1) as f64 * m.cell.width,
            m.cell.height,
        );
        let placed: Rc<[Placed]> = groups
            .into_iter()
            .map(|(mut run, colour)| {
                run.baseline = baseline;
                run.bounds = bounds;
                (Arc::new(run), colour)
            })
            .collect();
        self.lines.insert(key, Rc::clone(&placed));
        placed
    }

    /// One cell's glyphs alone, in `colour`: what a block cursor covers.
    fn cell(
        &mut self,
        te: &mut TextEngine,
        m: &Metrics,
        cell: &Cell,
        text: &str,
        colour: Rgba,
    ) -> Vec<Placed> {
        let Some(styles) = m.styles.clone() else {
            return Vec::new();
        };
        let template = self.template(te, &styles, text, style_index(cell.attrs));
        let baseline = m.baseline as f32;
        template
            .iter()
            .map(|piece| {
                let mut run = piece.run();
                run.baseline = baseline;
                run.bounds = kurbo::Rect::from_origin_size(
                    (-m.cell.width, 0.0),
                    (m.cell.width * 4.0, m.cell.height),
                );
                run.glyphs = piece
                    .glyphs
                    .iter()
                    .map(|&(id, x, y)| PositionedGlyph {
                        id,
                        x,
                        y: baseline + y,
                    })
                    .collect();
                (Arc::new(run), colour)
            })
            .collect()
    }
}

/// What makes two lines draw the same glyphs: their text, and each cell's
/// place, style and colour. The background matters only to faint text.
fn content_key(line: &Line, bg: Rgb) -> u64 {
    let mut h = DefaultHasher::new();
    line.text.hash(&mut h);
    (bg.r, bg.g, bg.b).hash(&mut h);
    for c in &line.cells {
        let glyph_attrs = c.attrs & (Attrs::BOLD | Attrs::ITALIC | Attrs::FAINT | Attrs::INVISIBLE);
        (
            c.start,
            c.len,
            c.width,
            c.fg.r,
            c.fg.g,
            c.fg.b,
            glyph_attrs.bits(),
        )
            .hash(&mut h);
    }
    h.finish()
}

fn draws(cell: &Cell, text: &str) -> bool {
    cell.width > 0 && !text.is_empty() && text != " " && !cell.attrs.contains(Attrs::INVISIBLE)
}

fn rgba(c: Rgb) -> Rgba {
    Rgba::from_straight_u8(c.r, c.g, c.b, 255)
}

/// The colour a cell's text is drawn in: faint text halfway to the
/// background.
fn ink(cell: &Cell, bg: Rgb) -> Rgba {
    if cell.attrs.contains(Attrs::FAINT) {
        let mix = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
        Rgba::from_straight_u8(
            mix(cell.fg.r, bg.r),
            mix(cell.fg.g, bg.g),
            mix(cell.fg.b, bg.b),
            255,
        )
    } else {
        rgba(cell.fg)
    }
}

fn colour(c: Color) -> Option<Rgba> {
    c.is_valid().then(|| {
        let [r, g, b, a] = c.to_rgba8();
        Rgba::from_straight_u8(r, g, b, a)
    })
}

/// One frame of a terminal: what changes per frame, and the view that
/// keeps what doesn't.
pub struct Frame {
    view: Rc<RefCell<View>>,
    padding: f64,
    focused: bool,
    selection: Option<Rgba>,
    found: Option<Rgba>,
    current_found: Option<Rgba>,
    cursor: Option<Rgba>,
    /// A blinking cursor in its hidden half.
    blink_off: bool,
    preedit: String,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

impl ItemPaint for Frame {
    fn paint(&self, size: kurbo::Size, te: &mut TextEngine, out: &mut DisplayListBuilder) {
        // A frame painted while the view is being updated draws nothing
        // rather than panicking; the update repaints.
        let Ok(mut view) = self.view.try_borrow_mut() else {
            return;
        };
        let View {
            grid,
            metrics,
            glyphs,
            link,
            ..
        } = &mut *view;
        let Some(grid) = grid.as_ref() else {
            return;
        };
        let m = &*metrics;
        let bg = grid.colors.background;
        out.fill_rect(size.to_rect(), rgba(bg));
        glyphs.begin_frame(te);
        let fg = grid.colors.foreground;
        let selection = self
            .selection
            .unwrap_or(Rgba::from_straight_u8(fg.r, fg.g, fg.b, 0x50));
        // Yellow, as finding is drawn nearly everywhere, from the theme's
        // own so it suits it; see-through so the cells' colours still show.
        let yellow = |i: usize, alpha: u8| {
            let c = grid.colors.palette.get(i).copied().unwrap_or(fg);
            Rgba::from_straight_u8(c.r, c.g, c.b, alpha)
        };
        let found = self.found.unwrap_or_else(|| yellow(3, 0x60));
        let current_found = self.current_found.unwrap_or_else(|| yellow(11, 0xb0));
        let mut highlights = grid.found.iter().peekable();
        for (y, line) in grid.lines.iter().enumerate() {
            let top = self.padding + y as f64 * m.cell.height;
            fill_runs(out, m, self.padding, top, line, |c| c.bg.map(rgba));
            // Highlights come in reading order, so each row takes its own
            // off the front.
            while let Some(h) = highlights.next_if(|h| usize::from(h.row) <= y) {
                if usize::from(h.row) < y {
                    continue;
                }
                let x0 = self.padding + f64::from(h.start) * m.cell.width;
                let x1 = self.padding + f64::from(h.end.saturating_add(1)) * m.cell.width;
                let colour = if h.current { current_found } else { found };
                out.fill_rect(kurbo::Rect::new(x0, top, x1, top + m.cell.height), colour);
            }
            fill_runs(out, m, self.padding, top, line, |c| {
                c.attrs.contains(Attrs::SELECTED).then_some(selection)
            });
            let origin = kurbo::Vec2::new(self.padding, top);
            for (run, colour) in glyphs.line(te, m, line, bg).iter() {
                out.glyphs(Arc::clone(run), origin, *colour, Decorations::default());
            }
            decorate(out, m, self.padding, top, line);
            for span in link.iter().flat_map(|l| &l.spans) {
                if usize::from(span.row) == y {
                    let x0 = self.padding + f64::from(span.start) * m.cell.width;
                    let x1 = self.padding + f64::from(span.end) * m.cell.width;
                    let y0 = top + m.underline;
                    out.fill_rect(kurbo::Rect::new(x0, y0, x1, y0 + m.thickness), rgba(fg));
                }
            }
        }
        self.paint_cursor(grid, m, glyphs, te, out);
    }
}

impl Frame {
    fn paint_cursor(
        &self,
        grid: &Grid,
        m: &Metrics,
        glyphs: &mut Glyphs,
        te: &mut TextEngine,
        out: &mut DisplayListBuilder,
    ) {
        let Some((x, y)) = grid.cursor.at else {
            return;
        };
        let Some(line) = grid.lines.get(usize::from(y)) else {
            return;
        };
        let mut col = usize::from(x);
        if grid.cursor.wide_tail && col > 0 {
            col -= 1;
        }
        let cell = line.cells.get(col);
        let wide = cell.is_some_and(|c| c.width == 2);
        let width = m.cell.width * if wide { 2.0 } else { 1.0 };
        let origin = kurbo::Point::new(
            self.padding + col as f64 * m.cell.width,
            self.padding + f64::from(y) * m.cell.height,
        );
        let rect = kurbo::Rect::from_origin_size(origin, (width, m.cell.height));
        let colour = self
            .cursor
            .or(grid.colors.cursor.map(rgba))
            .unwrap_or(rgba(grid.colors.foreground));

        if !self.preedit.is_empty() {
            self.paint_preedit(grid, m, te, out, origin);
            return;
        }
        if self.blink_off {
            return;
        }
        let shape = if self.focused {
            grid.cursor.shape
        } else {
            CursorShape::BlockHollow
        };
        let edge = m.pixel.max(m.thickness);
        match shape {
            CursorShape::Block => {
                out.fill_rect(rect, colour);
                if let Some(cell) = cell {
                    let text = line.cell_text(cell);
                    if draws(cell, text) {
                        let under = rgba(grid.colors.background);
                        for (run, c) in glyphs.cell(te, m, cell, text, under) {
                            out.glyphs(run, origin.to_vec2(), c, Decorations::default());
                        }
                    }
                }
            }
            CursorShape::BlockHollow => {
                let r = rect;
                out.fill_rect(kurbo::Rect::new(r.x0, r.y0, r.x1, r.y0 + edge), colour);
                out.fill_rect(kurbo::Rect::new(r.x0, r.y1 - edge, r.x1, r.y1), colour);
                out.fill_rect(kurbo::Rect::new(r.x0, r.y0, r.x0 + edge, r.y1), colour);
                out.fill_rect(kurbo::Rect::new(r.x1 - edge, r.y0, r.x1, r.y1), colour);
            }
            CursorShape::Bar => {
                let w = (2.0 * m.pixel).max(edge);
                out.fill_rect(
                    kurbo::Rect::new(rect.x0, rect.y0, rect.x0 + w, rect.y1),
                    colour,
                );
            }
            CursorShape::Underline => {
                let h = (2.0 * m.pixel).max(edge);
                out.fill_rect(
                    kurbo::Rect::new(rect.x0, rect.y1 - h, rect.x1, rect.y1),
                    colour,
                );
            }
        }
    }

    /// What the input method is composing, over the cells from the cursor
    /// on, underlined as compositions are.
    fn paint_preedit(
        &self,
        grid: &Grid,
        m: &Metrics,
        te: &mut TextEngine,
        out: &mut DisplayListBuilder,
        at: kurbo::Point,
    ) {
        let Some(styles) = m.styles.as_ref() else {
            return;
        };
        let layout = te.layout(&self.preedit, &styles[0], &LayoutParams::default());
        let width = layout.advance.max(m.cell.width);
        let fg = rgba(grid.colors.foreground);
        out.fill_rect(
            kurbo::Rect::from_origin_size(at, (width, m.cell.height)),
            rgba(grid.colors.background),
        );
        for line in &layout.lines {
            let origin = kurbo::Vec2::new(at.x, at.y + m.baseline - line.baseline);
            for run in &line.runs {
                out.glyphs(Arc::clone(&run.glyphs), origin, fg, Decorations::default());
            }
        }
        let y = at.y + m.underline;
        out.fill_rect(kurbo::Rect::new(at.x, y, at.x + width, y + m.thickness), fg);
    }
}

/// Fills each run of cells `fill` gives the same colour, as one rectangle.
fn fill_runs(
    out: &mut DisplayListBuilder,
    m: &Metrics,
    left: f64,
    top: f64,
    line: &Line,
    fill: impl Fn(&Cell) -> Option<Rgba>,
) {
    let mut run: Option<(usize, Rgba)> = None;
    let flush = |run: Option<(usize, Rgba)>, end: usize, out: &mut DisplayListBuilder| {
        if let Some((start, c)) = run {
            let x0 = left + start as f64 * m.cell.width;
            let x1 = left + end as f64 * m.cell.width;
            out.fill_rect(kurbo::Rect::new(x0, top, x1, top + m.cell.height), c);
        }
    };
    for (x, cell) in line.cells.iter().enumerate() {
        let c = fill(cell);
        if run.map(|(_, r)| Some(r)) != Some(c) {
            flush(run, x, out);
            run = c.map(|c| (x, c));
        }
    }
    flush(run, line.cells.len(), out);
}

/// Underlines, strike-throughs and overlines, in the cell's underline
/// colour or its text's.
fn decorate(out: &mut DisplayListBuilder, m: &Metrics, left: f64, top: f64, line: &Line) {
    const LINES: Attrs = Attrs::UNDERLINES
        .union(Attrs::STRIKETHROUGH)
        .union(Attrs::OVERLINE);
    let t = m.thickness;
    for (col, cell) in line.cells.iter().enumerate() {
        if !cell.attrs.intersects(LINES) || cell.attrs.contains(Attrs::INVISIBLE) {
            continue;
        }
        let x0 = left + col as f64 * m.cell.width;
        let w = m.cell.width * f64::from(cell.width.max(1));
        let x1 = x0 + w;
        let ink = rgba(cell.fg);
        let under = cell.underline.map_or(ink, rgba);
        let y = top + m.underline;
        let bar = |out: &mut DisplayListBuilder, x0: f64, x1: f64, y: f64, c: Rgba| {
            out.fill_rect(kurbo::Rect::new(x0, y, x1, y + t), c);
        };
        let a = cell.attrs;
        if a.contains(Attrs::UNDERLINE) {
            bar(out, x0, x1, y, under);
        } else if a.contains(Attrs::DOUBLE_UNDERLINE) {
            let up = (y - 2.0 * t).max(top + m.baseline + m.pixel);
            bar(out, x0, x1, up, under);
            bar(out, x0, x1, up + 2.0 * t, under);
        } else if a.contains(Attrs::DOTTED_UNDERLINE) || a.contains(Attrs::DASHED_UNDERLINE) {
            let (on, step) = if a.contains(Attrs::DOTTED_UNDERLINE) {
                (t, 2.0 * t)
            } else {
                (w / 4.0, w / 2.0)
            };
            let mut x = x0;
            while x < x1 {
                bar(out, x, (x + on).min(x1), y, under);
                x += step.max(m.pixel);
            }
        } else if a.contains(Attrs::CURLY_UNDERLINE) {
            // A wave in steps: low and high in turn, a quarter cell each.
            let step = (w / 4.0).max(m.pixel);
            let mut x = x0;
            let mut high = false;
            while x < x1 {
                let dy = if high { -t } else { 0.0 };
                bar(out, x, (x + step).min(x1), y + dy, under);
                x += step;
                high = !high;
            }
        }
        if a.contains(Attrs::STRIKETHROUGH) {
            bar(out, x0, x1, top + m.strike, ink);
        }
        if a.contains(Attrs::OVERLINE) {
            bar(out, x0, x1, top, ink);
        }
    }
}

/// Paints nothing: a terminal whose state is gone.
#[derive(Debug)]
struct Blank;

impl ItemPaint for Blank {
    fn paint(&self, _: kurbo::Size, _: &mut TextEngine, _: &mut DisplayListBuilder) {}
}

/// Tracked: a terminal repaints when its focus, padding or colours change,
/// and when the session's news says the screen did ([`item::repaint`]).
pub fn read(engine: &Engine, me: ObjectId) -> Rc<dyn ItemPaint> {
    let focused = item::prop(engine, me, Item::ACTIVE_FOCUS);
    let padding = item::prop(engine, me, Terminal::PADDING).max(0.0);
    let selection = colour(item::prop(engine, me, Terminal::SELECTION_COLOR));
    let cursor = colour(item::prop(engine, me, Terminal::CURSOR_COLOR));
    let found = colour(item::prop(engine, me, Terminal::MATCH_COLOR));
    let current_found = colour(item::prop(engine, me, Terminal::CURRENT_MATCH_COLOR));
    let Some(shared) = shared_of(engine, me) else {
        return Rc::new(Blank);
    };
    let moved = shared
        .session
        .borrow()
        .as_ref()
        .is_some_and(|session| shared.view.borrow_mut().update(session));
    let preedit = shared.preedit.borrow().clone();
    // Only while it has the keys: unfocused, the cursor is a steady outline.
    let blinks = focused && preedit.is_empty() && shared.view.borrow().blinks();
    let blink_off = crate::terminal::blink(engine, me, &shared, blinks, moved);
    Rc::new(Frame {
        view: Rc::clone(&shared.view),
        padding,
        focused,
        selection,
        found,
        current_found,
        cursor,
        blink_off,
        preedit,
    })
}
