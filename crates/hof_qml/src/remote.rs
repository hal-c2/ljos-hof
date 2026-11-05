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
    /// Its files are mounted here as soon as it is reached, not only when
    /// a folder there is opened.
    pub files: bool,
}

impl Reach {
    fn options(&self) -> client::Options {
        let mut services: Vec<Arc<dyn client::Service>> = Vec::new();
        // Looked for when the host is first reached: yserver where it is
        // installed (on a Mac, or named by `$HOF_YSERVER`), started when a
        // program first opens a window, so each X window is a window of
        // its own here; else the X server `$DISPLAY` names. With neither,
        // each program is told what to install.
        if self.x11 {
            if let Some(y) = yserver() {
                services.push(y);
            } else if let Some(x) = hof_remote::x11::Forward::from_env() {
                services.push(Arc::new(x));
            } else {
                services.push(Arc::new(hof_remote::x11::Missing::here()));
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
    /// Copies of its files, made when a program there first opens one.
    mirror: Option<Arc<hof_remote::files::Mirror>>,
    /// Its files, mounted here when a folder there is first opened.
    mount: Option<Arc<hof_remote::files::Mount>>,
}

static HOSTS: Mutex<BTreeMap<String, Known>> = Mutex::new(BTreeMap::new());

/// Whether a terminal going away leaves its program running on its
/// machine, to attach to again, rather than ending it.
static KEEP: AtomicBool = AtomicBool::new(false);

/// How to reach the agent that runs this machine's own panes, for them
/// to outlive Hof: its socket, and how to start it when none answers.
#[derive(Clone)]
pub struct Here {
    pub socket: std::path::PathBuf,
    pub start: Arc<dyn Fn() -> std::io::Result<()> + Send + Sync>,
}

impl std::fmt::Debug for Here {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Here")
            .field("socket", &self.socket)
            .finish()
    }
}

/// The agent for this machine's panes, if they are to outlive Hof, and
/// the connection to it once a pane wanted one. Apart from [`HOSTS`]:
/// nothing that takes a host's name (copies, mounts) can reach it.
static HERE: Mutex<Option<(Here, Option<Arc<Host>>)>> = Mutex::new(None);

/// Runs this machine's panes opened from now on under the agent `here`
/// says, so that their programs outlive Hof, to be attached again at the
/// next start; `None` runs them under Hof itself. A pane already open
/// stays where it is.
pub fn keep_here(here: Option<Here>) {
    let mut kept = HERE.lock().unwrap_or_else(PoisonError::into_inner);
    *kept = match (kept.take(), here) {
        (Some((was, host)), Some(here)) if was.socket == here.socket => Some((here, host)),
        (_, here) => here.map(|here| (here, None)),
    };
}

/// The agent for this machine's panes, connecting the first time; `None`
/// when they run under Hof itself.
pub(crate) fn here() -> Option<Arc<Host>> {
    let mut kept = HERE.lock().unwrap_or_else(PoisonError::into_inner);
    let (here, host) = kept.as_mut()?;
    let host = host.get_or_insert_with(|| {
        let start = here.start.clone();
        Arc::new(Host::new(
            client::Local {
                socket: here.socket.clone(),
                start: Box::new(move || start()),
            },
            // Nothing to pass on: the programs are here already.
            client::Options::default(),
        ))
    });
    Some(host.clone())
}

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
                mirror: k.mirror.clone(),
                mount: k.mount.clone(),
            };
            (name.clone(), k)
        })
        .collect();
    for (name, transport) in hosts {
        let same = known
            .get(&name)
            .filter(|k| k.transport.as_ref() == Some(&transport));
        let host = same.and_then(|k| k.host.clone());
        let mirror = same.and_then(|k| k.mirror.clone());
        let mount = same.and_then(|k| k.mount.clone());
        new.insert(
            name,
            Known {
                transport: Some(transport),
                host,
                mirror,
                mount,
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
            mirror: None,
            mount: None,
        },
    );
}

/// Where copies of other machines' files go, if not the user's cache.
static CACHE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

/// Puts copies of other machines' files under `dir` rather than the
/// user's cache, as a test does.
pub fn set_cache_dir(dir: impl Into<std::path::PathBuf>) {
    *CACHE.lock().unwrap_or_else(PoisonError::into_inner) = Some(dir.into());
}

/// Where other machines' files are mounted, if not the user's runtime
/// directory, and the folder on each that is mounted, if not all of it.
static MOUNTS: Mutex<Option<(std::path::PathBuf, Vec<u8>)>> = Mutex::new(None);

/// Mounts only the folder `root` of each machine, under `dir` rather
/// than the user's runtime directory, as a test does: what is done in
/// the mount, removing it all included, reaches nothing else there.
pub fn set_mounts(dir: impl Into<std::path::PathBuf>, root: impl Into<Vec<u8>>) {
    *MOUNTS.lock().unwrap_or_else(PoisonError::into_inner) = Some((dir.into(), root.into()));
}

/// `name`'s files, mounted here: the first time, only for a host that is
/// connected, and only where this system can (Linux, with FUSE).
pub(crate) fn mount(name: &str) -> Option<Arc<hof_remote::files::Mount>> {
    // One mounting at a time: two would mount the same host twice.
    static MOUNTING: Mutex<()> = Mutex::new(());
    let _mounting = MOUNTING.lock().unwrap_or_else(PoisonError::into_inner);
    let set = MOUNTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let (dir, root) = match set {
        Some(set) => set,
        None => (hof_remote::files::runtime_dir()?, b"/".to_vec()),
    };
    let host = {
        let known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
        let k = known.get(name)?;
        if let Some(mount) = &k.mount {
            return Some(mount.clone());
        }
        k.host.clone()?
    };
    // Not with the hosts locked: what is there may be slow to answer.
    let at = hof_remote::files::host_dir(&dir, name);
    let reach = host.clone();
    let mounted = hof_remote::files::Mount::of(at, name, &root, move || {
        reach.connect(hof_remote::files::SERVICE, Vec::new())
    });
    let mount = match mounted {
        Ok(mount) => Arc::new(mount),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::Unsupported {
                eprintln!("hof: could not mount the files on {name}: {e}");
            }
            return None;
        }
    };
    let mut known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
    // Kept only if the host is still the one it was mounted for.
    let k = known
        .get_mut(name)
        .filter(|k| k.host.as_ref().is_some_and(|h| Arc::ptr_eq(h, &host)))?;
    k.mount = Some(mount.clone());
    Some(mount)
}

/// The copies here of `name`'s files, kept the same as them: made the
/// first time, and only for a host that is connected.
pub(crate) fn mirror(name: &str) -> Option<Arc<hof_remote::files::Mirror>> {
    let cache = CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .or_else(hof_remote::files::cache_dir)?;
    let mut known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
    let k = known.get_mut(name)?;
    let host = k.host.clone()?;
    let mirror = k.mirror.get_or_insert_with(|| {
        let dir = hof_remote::files::host_dir(&cache, name);
        hof_remote::files::Mirror::new(dir, name, move || {
            host.connect(hof_remote::files::SERVICE, Vec::new())
        })
    });
    Some(mirror.clone())
}

/// Whether terminals closed from now on leave their programs running on
/// their machines: on when Hof quits with a session to restore, so the
/// panes are attached again at the next start.
pub fn keep_programs(keep: bool) {
    KEEP.store(keep, Ordering::Release);
}

/// Ends the programs of panes no window shows, as `hof forget` does for a
/// session: `(host, pane)` each, the host a `[hosts]` name, or "" for the
/// agent here. Each is attached first, so the close is sent only to an
/// agent that has it, and no window is the worse for one it can't reach.
/// Waits for the agents up to `within`, all together; returns how many
/// programs were ended. The connections are hung up after, as [`finish`]
/// does.
pub fn end_unseen(panes: &[(String, u32)], within: std::time::Duration) -> usize {
    let deadline = std::time::Instant::now() + within;
    let attached: Vec<_> = panes
        .iter()
        .filter_map(|(name, pane)| {
            let host = if name.is_empty() { here() } else { host(name) }?;
            let (told, answer) = std::sync::mpsc::channel();
            let pane = host.attach(*pane, 80, 24, Arc::new(Unseen(Mutex::new(told))));
            Some((pane, answer))
        })
        .collect();
    let mut ended = 0;
    for (pane, answer) in attached {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if answer.recv_timeout(left) == Ok(true) {
            pane.close();
            ended += 1;
        } else {
            pane.detach();
        }
    }
    finish(within);
    ended
}

/// A pane attached only to be ended: says whether its agent has it.
struct Unseen(Mutex<std::sync::mpsc::Sender<bool>>);

impl Unseen {
    fn tell(&self, has: bool) {
        let _ = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(has);
    }
}

impl Screen for Unseen {
    fn output(&self, _: &[u8]) {}
    fn replace(&self, _: &[u8]) {}
    fn exited(&self, _: Option<i32>) {
        self.tell(false);
    }
    fn state(&self, state: State) {
        match state {
            State::Connecting => {}
            State::Up { .. } => self.tell(true),
            State::Down(_) | State::Gone(_) => self.tell(false),
        }
    }
}

/// Hof is going: every remote pane's program is ended, or left running
/// when [`keep_programs`] says so, and the connections hung up. Waits for
/// that up to `within`, for all the hosts together.
pub fn finish(within: std::time::Duration) {
    let keep = KEEP.load(Ordering::Acquire);
    let hosts: Vec<Arc<Host>> = {
        let known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
        let mut hosts: Vec<_> = known.values().filter_map(|k| k.host.clone()).collect();
        let here = HERE.lock().unwrap_or_else(PoisonError::into_inner);
        hosts.extend(here.as_ref().and_then(|(_, host)| host.clone()));
        hosts
    };
    let waits: Vec<_> = hosts
        .into_iter()
        .map(|host| std::thread::spawn(move || host.finish(keep, within)))
        .collect();
    for wait in waits {
        let _ = wait.join();
    }
    // Unmounted, though a program here may still be in one: it keeps
    // what it has open.
    let mounts: Vec<_> = {
        let mut known = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
        known.values_mut().filter_map(|k| k.mount.take()).collect()
    };
    drop(mounts);
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
        k.mirror = None;
        k.mount = None;
        if reach.files {
            let name = name.to_owned();
            let _ = std::thread::Builder::new()
                .name("hof-mount".into())
                .spawn(move || mount(&name));
        }
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
