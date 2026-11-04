//! `hof agent`: Hof's helper on the machine a remote pane runs on.
//!
//! The agent runs each pane's program on a pty of its own machine, through
//! the same [`hof_vt::Session`] a local pane uses, so it knows the screen
//! as well as the program does. It keeps the newest output of each pane,
//! numbered from the pane's start. A client that drops and comes back says
//! how far it got and gets the rest, or the screen as it is when the rest
//! is gone. The programs run on meanwhile.
//!
//! [`Agent`] is the panes and the protocol, over any reader and writer.
//! [`daemon`] is the agent as a process that outlives its connections, on
//! a unix socket, and the `--stdio` bridge ssh runs to reach it.

mod agent;
pub mod daemon;

pub use agent::{Agent, Options};
