use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("pseudo-terminal: {0} failed: {1}")]
    Pty(&'static str, #[source] std::io::Error),
    #[error("could not start {path}: {source}", path = .0.display(), source = .1)]
    Spawn(PathBuf, #[source] std::io::Error),
    #[error("could not start the session's {0} thread: {1}")]
    Thread(&'static str, #[source] std::io::Error),
    #[error("terminal: {0}")]
    Vt(#[from] libghostty_vt::Error),
}
