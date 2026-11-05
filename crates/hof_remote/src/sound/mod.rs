//! Hof's own sound server, for remote panes on a machine with none of its
//! own to pass sound to, as on a Mac.
//!
//! Programs on the agent's machine speak PulseAudio's protocol to what
//! they take for a PulseAudio server; the agent passes their connections
//! to Hof, which serves them itself ([`Sound`]) and plays what they send
//! through this machine's speakers. Nothing needs installing.

mod mixer;
mod server;
#[cfg(all(feature = "speakers", target_os = "macos"))]
mod speakers;
mod tagstruct;

use std::sync::Arc;

pub use mixer::Mixer;
pub use server::Sound;

/// The rate a mixer plays at when the speakers have not said.
const RATE: u32 = 48_000;

/// A sound server playing through this machine's speakers, where Hof can
/// play through them itself: on a Mac.
pub fn speakers() -> Option<Sound> {
    #[cfg(all(feature = "speakers", target_os = "macos"))]
    {
        let mixer = Arc::new(Mixer::new(speakers::rate().unwrap_or(RATE)));
        speakers::start(mixer.clone()).ok()?;
        Some(Sound::new(mixer))
    }
    #[cfg(not(all(feature = "speakers", target_os = "macos")))]
    {
        None
    }
}

/// A sound server playing into a mixer nothing takes from but the caller:
/// for trying the server where it has no speakers.
pub fn silent() -> Sound {
    Sound::new(Arc::new(Mixer::new(RATE)))
}
