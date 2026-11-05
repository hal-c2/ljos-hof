//! Terminals whose programs run on other machines, under `hof agent`
//! there: the hosts they can reach, and the glue between a
//! [`hof_vt::Session`] and a [`hof_remote::client::Pane`].
//!
//! A host's connection is made the first time a pane needs it, and shared
//! by every pane on that host, in every tab and split.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hof_remote::client::{self, Host, Pane, Screen, Spawned, Start, State};
use hof_vt::{Feed, Link, Outlet, Size};

/// How to reach a host from the config, and what its programs may reach
/// here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reach {
    /// The command whose stdin and stdout reach `hof agent --stdio` there.
    pub transport: Vec<String>,
    /// Reached over ssh, and given Hof's agent when it has no Hof: then
    /// this, not `transport`.
    pub bootstrap: Option<hof_remote::bootstrap::Ssh>,
    /// Programs there may open windows on the X server here.
    pub x11: bool,
    /// Programs there may play sound here.
    pub sound: bool,
}

impl Reach {
    fn options(&self) -> client::Options {
        let mut services: Vec<Arc<dyn client::Service>> = Vec::new();
        // Looked for when the host is first reached: yserver where it is
        // installed (on a Mac, or named by `$HOF_YSERVER`), started when a
        // program first opens a window, so each X window is a window of
        // its own here; else the X server `$DISPLAY` names. With neither,
        // none.
        if self.x11 {
            if let Some(y) = yserver() {
                services.push(y);
            } else if let Some(x) = hof_remote::x11::Forward::from_env() {
                services.push(Arc::new(x));
            }
        }
        if self.sound
            && let Some(sound) = sound()
        {
            services.push(sound);
        }
        client::Options {
            services,
            ..client::Options::default()
        }
    }
}

/// The sound server running here (PulseAudio or PipeWire's), else Hof's
/// own, playing through the speakers itself where it can (on a Mac): one
/// for every host.
fn sound() -> Option<Arc<dyn client::Service>> {
    if let Some(running) = hof_remote::pulse::Forward::from_env().filter(|f| f.is_there()) {
        return Some(Arc::new(running));
    }
    static ONE: std::sync::OnceLock<Option<Arc<hof_remote::sound::Sound>>> =
        std::sync::OnceLock::new();
    let one = ONE.get_or_init(|| hof_remote::sound::speakers().map(Arc::new));
    one.clone().map(|s| s as Arc<dyn client::Service>)
}

/// yserver, if it is installed: one for every host, so their windows
/// share one server.
fn yserver() -> Option<Arc<dyn client::Service>> {
    static ONE: std::sync::OnceLock<Option<Arc<hof_remote::yserver::Yserver>>> =
        std::sync::OnceLock::new();
    let one = ONE.get_or_init(|| hof_remote::yserver::Yserver::find().map(Arc::new));
    one.clone().map(|y| y as Arc<dyn client::Service>)
}

/// A name a pane's `host` can take.
struct Known {
    /// How it is reached, for a host from the config; `None` for one given
    /// whole to [`set_host`].
    transport: Option<Reach>,
    /// Made when a pane first wants it.
    host: Option<Arc<Host>>,
}

static HOSTS: Mutex<BTreeMap<String, Known>> = Mutex::new(BTreeMap::new());

/// Whether a terminal going away leaves its program running on its
/// machine, to attach to again, rather than ending it.
static KEEP: AtomicBool = AtomicBool::new(false);

/// The hosts of the config: each name, and how it is reached. A host
/// reached the same way as before keeps its connection; panes already on
/// a host that changed or went keep theirs until they close.
pub fn set_hosts(hosts: impl IntoIterator<Item = (String, Reach)>) {
    let mut known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut new: BTreeMap<String, Known> = known
        .iter()
        .filter(|(_, k)| k.transport.is_none())
        .map(|(name, k)| {
            let k = Known {
                transport: None,
                host: k.host.clone(),
            };
            (name.clone(), k)
        })
        .collect();
    for (name, transport) in hosts {
        let host = known
            .get(&name)
            .filter(|k| k.transport.as_ref() == Some(&transport))
            .and_then(|k| k.host.clone());
        new.insert(
            name,
            Known {
                transport: Some(transport),
                host,
            },
        );
    }
    *known = new;
}

/// A host reached some way the config can't say, such as a test's.
pub fn set_host(name: &str, host: Host) {
    let mut known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
    known.insert(
        name.to_owned(),
        Known {
            transport: None,
            host: Some(Arc::new(host)),
        },
    );
}

/// Whether terminals closed from now on leave their programs running on
/// their machines: on when Hof quits with a session to restore, so the
/// panes are attached again at the next start.
pub fn keep_programs(keep: bool) {
    KEEP.store(keep, Ordering::Release);
}

/// Hof is going: every remote pane's program is ended, or left running
/// when [`keep_programs`] says so, and the connections hung up. Waits for
/// that up to `within`, for all the hosts together.
pub fn finish(within: std::time::Duration) {
    let keep = KEEP.load(Ordering::Acquire);
    let hosts: Vec<Arc<Host>> = {
        let known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
        known.values().filter_map(|k| k.host.clone()).collect()
    };
    let waits: Vec<_> = hosts
        .into_iter()
        .map(|host| std::thread::spawn(move || host.finish(keep, within)))
        .collect();
    for wait in waits {
        let _ = wait.join();
    }
}

/// The host called `name`, connecting the first time; `None` for a name no
/// one gave.
pub(crate) fn host(name: &str) -> Option<Arc<Host>> {
    let mut known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
    let k = known.get_mut(name)?;
    if k.host.is_none() {
        let reach = k.transport.clone()?;
        let options = reach.options();
        let host = match reach.bootstrap {
            Some(ssh) => Host::new(
                hof_remote::bootstrap::Bootstrapping::new(ssh, hof_remote::bootstrap::agent_dirs()),
                options,
            ),
            None => Host::new(
                Spawned {
                    argv: reach.transport,
                },
                options,
            ),
        };
        k.host = Some(Arc::new(host));
    }
    k.host.clone()
}

/// A session's screen, as the connection writes to it.
struct Shown(Feed);

impl Screen for Shown {
    fn output(&self, bytes: &[u8]) {
        self.0.output(bytes);
    }

    fn replace(&self, snapshot: &[u8]) {
        self.0.replace(snapshot);
    }

    fn exited(&self, code: Option<i32>) {
        self.0.exited(code);
    }

    fn foreground(&self, process: Option<hof_remote::Process>) {
        self.0.foreground(process.map(|p| hof_vt::Process {
            pid: p.pid,
            name: p.name,
            args: p.args,
        }));
    }

    fn state(&self, state: State) {
        self.0.link(match state {
            State::Connecting => Link::Connecting,
            State::Up { pane } => Link::Up { pane },
            State::Down(why) => Link::Down(why),
            State::Gone(why) => Link::Gone(why),
        });
    }
}

/// A session's way to its program: a pane on its host. The pane can be
/// replaced, with the screen kept: a restored pane whose program its
/// machine forgot starts a new one under what it showed.
pub(crate) struct Remote {
    host: Arc<Host>,
    screen: Arc<Shown>,
    pane: Mutex<Pane>,
}

/// How a remote pane starts: a new program, or one that runs already.
pub(crate) enum Begin {
    Open(Start),
    Attach { pane: u32, cols: u16, rows: u16 },
}

impl Remote {
    pub fn new(host: Arc<Host>, feed: Feed, begin: Begin) -> Arc<Self> {
        let screen = Arc::new(Shown(feed));
        let pane = Self::begin(&host, &screen, begin);
        Arc::new(Self {
            host,
            screen,
            pane: Mutex::new(pane),
        })
    }

    fn begin(host: &Host, screen: &Arc<Shown>, begin: Begin) -> Pane {
        let screen: Arc<dyn Screen> = screen.clone();
        match begin {
            Begin::Open(start) => host.open(start, screen),
            Begin::Attach { pane, cols, rows } => host.attach(pane, cols, rows, screen),
        }
    }

    /// Starts `start` in place of the pane there was, which is gone.
    pub fn reopen(&self, start: Start) {
        let pane = Self::begin(&self.host, &self.screen, Begin::Open(start));
        *self.pane() = pane;
    }

    fn pane(&self) -> std::sync::MutexGuard<'_, Pane> {
        self.pane.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the session hands its outlet: the same [`Remote`], shared with the
/// terminal, which may reopen it.
pub(crate) struct Out(pub Arc<Remote>);

impl Outlet for Out {
    fn input(&self, bytes: &[u8]) {
        self.0.pane().input(bytes);
    }

    fn resize(&self, size: Size) {
        self.0.pane().resize(size.cols, size.rows);
    }

    fn closed(&self) {
        let pane = self.0.pane();
        if KEEP.load(Ordering::Acquire) {
            pane.detach();
        } else {
            pane.close();
        }
    }
}
