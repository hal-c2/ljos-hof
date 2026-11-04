//! `Workspace`: a window's tabs and splits ([`Layout`]) for QML.
//!
//! QML keeps one terminal per pane, made once and placed where `panes()`
//! says. `revision` moves on every change, so a view re-reads the layout
//! without being told what changed.

use std::cell::RefCell;
use std::rc::Rc;

use ljos::prelude::*;

use crate::layout::{Axis, Closed, Direction, Layout, PaneId};

#[derive(Debug, Default, QmlObject)]
#[qml(module = "Hof", name = "Workspace")]
pub struct Workspace {
    #[qml(notify, readonly)]
    pub tab_count: i32,
    #[qml(notify, readonly)]
    pub current_tab: i32,
    /// The focused pane of the current tab; 0 for none.
    #[qml(notify, readonly)]
    pub focused_pane: i32,
    #[qml(notify, readonly)]
    pub zoomed: bool,
    /// Moves on every change of the layout.
    #[qml(notify, readonly)]
    pub revision: i32,
    #[qml(skip)]
    layout: Rc<RefCell<Layout>>,
}

fn direction(name: &str) -> Option<Direction> {
    Some(match name {
        "left" => Direction::Left,
        "right" => Direction::Right,
        "up" => Direction::Up,
        "down" => Direction::Down,
        _ => return None,
    })
}

fn layout_of(cx: &Cx) -> Rc<RefCell<Layout>> {
    cx.engine
        .with::<Workspace, _>(cx.this, |w| Rc::clone(&w.layout))
        .unwrap_or_default()
}

/// Runs `f` on the layout, then tells QML what it changed.
fn change<R>(cx: &Cx, f: impl FnOnce(&mut Layout) -> R) -> R {
    let layout = layout_of(cx);
    let out = f(&mut layout.borrow_mut());
    publish(cx, &layout);
    out
}

/// Tells QML how the layout stands now.
fn publish(cx: &Cx, layout: &RefCell<Layout>) {
    // Read, then let go: the handlers of what is set next call back in.
    let (tabs, current, focused, zoomed) = {
        let l = layout.borrow();
        let tab = l.current_tab();
        (
            l.tabs().len() as i32,
            l.current() as i32,
            tab.map_or(0, |t| t.focused.0 as i32),
            tab.is_some_and(|t| t.zoomed),
        )
    };
    let (e, me) = (cx.engine, cx.this);
    let _ = e.set_internal(me, Workspace::TAB_COUNT, tabs);
    let _ = e.set_internal(me, Workspace::CURRENT_TAB, current);
    let _ = e.set_internal(me, Workspace::FOCUSED_PANE, focused);
    let _ = e.set_internal(me, Workspace::ZOOMED, zoomed);
    let revision = cx.get(Workspace::REVISION).unwrap_or(0);
    let _ = e.set_internal(me, Workspace::REVISION, revision.wrapping_add(1));
}

fn pane(id: i32) -> PaneId {
    PaneId(id.max(0) as u32)
}

#[qml_impl]
impl Workspace {
    /// Opens a tab with one pane; its id.
    #[qml(invokable)]
    fn new_tab(cx: &Cx) -> i32 {
        change(cx, |l| l.new_tab().0 as i32)
    }

    /// Splits the focused pane: `"right"` puts the new pane beside it,
    /// `"down"` under it. The new pane's id, or 0.
    #[qml(invokable)]
    fn split(cx: &Cx, toward: String) -> i32 {
        let axis = match toward.as_str() {
            "right" | "horizontal" => Axis::Horizontal,
            _ => Axis::Vertical,
        };
        change(cx, |l| l.split(axis).map_or(0, |p| p.0 as i32))
    }

    /// Closes a pane: `"pane"`, `"tab"` when it was its tab's last,
    /// `"everything"` when it was the last of all, `""` for no such pane.
    #[qml(invokable)]
    fn close_pane(cx: &Cx, id: i32) -> String {
        let closed = change(cx, |l| l.close(pane(id)));
        match closed {
            Some(Closed::Pane) => "pane",
            Some(Closed::Tab) => "tab",
            Some(Closed::Everything) => "everything",
            None => "",
        }
        .to_owned()
    }

    /// Closes the current tab and every pane in it; the ids closed.
    #[qml(invokable)]
    fn close_tab(cx: &Cx) -> Vec<i32> {
        change(cx, |l| {
            let panes = l.current_tab().map(|t| t.panes()).unwrap_or_default();
            for p in &panes {
                l.close(*p);
            }
            panes.into_iter().map(|p| p.0 as i32).collect()
        })
    }

    #[qml(invokable)]
    fn focus_pane(cx: &Cx, id: i32) -> bool {
        change(cx, |l| l.focus_pane(pane(id)))
    }

    /// Focuses the pane `"left"`, `"right"`, `"up"` or `"down"` of the
    /// focused one.
    #[qml(invokable)]
    fn focus_toward(cx: &Cx, dir: String) -> bool {
        let Some(dir) = direction(&dir) else {
            return false;
        };
        change(cx, |l| l.focus_toward(dir))
    }

    #[qml(invokable)]
    fn select_tab(cx: &Cx, index: i32) -> bool {
        change(cx, |l| l.select_tab(index.max(0) as usize))
    }

    #[qml(invokable)]
    fn next_tab(cx: &Cx) {
        change(cx, |l| l.cycle_tab(1));
    }

    #[qml(invokable)]
    fn previous_tab(cx: &Cx) {
        change(cx, |l| l.cycle_tab(-1));
    }

    #[qml(invokable)]
    fn move_tab(cx: &Cx, index: i32) -> bool {
        change(cx, |l| l.move_tab(index.max(0) as usize))
    }

    #[qml(invokable)]
    fn toggle_zoom(cx: &Cx) -> bool {
        change(cx, |l| l.toggle_zoom())
    }

    /// Grows the focused pane toward `dir` by `by` of its split.
    #[qml(invokable)]
    fn grow(cx: &Cx, dir: String, by: f64) -> bool {
        let Some(dir) = direction(&dir) else {
            return false;
        };
        change(cx, |l| l.grow(dir, by))
    }

    /// Sets a split's share, as its divider is dragged.
    #[qml(invokable)]
    fn set_ratio(cx: &Cx, path: String, ratio: f64) -> bool {
        change(cx, |l| l.set_ratio(&path, ratio))
    }

    /// Every pane: `{ pane, tab, x, y, width, height, focused, visible }`,
    /// placed in fractions of the window.
    #[qml(invokable)]
    fn panes(cx: &Cx) -> Value {
        let layout = layout_of(cx);
        let placed = layout.borrow().placed();
        Value::list(
            placed
                .into_iter()
                .map(|p| {
                    Value::map([
                        ("pane", Value::from(p.pane.0 as i32)),
                        ("tab", Value::from(p.tab as i32)),
                        ("x", Value::from(p.rect.x)),
                        ("y", Value::from(p.rect.y)),
                        ("width", Value::from(p.rect.w)),
                        ("height", Value::from(p.rect.h)),
                        ("focused", Value::from(p.focused)),
                        ("visible", Value::from(p.visible)),
                    ])
                })
                .collect(),
        )
    }

    /// The current tab's dividers: `{ path, vertical, ratio, x, y, width,
    /// height }`, where the rect is all of the split.
    #[qml(invokable)]
    fn dividers(cx: &Cx) -> Value {
        let layout = layout_of(cx);
        let dividers = layout.borrow().dividers();
        Value::list(
            dividers
                .into_iter()
                .map(|d| {
                    Value::map([
                        ("path", Value::from(d.path.as_str())),
                        ("vertical", Value::from(d.axis == Axis::Vertical)),
                        ("ratio", Value::from(d.ratio)),
                        ("x", Value::from(d.within.x)),
                        ("y", Value::from(d.within.y)),
                        ("width", Value::from(d.within.w)),
                        ("height", Value::from(d.within.h)),
                    ])
                })
                .collect(),
        )
    }

    /// The panes of tab `index`, in reading order.
    #[qml(invokable)]
    fn tab_panes(cx: &Cx, index: i32) -> Vec<i32> {
        let layout = layout_of(cx);
        let l = layout.borrow();
        l.tabs()
            .get(index.max(0) as usize)
            .map(|t| t.panes().into_iter().map(|p| p.0 as i32).collect())
            .unwrap_or_default()
    }

    /// The focused pane of tab `index`, or 0.
    #[qml(invokable)]
    fn tab_focus(cx: &Cx, index: i32) -> i32 {
        let layout = layout_of(cx);
        let l = layout.borrow();
        l.tabs()
            .get(index.max(0) as usize)
            .map_or(0, |t| t.focused.0 as i32)
    }
}

impl Workspace {
    /// The layout, for Rust that saves or drives it (session restore, MCP).
    pub fn layout(engine: &Engine, me: ObjectId) -> Option<Rc<RefCell<Layout>>> {
        engine
            .with::<Workspace, _>(me, |w| Rc::clone(&w.layout))
            .ok()
    }

    /// Puts `layout` in place of the one there (a restored session), and
    /// tells QML.
    pub fn replace(engine: &Engine, me: ObjectId, layout: Layout) {
        let Some(current) = Self::layout(engine, me) else {
            return;
        };
        *current.borrow_mut() = layout;
        publish(&Cx::new(engine, me), &current);
    }
}
