//! A session kept on another machine: `hof attach --host desktop work`
//! opens `work` as it was left on `desktop`, its programs still running
//! under the agent there, and keeps it there as it changes, so that `hof
//! attach work` back on `desktop` opens it as it was left here.
//!
//! The agent there holds the session for this window, as a window there
//! would, for as long as the channel to it lasts. Each pane's host is as
//! the machine it is saved on names it: what runs on `desktop` itself is
//! its own (no host) there and `desktop` here, and what runs here is this
//! machine's name there.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::Context as _;
use hof_remote::client::{Host, Kept};
use hof_session::Session;
use hof_session::service::{self, Holding};

/// How long reaching the machine may take: the first ssh, and copying an
/// agent there.
const REACH_WAIT: Duration = Duration::from_secs(30);

/// How long the agent there may take to answer, once reached.
const ANSWER_WAIT: Duration = Duration::from_secs(30);

/// How long Hof going waits for the session to be kept there.
const LAST_SAVE_WAIT: Duration = Duration::from_secs(10);

/// The session this window keeps on another machine, if it does.
static AWAY: OnceLock<Away> = OnceLock::new();

/// The session this window keeps on another machine, if it does.
pub(crate) fn get() -> Option<&'static Away> {
    AWAY.get()
}

/// A session on another machine, as this window keeps it.
pub struct Away {
    /// It as it was held, the panes' hosts as named here; for the window
    /// to open, once.
    held: Mutex<Option<Session>>,
    /// For the thread that keeps it there.
    jobs: Mutex<mpsc::Sender<Job>>,
    host: String,
    ours: String,
}

enum Job {
    /// Kept there; told how it went, if anyone waits.
    Save(Session, Option<mpsc::Sender<bool>>),
    Forget(mpsc::Sender<bool>),
}

/// Holds the session `name` on the `[hosts]` host `host` for this window,
/// or with `None` the one saved last there that no Hof has open. Returns
/// its name.
pub fn open(host: &str, name: Option<&str>, config: &hof_config::Config) -> anyhow::Result<String> {
    open_with(host, config, |c| service::hold(c, name))
}

/// Holds a plain `hof`'s session on the `[hosts]` host `host` for this
/// window (`[session] host`): `first`, or when another Hof has it open the
/// first of `default`, `2`, `3` and on there that none has. Returns its
/// name.
pub fn open_free(host: &str, first: &str, config: &hof_config::Config) -> anyhow::Result<String> {
    open_with(host, config, |c| service::hold_free(c, first))
}

fn open_with(
    host: &str,
    config: &hof_config::Config,
    hold: impl FnOnce(UnixStream) -> io::Result<(Holding<UnixStream>, Option<Session>)>,
) -> anyhow::Result<String> {
    let (there, kept) = reach(host, config)?;
    let (holding, session) = hold(channel(&there, host)?).with_context(|| format!("on {host}"))?;
    let ours = this_machine();
    let name = holding.name.clone();
    let (jobs, taken) = mpsc::channel();
    let away = Away {
        held: Mutex::new(session.map(|mut s| {
            as_here(&mut s, host, &ours);
            s
        })),
        jobs: Mutex::new(jobs),
        host: host.to_owned(),
        ours,
    };
    let keeping = Keeping {
        holding,
        there,
        _kept: kept,
        host: host.to_owned(),
    };
    thread::Builder::new()
        .name("hof-away".into())
        .spawn(move || keeping.run(&taken))?;
    if AWAY.set(away).is_err() {
        anyhow::bail!("this Hof keeps a session elsewhere already");
    }
    Ok(name)
}

/// `hof sessions --host`: the sessions saved on `host`.
pub fn list(host: &str, config: &hof_config::Config) -> anyhow::Result<Vec<hof_session::Listed>> {
    let (there, _kept) = reach(host, config)?;
    service::list(channel(&there, host)?).with_context(|| format!("on {host}"))
}

/// `hof forget --host`: ends the programs the session `name` on `host`
/// kept running, wherever they run, and forgets it there. Returns how many
/// programs it ended.
pub fn forget(
    host: &str,
    name: &str,
    config: &hof_config::Config,
    agent: &std::path::Path,
) -> anyhow::Result<usize> {
    let (there, _kept) = reach(host, config)?;
    let (mut holding, session) = service::hold(channel(&there, host)?, Some(name))
        .with_context(|| format!("on {host}: close its panes there"))?;
    let mut session = session.with_context(|| format!("no session {name} is saved on {host}"))?;
    as_here(&mut session, host, &this_machine());
    let ended = crate::held::end_programs(&session, agent);
    holding.forget().with_context(|| format!("on {host}"))?;
    Ok(ended)
}

impl Away {
    /// The `[hosts]` host it is kept on.
    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    /// The session as it was held, for the window to open; `None` after
    /// the first time, or when it was not saved there yet.
    pub(crate) fn take(&self) -> Option<Session> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Keeps `session` there. As Hof runs, it is sent and not waited for:
    /// what is not sent yet gives way to a newer one. As it goes
    /// (`quitting`), it waits a while to hear it was kept, and says so.
    pub(crate) fn save(&self, mut session: Session, quitting: bool) -> bool {
        as_there(&mut session, &self.host, &self.ours);
        if !quitting {
            return self.send(Job::Save(session, None));
        }
        let (done, heard) = mpsc::channel();
        self.send(Job::Save(session, Some(done)))
            && heard.recv_timeout(LAST_SAVE_WAIT).unwrap_or(false)
    }

    /// Forgets the session there: its every pane was closed.
    pub(crate) fn forget(&self) -> bool {
        let (done, heard) = mpsc::channel();
        self.send(Job::Forget(done)) && heard.recv_timeout(LAST_SAVE_WAIT).unwrap_or(false)
    }

    fn send(&self, job: Job) -> bool {
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(job)
            .is_ok()
    }
}

/// The thread that keeps the session there, and holds it.
struct Keeping {
    holding: Holding<UnixStream>,
    there: Arc<Host>,
    /// The connection, kept up while the session is held on it.
    _kept: Kept,
    host: String,
}

impl Keeping {
    fn run(mut self, jobs: &mpsc::Receiver<Job>) {
        while let Ok(mut job) = jobs.recv() {
            // A save no one waits for gives way to a newer one.
            while let Job::Save(_, None) = job {
                match jobs.try_recv() {
                    Ok(next) => job = next,
                    Err(_) => break,
                }
            }
            let (done, kept) = match job {
                Job::Save(session, done) => {
                    let kept = self.again(|h| h.save(&session));
                    (done, kept)
                }
                Job::Forget(done) => (Some(done), self.again(Holding::forget)),
            };
            if let Err(e) = &kept {
                eprintln!("hof: the session was not kept on {}: {e}", self.host);
            }
            if let Some(done) = done {
                let _ = done.send(kept.is_ok());
            }
        }
    }

    /// `what` on the session held there, once more on a new channel when
    /// the one it was held on ended: a dropped connection ends it, and the
    /// agent there lets the session go.
    fn again(
        &mut self,
        mut what: impl FnMut(&mut Holding<UnixStream>) -> io::Result<()>,
    ) -> io::Result<()> {
        match what(&mut self.holding) {
            // The agent there said no: asking again would not change it.
            Err(e) if e.kind() == io::ErrorKind::Other => Err(e),
            Err(_) => {
                self.hold_again()?;
                what(&mut self.holding)
            }
            Ok(()) => Ok(()),
        }
    }

    fn hold_again(&mut self) -> io::Result<()> {
        let name = self.holding.name.clone();
        let mut last = None;
        // The agent there lets the old channel's hold go once it sees the
        // channel end, which may be a moment after this one is made.
        for _ in 0..5 {
            let held = channel(&self.there, &self.host)
                .map_err(io::Error::other)
                .and_then(|c| service::hold(c, Some(&name)));
            match held {
                Ok((holding, _)) => {
                    self.holding = holding;
                    return Ok(());
                }
                Err(e) => last = Some(e),
            }
            thread::sleep(Duration::from_secs(1));
        }
        Err(last.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }
}

/// The `[hosts]` host `host`, and its connection made and kept up.
fn reach(host: &str, config: &hof_config::Config) -> anyhow::Result<(Arc<Host>, Kept)> {
    hof_qml::remote::set_hosts(crate::settings::transports(config));
    let there = hof_qml::remote::host(host)
        .with_context(|| format!("no host {host} in [hosts] of the config"))?;
    let kept = there.keep_connected();
    Ok((there, kept))
}

/// A new sessions channel to `there`.
fn channel(there: &Host, host: &str) -> anyhow::Result<UnixStream> {
    let socket = there
        .connect_within(service::SERVICE, Vec::new(), REACH_WAIT)
        .with_context(|| format!("could not reach {host}"))?;
    socket.set_read_timeout(Some(ANSWER_WAIT))?;
    socket.set_write_timeout(Some(ANSWER_WAIT))?;
    Ok(socket)
}

/// Whether the `[hosts]` host `host` is this machine, by its name: a
/// config shared by both machines, its `[session] host` the one of them.
pub fn is_this_machine(host: &str) -> bool {
    host == this_machine()
}

/// This machine's name, as the agent here says it: what a pane here is
/// saved as running on, there.
fn this_machine() -> String {
    let name = rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .into_owned();
    match name.split_once('.') {
        Some((short, _)) if !short.is_empty() => short.to_owned(),
        _ => name,
    }
}

/// The panes of `session`, saved on `host`, with hosts as named here:
/// `host`'s own are `host`'s, and this machine's (`ours`) are this one's.
fn as_here(session: &mut Session, host: &str, ours: &str) {
    for pane in session
        .windows
        .iter_mut()
        .flat_map(|w| &mut w.tabs)
        .flat_map(|t| t.panes_mut())
    {
        pane.host = match pane.host.take() {
            None => Some(host.to_owned()),
            Some(h) if h == ours => None,
            other => other,
        };
    }
}

/// [`as_here`] undone, for `session` to be saved on `host`.
fn as_there(session: &mut Session, host: &str, ours: &str) {
    for pane in session
        .windows
        .iter_mut()
        .flat_map(|w| &mut w.tabs)
        .flat_map(|t| t.panes_mut())
    {
        pane.host = match pane.host.take() {
            None => Some(ours.to_owned()),
            Some(h) if h == host => None,
            other => other,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hof_session::{SavedAxis, SavedNode, SavedPane, SavedTab, SavedWindow};

    fn panes(hosts: [Option<&str>; 3]) -> Session {
        let pane = |id, host: Option<&str>| {
            Box::new(SavedNode::Pane(SavedPane {
                id,
                host: host.map(str::to_owned),
                remote: Some(1000 + id),
                ..SavedPane::default()
            }))
        };
        Session::new(vec![SavedWindow {
            tabs: vec![SavedTab {
                root: SavedNode::Split {
                    axis: SavedAxis::Horizontal,
                    ratio: 0.5,
                    first: pane(1, hosts[0]),
                    second: Box::new(SavedNode::Split {
                        axis: SavedAxis::Vertical,
                        ratio: 0.5,
                        first: pane(2, hosts[1]),
                        second: pane(3, hosts[2]),
                    }),
                },
                focused: 1,
                zoomed: false,
            }],
            current_tab: 0,
            width: 0,
            height: 0,
            dropdown: false,
        }])
    }

    #[test]
    fn each_pane_runs_where_it_did_as_this_machine_names_it() {
        // On desktop: its own pane, one on laptop, one on dev.
        let mut session = panes([None, Some("laptop"), Some("dev")]);
        as_here(&mut session, "desktop", "laptop");
        assert_eq!(session, panes([Some("desktop"), None, Some("dev")]));
        as_there(&mut session, "desktop", "laptop");
        assert_eq!(session, panes([None, Some("laptop"), Some("dev")]));
    }

    #[test]
    fn a_config_shared_with_the_machine_it_names_keeps_its_session_there() {
        assert!(is_this_machine(&this_machine()));
        assert!(!this_machine().contains('.'), "the short name");
        assert!(!is_this_machine("not-this-machine.invalid"));
    }
}
