//! The pointer and the wheel: reported to a program that tracks the mouse,
//! and otherwise selecting and scrolling. Shift always selects, as in
//! every terminal, so text can be copied from a program that tracks.

use std::time::Duration;

use hof_vt::{Mods, MouseAction, MouseButton, MouseInput, Scroll, Selecting};
use ljos::item::{self, FocusReason, Modifiers, MouseButtons, PointerEvent, PointerItem};
use ljos::item::{WheelEvent, kurbo};
use ljos::prelude::*;

use crate::terminal::{Terminal, copy_selection, flush_input, scroll, shared_of, with_session};

/// Clicks this close together, on the same cell, count as one more.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// Lines one wheel notch scrolls.
const LINES_PER_NOTCH: f64 = 3.0;

#[derive(Debug, Default)]
pub struct PointerState {
    /// The button a press reported to the program holds: its moves and its
    /// release go to the program too.
    reporting: Option<MouseButton>,
    /// A press is selecting.
    selecting: bool,
    last_click: Option<(Time, (u16, u16))>,
    clicks: u8,
    /// What the wheel has moved short of a whole line.
    wheel_rest: f64,
}

/// The cell under `p`, clamped to the grid, and `p` in device pixels from
/// the grid's corner, as mouse reports want it.
fn locate(engine: &Engine, me: ObjectId, p: kurbo::Point) -> ((u16, u16), (f32, f32)) {
    let padding = item::prop(engine, me, Terminal::PADDING).max(0.0);
    let cols = item::prop(engine, me, Terminal::COLUMNS).max(1);
    let rows = item::prop(engine, me, Terminal::ROWS).max(1);
    let cell = shared_of(engine, me).map_or(kurbo::Size::new(8.0, 16.0), |s| {
        s.view.borrow().metrics.cell
    });
    let (x, y) = (p.x - padding, p.y - padding);
    let col = (x / cell.width).floor().clamp(0.0, f64::from(cols - 1)) as u16;
    let row = (y / cell.height).floor().clamp(0.0, f64::from(rows - 1)) as u16;
    let dpr = item::device_pixel_ratio(engine, me);
    (
        (col, row),
        ((x.max(0.0) * dpr) as f32, (y.max(0.0) * dpr) as f32),
    )
}

fn mods(m: Modifiers) -> Mods {
    let mut out = Mods::empty();
    out.set(Mods::SHIFT, m.contains(Modifiers::SHIFT));
    out.set(Mods::CTRL, m.contains(Modifiers::CONTROL));
    out.set(Mods::ALT, m.contains(Modifiers::ALT));
    out.set(Mods::SUPER, m.contains(Modifiers::META));
    out
}

fn button(b: MouseButtons) -> Option<MouseButton> {
    Some(match b {
        MouseButtons::LEFT => MouseButton::Left,
        MouseButtons::MIDDLE => MouseButton::Middle,
        MouseButtons::RIGHT => MouseButton::Right,
        MouseButtons::BACK => MouseButton::Back,
        MouseButtons::FORWARD => MouseButton::Forward,
        _ => return None,
    })
}

/// Whether the program gets the pointer rather than the selection.
fn reports(engine: &Engine, me: ObjectId, m: Modifiers) -> bool {
    !m.contains(Modifiers::SHIFT) && with_session(engine, me, |s| s.mouse_tracking()) == Some(true)
}

fn report(engine: &Engine, me: ObjectId, input: MouseInput) -> bool {
    let sent = with_session(engine, me, |s| s.mouse(&input)).is_some_and(|r| r.unwrap_or(false));
    flush_input(engine, me);
    sent
}

fn press(engine: &Engine, me: ObjectId, ev: &PointerEvent, p: kurbo::Point) -> bool {
    let Some(b) = ev.button.and_then(button) else {
        return false;
    };
    let Some(shared) = shared_of(engine, me) else {
        return false;
    };
    if b == MouseButton::Left {
        item::force_active_focus(engine, me, FocusReason::Mouse);
    }
    let (cell, (x, y)) = locate(engine, me, p);
    if reports(engine, me, ev.modifiers) {
        let input = MouseInput {
            action: MouseAction::Press,
            button: Some(b),
            mods: mods(ev.modifiers),
            x,
            y,
        };
        if report(engine, me, input) {
            shared.pointer.borrow_mut().reporting = Some(b);
            return true;
        }
    }
    match b {
        MouseButton::Left => {
            let unit = {
                let mut ptr = shared.pointer.borrow_mut();
                let again = ptr.last_click.is_some_and(|(t, at)| {
                    at == cell && ev.time.saturating_since(t) <= MULTI_CLICK
                });
                ptr.clicks = if again { ptr.clicks % 3 + 1 } else { 1 };
                ptr.last_click = Some((ev.time, cell));
                ptr.selecting = true;
                match ptr.clicks {
                    2 => Selecting::Words,
                    3 => Selecting::Lines,
                    _ if ev.modifiers.contains(Modifiers::ALT) => Selecting::Block,
                    _ => Selecting::Cells,
                }
            };
            let extend = ev.modifiers.contains(Modifiers::SHIFT)
                && unit == Selecting::Cells
                && with_session(engine, me, |s| s.has_selection()) == Some(true);
            with_session(engine, me, |s| {
                if extend {
                    s.select_extend(cell.0, cell.1)
                } else {
                    s.clear_selection();
                    s.select_start(cell.0, cell.1, unit)
                }
            });
            true
        }
        // Middle-click pastes the primary selection, as X11 does.
        MouseButton::Middle => {
            crate::terminal::paste_from(engine, me, ClipboardMode::Selection);
            true
        }
        // Right-click is for the item's context menu, in QML.
        _ => false,
    }
}

fn moved(engine: &Engine, me: ObjectId, ev: &PointerEvent, p: kurbo::Point) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let (reporting, selecting) = {
        let ptr = shared.pointer.borrow();
        (ptr.reporting, ptr.selecting)
    };
    let (cell, (x, y)) = locate(engine, me, p);
    if let Some(b) = reporting {
        report(
            engine,
            me,
            MouseInput {
                action: MouseAction::Motion,
                button: Some(b),
                mods: mods(ev.modifiers),
                x,
                y,
            },
        );
    } else if selecting {
        with_session(engine, me, |s| s.select_extend(cell.0, cell.1));
    }
}

fn release(engine: &Engine, me: ObjectId, ev: &PointerEvent, p: kurbo::Point) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let (reporting, selecting) = {
        let mut ptr = shared.pointer.borrow_mut();
        (ptr.reporting.take(), std::mem::take(&mut ptr.selecting))
    };
    if let Some(b) = reporting {
        let (_, (x, y)) = locate(engine, me, p);
        report(
            engine,
            me,
            MouseInput {
                action: MouseAction::Release,
                button: Some(b),
                mods: mods(ev.modifiers),
                x,
                y,
            },
        );
    } else if selecting {
        // What is selected is the primary selection, as on X11.
        copy_selection(engine, me, ClipboardMode::Selection);
        if engine.untrack(|| item::prop(engine, me, Terminal::COPY_ON_SELECT)) {
            copy_selection(engine, me, ClipboardMode::Clipboard);
        }
    }
}

fn cancel(engine: &Engine, me: ObjectId) {
    if let Some(shared) = shared_of(engine, me) {
        let mut ptr = shared.pointer.borrow_mut();
        ptr.reporting = None;
        ptr.selecting = false;
    }
}

/// A move with no button held: reported to a program that asked for every
/// motion (mode 1003); the encoder drops it for one that didn't.
pub fn hover_move(engine: &Engine, me: ObjectId, p: kurbo::Point) {
    if !reports(engine, me, Modifiers::empty()) {
        return;
    }
    let (_, (x, y)) = locate(engine, me, p);
    report(
        engine,
        me,
        MouseInput {
            action: MouseAction::Motion,
            button: None,
            mods: Mods::empty(),
            x,
            y,
        },
    );
}

pub const POINTER: PointerItem = PointerItem {
    press,
    moved,
    release,
    cancel,
    hover: |_, _, _| {},
};

/// The wheel scrolls the view by lines, or goes to a program that tracks
/// the mouse as wheel buttons, a notch at a time.
pub fn wheel(engine: &Engine, me: ObjectId, ev: &WheelEvent, p: kurbo::Point) -> bool {
    let Some(shared) = shared_of(engine, me) else {
        return false;
    };
    let cell_height = shared.view.borrow().metrics.cell.height.max(1.0);
    // Notches, in lines; a touchpad's pixels where it gives no notches.
    let by = if ev.angle_delta.y != 0.0 {
        ev.angle_delta.y / WheelEvent::NOTCH * LINES_PER_NOTCH
    } else {
        ev.pixel_delta.y / cell_height
    };
    let lines = {
        let mut ptr = shared.pointer.borrow_mut();
        let total = ptr.wheel_rest + by;
        let whole = total.trunc();
        ptr.wheel_rest = total - whole;
        whole as isize
    };
    if lines == 0 {
        return true;
    }
    if reports(engine, me, ev.modifiers) {
        let (_, (x, y)) = locate(engine, me, p);
        let b = if lines > 0 {
            MouseButton::WheelUp
        } else {
            MouseButton::WheelDown
        };
        let notches = (lines.unsigned_abs() as f64 / LINES_PER_NOTCH).ceil() as usize;
        for _ in 0..notches {
            report(
                engine,
                me,
                MouseInput {
                    action: MouseAction::Press,
                    button: Some(b),
                    mods: mods(ev.modifiers),
                    x,
                    y,
                },
            );
        }
        return true;
    }
    // Up on the wheel is back in the scrollback.
    scroll(engine, me, Scroll::Lines(-lines));
    true
}
