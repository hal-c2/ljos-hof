//! Tabs of panes, each tab a tree of splits: a pane is split in two, side
//! by side or stacked, and either half split again.
//!
//! The tree only says where panes go, as fractions of the tab. The panes'
//! terminals live elsewhere and are never remade when the tree changes, so
//! splitting, closing or zooming a pane keeps every screen and its program.

use std::fmt;

/// A pane, for as long as the layout lives. Ids are never reused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PaneId(pub u32);

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// How a split lays out its halves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// Side by side: the first half on the left.
    Horizontal,
    /// Stacked: the first half on top.
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Up | Self::Down => Axis::Vertical,
        }
    }

    /// Toward the second half of a split.
    fn forward(self) -> bool {
        matches!(self, Self::Right | Self::Down)
    }
}

/// A part of a tab, in fractions of its width and height.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const FULL: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };

    fn halves(self, axis: Axis, ratio: f64) -> (Rect, Rect) {
        match axis {
            Axis::Horizontal => {
                let w = self.w * ratio;
                (
                    Rect { w, ..self },
                    Rect {
                        x: self.x + w,
                        w: self.w - w,
                        ..self
                    },
                )
            }
            Axis::Vertical => {
                let h = self.h * ratio;
                (
                    Rect { h, ..self },
                    Rect {
                        y: self.y + h,
                        h: self.h - h,
                        ..self
                    },
                )
            }
        }
    }
}

/// A tab's tree.
#[derive(Clone, PartialEq, Debug)]
pub enum Node {
    Pane(PaneId),
    Split {
        axis: Axis,
        /// The first half's share, kept between [`MIN_RATIO`] and its
        /// complement so neither half shrinks to nothing.
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// The smallest share a half of a split keeps.
pub const MIN_RATIO: f64 = 0.05;

impl Node {
    fn contains(&self, pane: PaneId) -> bool {
        match self {
            Node::Pane(p) => *p == pane,
            Node::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    fn panes(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Pane(p) => out.push(*p),
            Node::Split { first, second, .. } => {
                first.panes(out);
                second.panes(out);
            }
        }
    }

    fn place(&self, rect: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Node::Pane(p) => out.push((*p, rect)),
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, b) = rect.halves(*axis, *ratio);
                first.place(a, out);
                second.place(b, out);
            }
        }
    }

    fn dividers(&self, rect: Rect, path: &mut String, out: &mut Vec<Divider>) {
        if let Node::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        {
            out.push(Divider {
                path: path.clone(),
                axis: *axis,
                ratio: *ratio,
                within: rect,
            });
            let (a, b) = rect.halves(*axis, *ratio);
            path.push('0');
            first.dividers(a, path, out);
            path.pop();
            path.push('1');
            second.dividers(b, path, out);
            path.pop();
        }
    }

    /// Replaces the leaf `pane` with what `f` makes of it.
    fn replace(&mut self, pane: PaneId, f: &mut dyn FnMut(PaneId) -> Node) -> bool {
        match self {
            Node::Pane(p) if *p == pane => {
                *self = f(pane);
                true
            }
            Node::Pane(_) => false,
            Node::Split { first, second, .. } => first.replace(pane, f) || second.replace(pane, f),
        }
    }

    /// Takes `pane` out, its sibling taking its parent's place. `false` when
    /// the node is that pane alone: the caller drops it.
    fn remove(&mut self, pane: PaneId) -> bool {
        let Node::Split { first, second, .. } = self else {
            return false;
        };
        let keep = if **first == Node::Pane(pane) {
            Some(std::mem::replace(&mut **second, Node::Pane(pane)))
        } else if **second == Node::Pane(pane) {
            Some(std::mem::replace(&mut **first, Node::Pane(pane)))
        } else {
            None
        };
        match keep {
            Some(sibling) => {
                *self = sibling;
                true
            }
            None => first.remove(pane) || second.remove(pane),
        }
    }

    /// The split at `path` ("" the root, then '0' first and '1' second).
    fn at_path(&mut self, path: &str) -> Option<&mut Node> {
        let mut node = self;
        for step in path.chars() {
            let Node::Split { first, second, .. } = node else {
                return None;
            };
            node = match step {
                '0' => first,
                '1' => second,
                _ => return None,
            };
        }
        Some(node)
    }

    /// Grows the side of `pane` that faces `dir` by `by`: the split nearest
    /// the pane whose border is on that side moves.
    fn grow(&mut self, pane: PaneId, dir: Direction, by: f64) -> bool {
        let Node::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        else {
            return false;
        };
        let in_first = first.contains(pane);
        let inner = if in_first { first } else { second };
        if inner.grow(pane, dir, by) {
            return true;
        }
        // The border between the halves faces the first half's pane forward
        // and the second's back.
        if *axis == dir.axis() && in_first == dir.forward() {
            let moved = if in_first { *ratio + by } else { *ratio - by };
            *ratio = moved.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
            return true;
        }
        false
    }
}

/// The border of a split, for the UI to draw and drag.
#[derive(Clone, PartialEq, Debug)]
pub struct Divider {
    /// Which split, for [`Layout::set_ratio`].
    pub path: String,
    pub axis: Axis,
    pub ratio: f64,
    /// All of the split, both halves.
    pub within: Rect,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub root: Node,
    pub focused: PaneId,
    /// The focused pane fills the tab and the others are hidden.
    pub zoomed: bool,
    /// The panes focused before, the latest last: where focus goes when the
    /// focused pane closes.
    history: Vec<PaneId>,
}

impl Tab {
    fn new(pane: PaneId) -> Self {
        Self {
            root: Node::Pane(pane),
            focused: pane,
            zoomed: false,
            history: Vec::new(),
        }
    }

    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.root.panes(&mut out);
        out
    }

    fn placed(&self) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.root.place(Rect::FULL, &mut out);
        out
    }

    fn focus(&mut self, pane: PaneId) {
        if pane != self.focused {
            self.history.retain(|p| *p != pane);
            self.history.push(self.focused);
            self.focused = pane;
        }
    }
}

/// Where a pane is shown.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placed {
    pub pane: PaneId,
    /// Its tab's index.
    pub tab: usize,
    pub rect: Rect,
    /// The focused pane of its tab.
    pub focused: bool,
    /// On screen: in the current tab, and not hidden by a zoomed pane.
    pub visible: bool,
}

/// What closing a pane closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Closed {
    /// The pane; its tab has others.
    Pane,
    /// Its tab, whose last pane it was.
    Tab,
    /// The last pane of all.
    Everything,
}

/// All of a window's tabs.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    tabs: Vec<Tab>,
    current: usize,
    next: u32,
}

impl Layout {
    pub fn new() -> Self {
        Self::default()
    }

    fn fresh(&mut self) -> PaneId {
        self.next += 1;
        PaneId(self.next)
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn current_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.current)
    }

    /// The focused pane of the current tab.
    pub fn focused(&self) -> Option<PaneId> {
        self.current_tab().map(|t| t.focused)
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.root.contains(pane))
    }

    /// Opens a tab with one new pane after the current tab, and shows it.
    pub fn new_tab(&mut self) -> PaneId {
        let pane = self.fresh();
        let at = if self.tabs.is_empty() {
            0
        } else {
            self.current + 1
        };
        self.tabs.insert(at, Tab::new(pane));
        self.current = at;
        pane
    }

    /// Splits the focused pane in two along `axis`; the new pane is the
    /// second half and takes the focus. `None` with no tab to split.
    pub fn split(&mut self, axis: Axis) -> Option<PaneId> {
        let pane = self.fresh();
        let tab = self.tabs.get_mut(self.current)?;
        let at = tab.focused;
        tab.root.replace(at, &mut |old| Node::Split {
            axis,
            ratio: 0.5,
            first: Box::new(Node::Pane(old)),
            second: Box::new(Node::Pane(pane)),
        });
        tab.zoomed = false;
        tab.focus(pane);
        Some(pane)
    }

    /// Closes `pane`. Focus goes back to the pane focused before it.
    pub fn close(&mut self, pane: PaneId) -> Option<Closed> {
        let index = self.tab_of(pane)?;
        let tab = &mut self.tabs[index];
        if tab.root == Node::Pane(pane) {
            self.tabs.remove(index);
            if self.tabs.is_empty() {
                self.current = 0;
                return Some(Closed::Everything);
            }
            if index < self.current || self.current >= self.tabs.len() {
                self.current = self.current.saturating_sub(1);
            }
            return Some(Closed::Tab);
        }
        tab.root.remove(pane);
        tab.history.retain(|p| *p != pane);
        if tab.focused == pane {
            let alive = tab.panes();
            tab.focused = tab
                .history
                .pop()
                .filter(|p| alive.contains(p))
                .unwrap_or(alive[0]);
            tab.zoomed = false;
        }
        Some(Closed::Pane)
    }

    /// Focuses `pane`, showing its tab.
    pub fn focus_pane(&mut self, pane: PaneId) -> bool {
        let Some(index) = self.tab_of(pane) else {
            return false;
        };
        self.current = index;
        self.tabs[index].focus(pane);
        true
    }

    /// Focuses the pane next to the focused one toward `dir`: of those
    /// touching that side, the one sharing most of it.
    pub fn focus_toward(&mut self, dir: Direction) -> bool {
        let Some(tab) = self.tabs.get_mut(self.current) else {
            return false;
        };
        if tab.zoomed {
            return false;
        }
        let placed = tab.placed();
        let Some(&(_, from)) = placed.iter().find(|(p, _)| *p == tab.focused) else {
            return false;
        };
        let best = placed
            .iter()
            .filter(|(p, _)| *p != tab.focused)
            .filter_map(|&(p, r)| neighbour(from, r, dir).map(|shared| (p, shared)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((p, _)) => {
                tab.focus(p);
                true
            }
            None => false,
        }
    }

    pub fn select_tab(&mut self, index: usize) -> bool {
        if index < self.tabs.len() {
            self.current = index;
            true
        } else {
            false
        }
    }

    /// The next tab, or back to the first; `by` negative goes back.
    pub fn cycle_tab(&mut self, by: isize) {
        let n = self.tabs.len() as isize;
        if n > 0 {
            self.current = (self.current as isize + by).rem_euclid(n) as usize;
        }
    }

    /// Moves the current tab to `index`.
    pub fn move_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() || self.tabs.is_empty() {
            return false;
        }
        let tab = self.tabs.remove(self.current);
        self.tabs.insert(index, tab);
        self.current = index;
        true
    }

    /// Fills the tab with the focused pane, or puts it back.
    pub fn toggle_zoom(&mut self) -> bool {
        match self.tabs.get_mut(self.current) {
            Some(tab) if tab.root != Node::Pane(tab.focused) => {
                tab.zoomed = !tab.zoomed;
                true
            }
            _ => false,
        }
    }

    /// Grows the focused pane toward `dir` by `by` of its split.
    pub fn grow(&mut self, dir: Direction, by: f64) -> bool {
        match self.tabs.get_mut(self.current) {
            Some(tab) => {
                let pane = tab.focused;
                tab.root.grow(pane, dir, by)
            }
            None => false,
        }
    }

    /// Sets the split at `path` of the current tab, as a dragged divider.
    pub fn set_ratio(&mut self, path: &str, ratio: f64) -> bool {
        let Some(tab) = self.tabs.get_mut(self.current) else {
            return false;
        };
        match tab.root.at_path(path) {
            Some(Node::Split { ratio: r, .. }) => {
                *r = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
                true
            }
            _ => false,
        }
    }

    /// Every pane of every tab, where it goes.
    pub fn placed(&self) -> Vec<Placed> {
        let mut out = Vec::new();
        for (index, tab) in self.tabs.iter().enumerate() {
            let shown = index == self.current;
            for (pane, rect) in tab.placed() {
                let focused = pane == tab.focused;
                let (rect, visible) = if tab.zoomed {
                    (Rect::FULL, shown && focused)
                } else {
                    (rect, shown)
                };
                out.push(Placed {
                    pane,
                    tab: index,
                    rect,
                    focused,
                    visible,
                });
            }
        }
        out
    }

    /// The current tab's dividers; none while a pane is zoomed.
    pub fn dividers(&self) -> Vec<Divider> {
        let mut out = Vec::new();
        if let Some(tab) = self.current_tab().filter(|t| !t.zoomed) {
            tab.root.dividers(Rect::FULL, &mut String::new(), &mut out);
        }
        out
    }

    /// Zooms tab `index`'s focused pane, or stops; for a restored session.
    pub fn set_zoom(&mut self, index: usize, zoomed: bool) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.zoomed = zoomed;
        }
    }

    /// A layout of `tabs`, as saved: for restoring a session. Pane ids are
    /// kept; new ones start after the largest.
    pub fn restore(tabs: Vec<(Node, PaneId)>, current: usize) -> Self {
        let mut next = 0;
        let tabs: Vec<Tab> = tabs
            .into_iter()
            .map(|(root, focused)| {
                let mut all = Vec::new();
                root.panes(&mut all);
                next = all.iter().map(|p| p.0).fold(next, u32::max);
                let focused = if all.contains(&focused) {
                    focused
                } else {
                    all[0]
                };
                Tab {
                    root,
                    ..Tab::new(focused)
                }
            })
            .collect();
        let current = current.min(tabs.len().saturating_sub(1));
        Self {
            tabs,
            current,
            next,
        }
    }
}

/// How much of `from`'s side toward `dir` the pane at `to` shares, if it
/// touches that side.
fn neighbour(from: Rect, to: Rect, dir: Direction) -> Option<f64> {
    const EPS: f64 = 1e-9;
    let overlap = |a0: f64, a1: f64, b0: f64, b1: f64| a1.min(b1) - a0.max(b0);
    let (touches, shared) = match dir {
        Direction::Right => (
            (to.x - (from.x + from.w)).abs() < EPS,
            overlap(from.y, from.y + from.h, to.y, to.y + to.h),
        ),
        Direction::Left => (
            (from.x - (to.x + to.w)).abs() < EPS,
            overlap(from.y, from.y + from.h, to.y, to.y + to.h),
        ),
        Direction::Down => (
            (to.y - (from.y + from.h)).abs() < EPS,
            overlap(from.x, from.x + from.w, to.x, to.x + to.w),
        ),
        Direction::Up => (
            (from.y - (to.y + to.h)).abs() < EPS,
            overlap(from.x, from.x + from.w, to.x, to.x + to.w),
        ),
    };
    (touches && shared > EPS).then_some(shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_of(layout: &Layout, pane: PaneId) -> Rect {
        layout
            .placed()
            .into_iter()
            .find(|p| p.pane == pane)
            .unwrap()
            .rect
    }

    #[test]
    fn a_split_puts_the_new_pane_beside_and_focuses_it() {
        let mut l = Layout::new();
        let a = l.new_tab();
        let b = l.split(Axis::Horizontal).unwrap();
        assert_eq!(l.focused(), Some(b));
        assert_eq!(rect_of(&l, a).w, 0.5);
        assert_eq!(rect_of(&l, b).x, 0.5);
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_its_sibling() {
        let mut l = Layout::new();
        let a = l.new_tab();
        let b = l.split(Axis::Horizontal).unwrap();
        let c = l.split(Axis::Vertical).unwrap();
        assert_eq!(l.close(c), Some(Closed::Pane));
        assert_eq!(
            rect_of(&l, b),
            Rect {
                x: 0.5,
                y: 0.0,
                w: 0.5,
                h: 1.0
            }
        );
        assert_eq!(l.focused(), Some(b), "focus goes back where it was");
        assert_eq!(l.close(b), Some(Closed::Pane));
        assert_eq!(rect_of(&l, a), Rect::FULL);
        assert_eq!(l.close(a), Some(Closed::Everything));
        assert!(l.is_empty());
    }

    #[test]
    fn focus_moves_to_the_pane_that_shares_most_of_the_side() {
        // a | b over c, with b taller than c.
        let mut l = Layout::new();
        let a = l.new_tab();
        let b = l.split(Axis::Horizontal).unwrap();
        let c = l.split(Axis::Vertical).unwrap();
        // c gives up a fifth of its height to b.
        assert!(l.grow(Direction::Up, -0.2));
        assert!(l.focus_toward(Direction::Up));
        assert_eq!(l.focused(), Some(b));
        assert!(l.focus_toward(Direction::Left));
        assert_eq!(l.focused(), Some(a));
        assert!(!l.focus_toward(Direction::Left), "nothing further left");
        assert!(l.focus_toward(Direction::Right));
        assert_eq!(l.focused(), Some(b), "b shares more of a's side than c");
        let _ = c;
    }

    #[test]
    fn growing_moves_the_nearest_border_on_that_side() {
        let mut l = Layout::new();
        let a = l.new_tab();
        l.split(Axis::Horizontal).unwrap();
        l.focus_pane(a);
        assert!(l.grow(Direction::Right, 0.1));
        assert!((rect_of(&l, a).w - 0.6).abs() < 1e-9);
        assert!(!l.grow(Direction::Left, 0.1), "a has no border on the left");
        assert!(!l.grow(Direction::Up, 0.1));
    }

    #[test]
    fn a_zoomed_pane_fills_its_tab_and_hides_the_rest() {
        let mut l = Layout::new();
        let a = l.new_tab();
        let b = l.split(Axis::Horizontal).unwrap();
        assert!(l.toggle_zoom());
        let placed = l.placed();
        let shown: Vec<_> = placed.iter().filter(|p| p.visible).collect();
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].pane, shown[0].rect), (b, Rect::FULL));
        assert!(l.dividers().is_empty());
        // Splitting shows them all again.
        l.split(Axis::Vertical);
        assert!(l.placed().iter().all(|p| p.visible));
        let _ = a;
    }

    #[test]
    fn tabs_open_after_the_current_one_and_cycle() {
        let mut l = Layout::new();
        let a = l.new_tab();
        let b = l.new_tab();
        l.select_tab(0);
        let c = l.new_tab();
        let order: Vec<_> = l.tabs().iter().map(|t| t.focused).collect();
        assert_eq!(order, vec![a, c, b]);
        l.cycle_tab(2);
        assert_eq!(l.focused(), Some(a));
        l.cycle_tab(-1);
        assert_eq!(l.focused(), Some(b));
        // Only the current tab's panes are on screen.
        let visible: Vec<_> = l
            .placed()
            .into_iter()
            .filter(|p| p.visible)
            .map(|p| p.pane)
            .collect();
        assert_eq!(visible, vec![b]);
    }

    #[test]
    fn closing_the_current_tab_shows_its_neighbour() {
        let mut l = Layout::new();
        let a = l.new_tab();
        let b = l.new_tab();
        let c = l.new_tab();
        l.select_tab(1);
        assert_eq!(l.close(b), Some(Closed::Tab));
        assert_eq!(l.focused(), Some(c), "the one after it, as in a browser");
        assert_eq!(l.close(c), Some(Closed::Tab));
        assert_eq!(l.focused(), Some(a), "the one before the last");
    }

    #[test]
    fn dividers_can_be_dragged_by_path() {
        let mut l = Layout::new();
        let a = l.new_tab();
        l.split(Axis::Horizontal);
        l.split(Axis::Vertical);
        let paths: Vec<_> = l.dividers().into_iter().map(|d| d.path).collect();
        assert_eq!(paths, vec!["".to_owned(), "1".to_owned()]);
        assert!(l.set_ratio("", 0.25));
        assert_eq!(rect_of(&l, a).w, 0.25);
        assert!(l.set_ratio("1", 0.0));
        assert_eq!(l.dividers()[1].ratio, MIN_RATIO);
        assert!(!l.set_ratio("0", 0.5), "the first half is a pane");
    }

    #[test]
    fn a_restored_layout_hands_out_new_ids() {
        let root = Node::Split {
            axis: Axis::Vertical,
            ratio: 0.3,
            first: Box::new(Node::Pane(PaneId(4))),
            second: Box::new(Node::Pane(PaneId(9))),
        };
        let mut l = Layout::restore(vec![(root, PaneId(9))], 0);
        assert_eq!(l.focused(), Some(PaneId(9)));
        assert_eq!(l.new_tab(), PaneId(10));
    }
}
