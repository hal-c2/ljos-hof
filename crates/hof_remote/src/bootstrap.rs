//! Reaching a machine that has no Hof: Hof copies its agent there.
//!
//! The ssh command Hof runs prefers the agent it copied for this version,
//! then the host's `hof`. With neither, it says so, with the machine's
//! system and processor, and exits with 127. The next time Hof connects,
//! it picks the static `hof-agent-<arch>-<os>` it has for that machine and
//! copies it over ssh, as `hof` in a directory of this version's:
//! `${XDG_DATA_HOME:-~/.local/share}/hof/<version>/`. The copy runs
//! `--version` there before it takes the place of an older one, so a copy
//! that can't run there never does.
//!
//! The commands go through the remote user's login shell, whichever it is,
//! so they are one `sh -c '...'` with no quote or backslash inside.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::client::{self, Connection, Ended, Transport};

/// What the agent's copy is called on that machine, so its panes find
/// `hof copy` and the rest.
pub const NAME: &str = "hof";

/// How long copying the agent may take.
const COPY_WAIT: Duration = Duration::from_secs(120);
/// How long asking the machine what it is may take.
const ASK_WAIT: Duration = Duration::from_secs(30);
/// Copies that failed before Hof stops trying, until it starts again.
const MOST_TRIES: u32 = 3;
/// The most output kept from one bootstrap command; the rest is drained.
const MOST_OUTPUT: usize = 64 * 1024;

/// What the snippet says when the machine has no Hof, before the machine.
const MISSING: &str = "Hof is not on this machine";

/// The directory, under Hof's data directory there, of the agent copied
/// for this Hof: its version and the wire's.
pub fn version_dir() -> String {
    format!("{}-{}", env!("CARGO_PKG_VERSION"), crate::VERSION)
}

/// A host reached with ssh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ssh {
    /// `ssh -T <destination>`: what runs a command there.
    pub argv: Vec<String>,
    /// The host's own Hof, a name on its `PATH` or a path.
    pub hof: String,
}

/// The host's `hof` could not be put in a command.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the host's `hof` ({0}) can't have quotes or backslashes in it")]
pub struct BadHof(pub String);

impl Ssh {
    /// The command that runs the agent there: ssh, then the snippet.
    pub fn agent_command(&self) -> Result<Vec<String>, BadHof> {
        if self.hof.contains(['\'', '"', '\\']) || self.hof.trim().is_empty() {
            return Err(BadHof(self.hof.clone()));
        }
        let hof = &self.hof;
        let script = format!(
            "{dir}; if [ -x \"$d/{NAME}\" ]; then exec \"$d/{NAME}\" agent --stdio; fi; \
             if command -v {hof} >/dev/null 2>&1; then exec {hof} agent --stdio; fi; \
             echo \"{MISSING} ($(uname -sm))\" >&2; exit 127",
            dir = dir_line(),
        );
        Ok(self.command(&script))
    }

    /// The command that writes what comes on its stdin to the agent's
    /// place there, once it has run there.
    ///
    /// It links `~/.local/bin/hof` to the copy too, unless something else
    /// is there: a login shell's profile may set its own `PATH` over the
    /// one the agent gives its panes, and the profiles Debian and Ubuntu
    /// give users put `~/.local/bin` on it. A link to an older copy is
    /// moved on to this one.
    fn copy_command(&self) -> Vec<String> {
        let part = format!("\"$d/.{NAME}.part\"");
        let script = format!(
            "{dir}; mkdir -p \"$d\" && cat > {part} && chmod 755 {part} && \
             {part} --version >/dev/null && mv -f {part} \"$d/{NAME}\" || \
             {{ rm -f {part}; exit 1; }}; \
             l=\"$HOME/.local/bin/{NAME}\"; \
             if [ ! -e \"$l\" ] || case \"$(readlink \"$l\")\" in */hof/*/{NAME}) true;; *) false;; esac; \
             then mkdir -p \"$HOME/.local/bin\" && ln -sf \"$d/{NAME}\" \"$l\"; fi; exit 0",
            dir = dir_line(),
        );
        self.command(&script)
    }

    fn command(&self, script: &str) -> Vec<String> {
        debug_assert!(!script.contains(['\'', '\\']), "{script}");
        let mut argv = self.argv.clone();
        argv.extend(["--".into(), format!("sh -c '{script}'")]);
        argv
    }
}

/// Sets `d` to the agent's directory there.
fn dir_line() -> String {
    format!(
        "d=\"${{XDG_DATA_HOME:-$HOME/.local/share}}/hof/{}\"",
        version_dir()
    )
}

/// A machine's system and processor, as Hof names its agents for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

impl Platform {
    /// From what `uname -sm` prints: `Linux x86_64`.
    pub fn of_uname(uname: &str) -> Option<Self> {
        let mut words = uname.split_whitespace();
        let (os, arch) = (words.next()?, words.next()?);
        if words.next().is_some() {
            return None;
        }
        let os = match os {
            "Darwin" => "macos".to_owned(),
            other => other.to_lowercase(),
        };
        let arch = match arch {
            "amd64" => "x86_64",
            "arm64" | "armv8l" => "aarch64",
            other => other,
        };
        // These words become a local filename, not just a description of
        // the remote machine. Never let one become a path.
        let safe = |word: &str| {
            !word.is_empty()
                && word.len() <= 32
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        };
        if !safe(&os) || !safe(arch) {
            return None;
        }
        Some(Self {
            os,
            arch: arch.to_owned(),
        })
    }

    /// What Hof calls its agent for this machine: `hof-agent-x86_64-linux`.
    pub fn agent_name(&self) -> String {
        format!("hof-agent-{}-{}", self.arch, self.os)
    }

    /// The machine the snippet said it is, when it said it has no Hof.
    fn said_missing(said: &[String]) -> Option<Self> {
        said.iter().rev().find_map(|line| {
            let rest = line.strip_prefix(MISSING)?.trim().strip_prefix('(')?;
            Self::of_uname(rest.strip_suffix(')')?)
        })
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} on {}", self.arch, self.os)
    }
}

/// Where Hof looks for the agents it can copy: `$HOF_AGENT_DIR`, beside
/// its own program, `../lib/hof` from there, and
/// `${XDG_DATA_HOME:-~/.local/share}/hof/agents`.
pub fn agent_dirs() -> Vec<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    let mut dirs: Vec<PathBuf> = var("HOF_AGENT_DIR")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_owned))
    {
        dirs.push(dir.join("../lib/hof"));
        dirs.insert(dirs.len() - 1, dir);
    }
    let data = var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| Path::new(&h).join(".local/share")));
    dirs.extend(data.map(|d| d.join("hof").join("agents")));
    dirs
}

/// The agent for `platform` in the first of `dirs` that has one.
pub fn find_agent(dirs: &[PathBuf], platform: &Platform) -> Option<PathBuf> {
    let name = platform.agent_name();
    dirs.iter().map(|d| d.join(&name)).find(|p| p.is_file())
}

/// Why the agent could not be copied there.
#[derive(Debug, thiserror::Error)]
pub enum CopyFailed {
    #[error(
        "Hof has no agent for {platform} to copy there; build `{name}` (mise run agents) and put it in {dirs}"
    )]
    NoAgent {
        platform: Platform,
        name: String,
        dirs: String,
    },
    #[error("can't tell what the machine is: {0}")]
    Unknown(String),
    #[error("copying Hof's agent there: {0}")]
    Copy(String),
}

/// A host reached with ssh, that gets Hof's agent when it has no Hof.
pub struct Bootstrapping {
    ssh: Ssh,
    dirs: Vec<PathBuf>,
    ended: Arc<Mutex<Ended>>,
    failures: u32,
}

impl Bootstrapping {
    pub fn new(ssh: Ssh, dirs: Vec<PathBuf>) -> Self {
        Self {
            ssh,
            dirs,
            ended: Arc::default(),
            failures: 0,
        }
    }

    /// Copies the agent there, for the machine `said` says it is, or asks
    /// it what it is.
    fn copy_agent(&self, said: &[String]) -> Result<PathBuf, CopyFailed> {
        let platform = match Platform::said_missing(said) {
            Some(p) => p,
            None => self.ask_platform()?,
        };
        let agent = find_agent(&self.dirs, &platform).ok_or_else(|| CopyFailed::NoAgent {
            name: platform.agent_name(),
            dirs: self
                .dirs
                .iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            platform,
        })?;
        let binary = std::fs::read(&agent)
            .map_err(|e| CopyFailed::Copy(format!("{}: {e}", agent.display())))?;
        run(&self.ssh.copy_command(), Some(binary), COPY_WAIT).map_err(CopyFailed::Copy)?;
        Ok(agent)
    }

    fn ask_platform(&self) -> Result<Platform, CopyFailed> {
        let out =
            run(&self.ssh.command("uname -sm"), None, ASK_WAIT).map_err(CopyFailed::Unknown)?;
        Platform::of_uname(&out).ok_or(CopyFailed::Unknown(out))
    }
}

impl Transport for Bootstrapping {
    fn connect(&mut self) -> io::Result<Connection> {
        let ended = std::mem::take(&mut *self.ended.lock().unwrap_or_else(PoisonError::into_inner));
        // 127 from the snippet, or from a `hof` there that can't run.
        let missing = ended.code == Some(127) || Platform::said_missing(&ended.said).is_some();
        if missing && self.failures < MOST_TRIES {
            if let Err(e) = self.copy_agent(&ended.said) {
                self.failures += 1;
                return Err(io::Error::other(e.to_string()));
            }
            self.failures = 0;
        }
        let argv = self
            .ssh
            .agent_command()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        client::spawn(&argv, Some(self.ended.clone()))
    }
}

/// Runs `argv` with `input` on its stdin, for up to `within`: what it
/// printed, or what went wrong.
fn run(argv: &[String], input: Option<Vec<u8>>, within: Duration) -> Result<String, String> {
    let (program, args) = argv.split_first().ok_or("no command")?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let feed = child.stdin.take().zip(input).map(|(mut stdin, bytes)| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        })
    });
    let reader = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut all = Vec::new();
            if let Some(mut r) = r {
                {
                    let mut kept = (&mut r).take(MOST_OUTPUT as u64);
                    let _ = kept.read_to_end(&mut all);
                }
                // The command may not stop writing: drain it without
                // letting its output take over this process's memory.
                let _ = io::copy(&mut r, &mut io::sink());
            }
            String::from_utf8_lossy(&all).into_owned()
        })
    };
    let out = reader(child.stdout.take().map(|o| Box::new(o) as _));
    let err = reader(child.stderr.take().map(|e| Box::new(e) as _));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < within => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer within {} s", within.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    if let Some(feed) = feed {
        let _ = feed.join();
    }
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if status.success() {
        Ok(out.trim().to_owned())
    } else {
        let said = err.trim();
        Err(if said.is_empty() {
            status.to_string()
        } else {
            said.lines().last().unwrap_or(said).to_owned()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(hof: &str) -> Ssh {
        Ssh {
            argv: vec!["ssh".into(), "-T".into(), "dev".into()],
            hof: hof.into(),
        }
    }

    #[test]
    fn the_agent_command_is_one_quoted_script_for_any_login_shell() {
        let argv = ssh("~/bin/hof").agent_command().unwrap();
        assert_eq!(argv[..4], ["ssh", "-T", "dev", "--"]);
        let script = &argv[4];
        assert!(script.starts_with("sh -c '") && script.ends_with('\''));
        assert_eq!(script.matches('\'').count(), 2, "{script}");
        assert!(!script.contains('\\'));
        assert!(script.contains(&format!("/hof/{}", version_dir())));
        assert!(script.contains("command -v ~/bin/hof"));
        assert_eq!(ssh("it's").agent_command(), Err(BadHof("it's".into())));
    }

    #[test]
    fn a_bootstrap_commands_output_is_bounded() {
        let argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "head -c 100000 /dev/zero".to_owned(),
        ];
        let out = run(&argv, None, ASK_WAIT).unwrap();
        assert_eq!(out.len(), MOST_OUTPUT);
    }

    #[test]
    fn a_machine_is_named_as_hof_names_its_agents() {
        let linux = Platform::of_uname("Linux x86_64\n").unwrap();
        assert_eq!(linux.agent_name(), "hof-agent-x86_64-linux");
        let mac = Platform::of_uname("Darwin arm64").unwrap();
        assert_eq!(mac.agent_name(), "hof-agent-aarch64-macos");
        assert_eq!(Platform::of_uname("Linux"), None);
    }

    #[test]
    fn a_platform_answer_cannot_become_a_local_path() {
        for answer in [
            "Linux ../../x",
            "Linux x/../y",
            "a/b c",
            "Linux x86_64 extra",
            "Linux x86-64",
        ] {
            assert_eq!(Platform::of_uname(answer), None, "{answer}");
        }
    }

    #[test]
    fn the_machine_is_read_from_what_the_script_said() {
        let said = vec![
            "Warning: Permanently added 'dev' to the list of known hosts.".to_owned(),
            format!("{MISSING} (Linux aarch64)"),
        ];
        let platform = Platform::said_missing(&said).unwrap();
        assert_eq!(platform.arch, "aarch64");
        assert_eq!(
            Platform::said_missing(&["bash: hof: not found".into()]),
            None
        );
    }

    #[test]
    fn the_agent_is_found_in_the_first_directory_that_has_it() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let platform = Platform::of_uname("Linux x86_64").unwrap();
        let dirs = [a.path().to_owned(), b.path().to_owned()];
        assert_eq!(find_agent(&dirs, &platform), None);
        std::fs::write(b.path().join("hof-agent-x86_64-linux"), b"agent").unwrap();
        assert_eq!(
            find_agent(&dirs, &platform),
            Some(b.path().join("hof-agent-x86_64-linux"))
        );
    }

    /// The scripts, run by a local `sh` in place of ssh, with `HOME` a
    /// temporary directory.
    #[test]
    fn a_machine_without_hof_gets_the_copy_and_runs_it() {
        let home = tempfile::tempdir().unwrap();
        let ssh = Ssh {
            argv: vec![
                "env".into(),
                format!("HOME={}", home.path().display()),
                "XDG_DATA_HOME=".into(),
                "PATH=/usr/bin:/bin".into(),
                "sh".into(),
                "-c".into(),
                // What ssh does with what follows `--`: hands it to the
                // login shell as one line.
                "eval \"$1\"".into(),
                "sh".into(),
            ],
            hof: "hof-that-is-not-there".into(),
        };
        let unwrap = |argv: Vec<String>| argv.into_iter().filter(|a| a != "--").collect::<Vec<_>>();

        let missing = run(&unwrap(ssh.agent_command().unwrap()), None, ASK_WAIT);
        let said = missing.unwrap_err();
        assert!(said.starts_with(MISSING), "{said}");

        // A stand-in for the agent: a script that answers `--version` and
        // says what it was asked to do.
        let agent = b"#!/bin/sh\necho \"agent $*\"\n".to_vec();
        run(&unwrap(ssh.copy_command()), Some(agent.clone()), COPY_WAIT).unwrap();
        let ran = run(&unwrap(ssh.agent_command().unwrap()), None, ASK_WAIT).unwrap();
        assert_eq!(ran, "agent agent --stdio");
        let copied = home
            .path()
            .join(".local/share/hof")
            .join(version_dir())
            .join(NAME);
        assert!(copied.is_file());
        let link = home.path().join(".local/bin").join(NAME);
        assert_eq!(std::fs::read_link(&link).unwrap(), copied);

        // A link to an older copy moves on; a Hof of the host's own stays.
        let older = home.path().join(".local/share/hof/0.0.1-0/hof");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&older, &link).unwrap();
        run(&unwrap(ssh.copy_command()), Some(agent.clone()), COPY_WAIT).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), copied);
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, b"its own").unwrap();
        run(&unwrap(ssh.copy_command()), Some(agent), COPY_WAIT).unwrap();
        assert_eq!(std::fs::read(&link).unwrap(), b"its own");

        // A copy that can't run there leaves the one before in place.
        run(
            &unwrap(ssh.copy_command()),
            Some(b"\x7fELF not this".to_vec()),
            COPY_WAIT,
        )
        .unwrap_err();
        let ran = run(&unwrap(ssh.agent_command().unwrap()), None, ASK_WAIT).unwrap();
        assert_eq!(ran, "agent agent --stdio");
    }
}
