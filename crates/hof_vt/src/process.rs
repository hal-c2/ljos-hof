//! Which program a terminal is running right now: the process group its
//! pty gives the keys to, as the kernel says.
//!
//! A shell runs one command at a time in the foreground, in a process group
//! of its own, and hands the pty to it; `tcgetpgrp` names that group, and
//! its leader is the command the user typed. That is how Hof tells an agent
//! from the shell it was started in, without the agent's help.

use std::os::fd::AsFd;
use std::path::Path;

/// A process, as `/proc` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    /// The kernel's name for it (`comm`): the program's file name cut to 15
    /// bytes, or what the program renamed itself to.
    pub name: String,
    /// Its command line, the program first.
    pub args: Vec<String>,
}

impl Process {
    /// Whether it is the program `name`: by its own name, or by the file it
    /// runs, which for a script is the interpreter's first argument
    /// (`node /usr/lib/node_modules/x/bin/x.js` is `x`).
    pub fn is(&self, name: &str) -> bool {
        self.name == name
            || self
                .args
                .iter()
                .take(2)
                .any(|a| file_name(a) == name || file_stem(a) == name)
    }
}

fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

fn file_stem(path: &str) -> &str {
    Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

/// The leader of the process group in the foreground of the terminal whose
/// master side is `master`; `None` when the kernel won't say, or `/proc`
/// has no such process (it ended meanwhile, or this is not Linux).
pub(crate) fn foreground(master: impl AsFd) -> Option<Process> {
    let group = rustix::termios::tcgetpgrp(master).ok()?;
    let pid = u32::try_from(group.as_raw_nonzero().get()).ok()?;
    of(pid)
}

/// The process `pid`, from `/proc`.
pub fn of(pid: u32) -> Option<Process> {
    let dir = Path::new("/proc").join(pid.to_string());
    let name = std::fs::read_to_string(dir.join("comm")).ok()?;
    let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
    let args = cmdline
        .split(|&b| b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    Some(Process {
        pid,
        name: name.trim_end_matches('\n').to_owned(),
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(name: &str, args: &[&str]) -> Process {
        Process {
            pid: 1,
            name: name.into(),
            args: args.iter().map(|&a| a.into()).collect(),
        }
    }

    #[test]
    fn a_program_is_known_by_its_own_name() {
        assert!(process("claude", &["claude", "--resume"]).is("claude"));
        assert!(!process("bash", &["-bash"]).is("claude"));
    }

    #[test]
    fn a_script_is_known_by_the_file_its_interpreter_runs() {
        let p = process(
            "node",
            &["node", "/usr/lib/node_modules/codex/bin/codex.js"],
        );
        assert!(p.is("codex"));
        assert!(!p.is("claude"));
    }

    #[test]
    fn a_program_run_by_its_path_is_known_by_its_file_name() {
        assert!(process("python3", &["/home/me/.local/bin/aider"]).is("aider"));
    }

    #[test]
    fn this_process_is_in_proc() {
        let me = of(std::process::id()).expect("this is Linux");
        assert!(!me.name.is_empty());
        assert!(!me.args.is_empty());
    }
}
