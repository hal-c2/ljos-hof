//! Remote panes: what Hof and `hof agent`, its helper on another machine,
//! say to each other, and the streams that let a pane outlast its
//! connection.
//!
//! A remote pane's program runs under the agent, which keeps its output
//! numbered and the newest of it at hand ([`Backlog`]). Hof keeps how far
//! it got ([`Received`]). When a connection drops and another comes, Hof
//! says how far it got, and gets the rest, or a snapshot of the screen
//! when the rest is gone. What the user types goes the same way, the other
//! way round. The messages are in [`wire`], Hof's end of a connection in
//! [`client`]; see `docs/remote.md` for the
//! whole design.

pub mod client;
pub mod stream;
pub mod wire;

pub use stream::{Backlog, Gap, Received, Resume};
pub use wire::{Decoder, Message, PaneId, PaneInfo, VERSION};
