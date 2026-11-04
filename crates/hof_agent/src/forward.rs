//! What the agent passes on to Hof: X11 and sound clients on this
//! machine, each connection a channel of a Hof connection that offered
//! to take them.
//!
//! The listeners start when a connection first offers their service, and
//! stay. A program that connects goes to the newest connection still
//! offering: the Hof the user is at. With none, an X11 client is told why,
//! and a sound client is hung up on.
//!
//! The panes a connection opens are given what finds the listeners:
//! `DISPLAY` and `XAUTHORITY`, `PULSE_SERVER`. What they hold stays right
//! from one connection to the next, so a pane outlives the Hof that
//! opened it and its programs still find whichever Hof comes next.

use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread;
use std::time::Duration;

use hof_remote::wire::ChannelId;
use hof_remote::{Relays, Socket, pulse, x11};

/// How long a program has to say how it opens.
const OPENING_WAIT: Duration = Duration::from_secs(10);

/// The first display number tried, as `ssh -X` does: past the ones the
/// machine's own servers take.
const FIRST_DISPLAY: u32 = 10;

pub(crate) struct Forwards {
    /// Where the authorization file and the sound socket go: a directory
    /// only the user can reach, made when first needed.
    dir: PathBuf,
    offers: Mutex<Vec<Offer>>,
    x11: Mutex<Option<X11>>,
    sound: Mutex<Option<PathBuf>>,
    next_channel: AtomicU32,
}

struct Offer {
    connection: u64,
    services: Vec<String>,
    relays: Relays,
}

#[derive(Clone)]
struct X11 {
    display: String,
    xauthority: PathBuf,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Forwards {
    pub fn new(instance: u64) -> Self {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|d| d.is_absolute())
            .unwrap_or_else(std::env::temp_dir);
        Self {
            dir: base.join(format!("hof-agent-{instance:x}")),
            offers: Mutex::default(),
            x11: Mutex::default(),
            sound: Mutex::default(),
            next_channel: AtomicU32::new(1),
        }
    }

    /// `connection` takes connections for `services`, over `relays`.
    pub fn offer(self: &Arc<Self>, connection: u64, services: Vec<String>, relays: Relays) {
        let services: Vec<String> = services
            .into_iter()
            // Those it knows, and can listen for.
            .filter(|s| match s.as_str() {
                x11::SERVICE => self.x11().is_some(),
                pulse::SERVICE => self.sound().is_some(),
                _ => false,
            })
            .collect();
        let mut offers = lock(&self.offers);
        offers.retain(|o| o.connection != connection);
        offers.push(Offer {
            connection,
            services,
            relays,
        });
    }

    /// `connection` went.
    pub fn withdraw(&self, connection: u64) {
        lock(&self.offers).retain(|o| o.connection != connection);
    }

    /// What the panes `connection` opens are given to find its services.
    pub fn environment(&self, connection: u64) -> Vec<(String, String)> {
        let offers = lock(&self.offers);
        let Some(offer) = offers.iter().find(|o| o.connection == connection) else {
            return Vec::new();
        };
        let mut env = Vec::new();
        for service in &offer.services {
            match service.as_str() {
                x11::SERVICE => {
                    if let Some(x) = lock(&self.x11).clone() {
                        env.push(("DISPLAY".into(), x.display));
                        let file = x.xauthority.to_string_lossy().into_owned();
                        env.push(("XAUTHORITY".into(), file));
                    }
                }
                pulse::SERVICE => {
                    if let Some(socket) = lock(&self.sound).as_ref() {
                        let server = format!("unix:{}", socket.display());
                        env.push(("PULSE_SERVER".into(), server));
                    }
                }
                _ => {}
            }
        }
        env
    }

    /// The newest connection that offers `service`.
    fn route(&self, service: &str) -> Option<Relays> {
        lock(&self.offers)
            .iter()
            .rev()
            .find(|o| o.services.iter().any(|s| s == service))
            .map(|o| o.relays.clone())
    }

    fn channel(&self) -> ChannelId {
        self.next_channel.fetch_add(1, Ordering::Relaxed)
    }

    /// The directory, made if it is not there yet.
    fn dir(&self) -> io::Result<&Path> {
        private_dir(&self.dir).map(|()| self.dir.as_path())
    }

    /// Takes the files away, at the agent's end.
    pub fn remove_files(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// The display, listening from now on.
    fn x11(self: &Arc<Self>) -> Option<X11> {
        let mut x11 = lock(&self.x11);
        if x11.is_none() {
            *x11 = self.start_x11().ok();
        }
        x11.clone()
    }

    fn start_x11(self: &Arc<Self>) -> io::Result<X11> {
        let (listener, number) = bind_display()?;
        let cookie = random_cookie()?;
        let xauthority = self.dir()?.join("Xauthority");
        let mut file = Vec::new();
        x11::Auth {
            family: x11::FAMILY_WILD,
            address: Vec::new(),
            number: number.to_string().into_bytes(),
            name: x11::COOKIE.to_vec(),
            data: cookie.to_vec(),
        }
        .encode(&mut file);
        write_private(&xauthority, &file)?;
        let me = Arc::downgrade(self);
        // Not joined: it waits in `accept` while the agent runs.
        thread::Builder::new()
            .name("hof-agent-x11".into())
            .spawn(move || {
                accept(&listener, |socket| {
                    let me = me.clone();
                    thread::spawn(move || take_x11(&me, socket, &cookie));
                });
            })?;
        Ok(X11 {
            display: display_name(number),
            xauthority,
        })
    }

    /// The sound socket, listening from now on.
    fn sound(self: &Arc<Self>) -> Option<PathBuf> {
        let mut sound = lock(&self.sound);
        if sound.is_none() {
            *sound = self.start_sound().ok();
        }
        sound.clone()
    }

    fn start_sound(self: &Arc<Self>) -> io::Result<PathBuf> {
        let path = self.dir()?.join("pulse");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let me = Arc::downgrade(self);
        thread::Builder::new()
            .name("hof-agent-sound".into())
            .spawn(move || {
                accept(&Listener::Unix(listener), |socket| {
                    let me = me.clone();
                    thread::spawn(move || take_sound(&me, socket));
                });
            })?;
        Ok(path)
    }
}

impl Drop for Forwards {
    fn drop(&mut self) {
        self.remove_files();
    }
}

enum Listener {
    Unix(UnixListener),
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Tcp(std::net::TcpListener),
}

fn accept(listener: &Listener, mut each: impl FnMut(Box<dyn Socket>)) {
    loop {
        let socket: io::Result<Box<dyn Socket>> = match listener {
            Listener::Unix(l) => l.accept().map(|(s, _)| Box::new(s) as Box<dyn Socket>),
            Listener::Tcp(l) => l.accept().map(|(s, _)| Box::new(s) as Box<dyn Socket>),
        };
        match socket {
            Ok(socket) => {
                if socket.same_user().is_err() {
                    socket.close();
                } else {
                    each(socket);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// A display no server here has, listening: on Linux the abstract socket
/// X11 clients try first, which needs no `/tmp/.X11-unix` to write to.
#[cfg(target_os = "linux")]
fn bind_display() -> io::Result<(Listener, u32)> {
    use std::os::linux::net::SocketAddrExt;
    let mut last = io::Error::other("no display number free");
    for n in FIRST_DISPLAY..FIRST_DISPLAY + 1000 {
        // A server that has the number, listening some other way.
        if Path::new(&format!("/tmp/.X{n}-lock")).exists()
            || Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
        {
            continue;
        }
        let addr =
            std::os::unix::net::SocketAddr::from_abstract_name(format!("/tmp/.X11-unix/X{n}"))?;
        match UnixListener::bind_addr(&addr) {
            Ok(listener) => return Ok((Listener::Unix(listener), n)),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(not(target_os = "linux"))]
fn bind_display() -> io::Result<(Listener, u32)> {
    let mut last = io::Error::other("no display number free");
    for n in FIRST_DISPLAY..FIRST_DISPLAY + 1000 {
        match std::net::TcpListener::bind(("127.0.0.1", (6000 + n) as u16)) {
            Ok(listener) => return Ok((Listener::Tcp(listener), n)),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn display_name(number: u32) -> String {
    if cfg!(target_os = "linux") {
        format!(":{number}")
    } else {
        format!("localhost:{number}")
    }
}

fn private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} must be a directory owned by this user and private to it",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn random_cookie() -> io::Result<[u8; 16]> {
    let mut cookie = [0; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut cookie)?;
    Ok(cookie)
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Inside the private directory an existing link is ours to replace;
    // create_new then keeps a race from becoming a followed symlink.
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)
}

/// Reads what `socket` opens with, as far as `whole` says it goes.
fn opening<T, E>(
    socket: &mut Box<dyn Socket>,
    whole: impl Fn(&[u8]) -> Result<Option<T>, E>,
) -> Option<(Vec<u8>, T)> {
    socket.read_timeout(Some(OPENING_WAIT)).ok()?;
    let mut got = Vec::new();
    let mut buf = [0; 4096];
    let found = loop {
        if let Some(found) = whole(&got).ok()? {
            break found;
        }
        match socket.read(&mut buf) {
            Ok(0) => return None,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    };
    socket.read_timeout(None).ok()?;
    Some((got, found))
}

fn take_x11(forwards: &Weak<Forwards>, mut socket: Box<dyn Socket>, cookie: &[u8]) {
    let Some((got, (setup, len))) = opening(&mut socket, x11::Setup::parse) else {
        socket.close();
        return;
    };
    let refuse = |mut socket: Box<dyn Socket>, why: &str| {
        let _ = socket.write_all(&x11::refusal(setup.big_endian, why));
        socket.close();
    };
    if setup.auth_name != x11::COOKIE || setup.auth_data != cookie {
        return refuse(socket, "Hof: not this display's cookie");
    }
    let Some(forwards) = forwards.upgrade() else {
        return refuse(socket, "Hof: the agent is going");
    };
    let Some(relays) = forwards.route(x11::SERVICE) else {
        return refuse(socket, "Hof: no Hof is connected to show it");
    };
    // What only this machine needs to know stays here.
    let mut opening = setup.with_cookie(None).encode();
    opening.extend_from_slice(&got[len..]);
    relays.connect_out(forwards.channel(), x11::SERVICE, opening, socket);
}

fn take_sound(forwards: &Weak<Forwards>, mut socket: Box<dyn Socket>) {
    let Some((opening, _)) = opening(&mut socket, pulse::packet_len) else {
        socket.close();
        return;
    };
    let route = forwards
        .upgrade()
        .and_then(|f| Some((f.route(pulse::SERVICE)?, f.channel())));
    match route {
        Some((relays, channel)) => relays.connect_out(channel, pulse::SERVICE, opening, socket),
        None => socket.close(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forwarding_directory_must_be_private_and_not_a_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("forward");
        private_dir(&dir).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(&dir).is_err());
        std::fs::remove_dir(&dir).unwrap();
        std::os::unix::fs::symlink(temp.path(), &dir).unwrap();
        assert!(private_dir(&dir).is_err());
    }

    #[test]
    fn an_xauthority_symlink_is_replaced_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("precious");
        std::fs::write(&target, b"keep me").unwrap();
        let path = temp.path().join("Xauthority");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        write_private(&path, b"cookie").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
        assert_eq!(std::fs::read(&path).unwrap(), b"cookie");
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
