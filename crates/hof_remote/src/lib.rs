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
//!
//! Beside the panes, a connection carries other connections: X11 and
//! sound clients on the agent's machine, passed on to the servers on
//! Hof's ([`relay`], [`x11`], [`pulse`]).

pub mod bootstrap;
pub mod client;
pub mod pulse;
pub mod relay;
pub mod stream;
pub mod wire;
pub mod x11;

pub use relay::{Relays, Socket};
pub use stream::{Backlog, Gap, Received, Resume};
pub use wire::{ChannelId, Decoder, Message, PaneId, PaneInfo, Process, VERSION};
