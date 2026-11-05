//! Finding the programs Hof starts for remote panes: a server for their
//! windows or their sound, where this machine has none running.

use std::path::{Path, PathBuf};

/// The program `$var` names, else the first of `name` on the `PATH` and
/// `elsewhere` that is one. Only `$var` is looked at off a Mac, where a
/// desktop runs its own servers.
pub(crate) fn find(var: &str, name: &str, elsewhere: &[PathBuf]) -> Option<PathBuf> {
    if let Some(program) = std::env::var_os(var).filter(|p| !p.is_empty()) {
        return Some(program.into());
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(name));
    on_path
        .chain(elsewhere.iter().cloned())
        .find(|p| is_program(p))
}

fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// A directory only this user can enter, of this Hof's own: one for each
/// call, so two servers starting at once don't share one, and one left by
/// a Hof that died is not reused.
pub(crate) fn private_dir(what: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    static MADE: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hof-{what}-{}-{}",
        std::process::id(),
        MADE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", dir.display())))?;
    Ok(dir)
}

/// Waits for `child` to listen on `socket`; an error if it stops first or
/// takes longer than `within`.
pub(crate) fn wait_to_listen(
    child: &mut std::process::Child,
    program: &Path,
    socket: &Path,
    within: std::time::Duration,
) -> std::io::Result<()> {
    let began = std::time::Instant::now();
    loop {
        if std::os::unix::net::UnixStream::connect(socket).is_ok() {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(std::io::Error::other(format!(
                "{} stopped before it listened ({status})",
                program.display()
            )));
        }
        if began.elapsed() > within {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "{} did not listen on {}",
                    program.display(),
                    socket.display()
                ),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
