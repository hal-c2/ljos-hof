//! The QML side of Ljós Hof.
//!
//! [`Terminal`] is a native item: it owns a [`hof_vt::Session`], paints its
//! grid, and hands the session its keys, pointer, wheel and input method.
//! The session's reader thread wakes the item through the platform's inbox,
//! so an idle terminal costs nothing: no timer, no polling.
//!
//! An item can also stand in for a session that lives elsewhere
//! (`external: true`): output comes in through `write()`, and what the user
//! types leaves through the `input` signal. That is the shape hal-c2's
//! terminal had, and what a remote session or a replayed one needs.

pub mod agents;
mod keys;
pub mod layout;
mod paint;
mod pointer;
pub mod remote;
mod terminal;
mod workspace;

use std::rc::Rc;

use ljos::item::{self, KeyItem, WheelItem};
use ljos::prelude::*;

pub use keys::translate as translate_key;
pub use terminal::Terminal;
pub use workspace::Workspace;

/// The one seam a host calls: registers `import Hof 1.0`.
pub fn register(engine: &Engine) {
    engine.register_module(ljos::module!("Hof" 1.0 => [Terminal, Workspace]));
    item::register_paint_item::<Terminal>(engine, paint::read);
    item::register_key_item::<Terminal>(
        engine,
        KeyItem {
            shortcut_override: |_, _, _| false,
            press: terminal::key_press,
            release: terminal::key_release,
        },
    );
    item::register_pointer_item::<Terminal>(engine, pointer::POINTER);
    item::register_hover_move::<Terminal>(engine, pointer::hover_move);
    item::register_wheel_item::<Terminal>(
        engine,
        WheelItem {
            wheel: pointer::wheel,
        },
    );
    item::register_text_input_client::<Terminal>(engine, Rc::new(terminal::Typing));
}
