//! X11 and sound clients on the agent's machine, reaching servers on
//! Hof's: a real client and agent between, the servers and the clients
//! played by the test.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::{Agent, Options};
use hof_remote::client::{self, Connection, Host, Screen, Service, Start, State, Transport};
use hof_remote::{pulse, x11};

const PATIENCE: Duration = Duration::from_secs(10);

/// A transport to an agent in this process.
struct Pipe(Agent);

impl Transport for Pipe {
    fn connect(&mut self) -> io::Result<Connection> {
        let (ours, theirs) = UnixStream::pair()?;
        let agent = self.0.clone();
        thread::spawn(move || {
            let reader = theirs.try_clone().unwrap();
            let _ = agent.serve(reader, theirs);
        });
        let cut = ours.try_clone()?;
        Ok(Connection {
            reader: Box::new(ours.try_clone()?),
            writer: Box::new(ours),
            end: Box::new(move || {
                let _ = cut.shutdown(std::net::Shutdown::Both);
                None
            }),
        })
    }
}

/// A screen that collects the text.
#[derive(Default)]
struct Seen {
    text: Mutex<Vec<u8>>,
    changed: Condvar,
}

impl Screen for Seen {
    fn output(&self, bytes: &[u8]) {
        self.text.lock().unwrap().extend_from_slice(bytes);
        self.changed.notify_all();
    }
    fn replace(&self, snapshot: &[u8]) {
        *self.text.lock().unwrap() = snapshot.to_vec();
        self.changed.notify_all();
    }
    fn exited(&self, _: Option<i32>) {}
    fn state(&self, _: State) {}
}

impl Seen {
    /// The first line the program writes that starts with `prefix`, what
    /// follows it.
    fn line(&self, prefix: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        let mut text = self.text.lock().unwrap();
        loop {
            let shown = String::from_utf8_lossy(&text).into_owned();
            if let Some(line) = shown
                .lines()
                .find_map(|l| l.trim().strip_prefix(prefix))
                .filter(|_| shown.contains('\n'))
            {
                return line.to_owned();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "waiting for {prefix:?}: {shown:?}");
            text = self.changed.wait_timeout(text, left).unwrap().0;
        }
    }
}

/// An agent that takes its files away when the test ends, its panes
/// still running or not.
struct Tidy(Agent);

impl std::ops::Deref for Tidy {
    type Target = Agent;
    fn deref(&self) -> &Agent {
        &self.0
    }
}

impl Drop for Tidy {
    fn drop(&mut self) {
        self.0.remove_files();
    }
}

fn agent() -> Tidy {
    Tidy(Agent::new(Options {
        name: "test".into(),
        ..Options::default()
    }))
}

fn host(agent: &Agent, services: Vec<Arc<dyn Service>>) -> Host {
    let options = client::Options {
        services,
        ..client::Options::default()
    };
    Host::new(Pipe(agent.clone()), options)
}

/// A pane that says what it was given, and stays.
fn pane_saying(host: &Host, variables: &[&str]) -> Arc<Seen> {
    let script = variables
        .iter()
        .map(|v| format!("echo {v}=${v}"))
        .collect::<Vec<_>>()
        .join("; ");
    let seen = Arc::new(Seen::default());
    let start = Start {
        command: vec!["/bin/sh".into(), "-c".into(), format!("{script}; sleep 30")],
        cols: 200,
        rows: 10,
        ..Start::default()
    };
    // Dropped, the pane runs on: the agent ends with the test.
    let _ = host.open(start, seen.clone());
    seen
}

/// A server here: what the first connection to it said, once it said
/// `wanted` bytes, and that connection, to answer on.
fn serve_once(listener: UnixListener, wanted: usize) -> thread::JoinHandle<(Vec<u8>, UnixStream)> {
    thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(PATIENCE)).unwrap();
        let mut got = vec![0; wanted];
        s.read_exact(&mut got).unwrap();
        (got, s)
    })
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// A client of the display the agent made: on Linux its abstract socket,
/// elsewhere TCP on this machine.
fn x_client(display: &str) -> Box<dyn Stream> {
    let (target, _) = x11::Target::of(display).expect("a display");
    match target {
        #[cfg(target_os = "linux")]
        x11::Target::Local(n) => {
            use std::os::linux::net::SocketAddrExt;
            let addr =
                std::os::unix::net::SocketAddr::from_abstract_name(format!("/tmp/.X11-unix/X{n}"))
                    .unwrap();
            let s = UnixStream::connect_addr(&addr).unwrap();
            s.set_read_timeout(Some(PATIENCE)).unwrap();
            Box::new(s)
        }
        x11::Target::Tcp(host, port) => {
            let s = std::net::TcpStream::connect((host.as_str(), port)).unwrap();
            s.set_read_timeout(Some(PATIENCE)).unwrap();
            Box::new(s)
        }
        other => panic!("not a display the agent makes: {other:?}"),
    }
}

fn cookie_in(xauthority: &str, display: &str) -> Vec<u8> {
    let (_, number) = x11::Target::of(display).unwrap();
    let file = std::fs::read(xauthority).unwrap();
    x11::Auth::cookie_for(&x11::Auth::parse_all(&file), &number, &x11::host_name())
        .expect("a cookie for the display")
}

fn setup_with(cookie: &[u8]) -> Vec<u8> {
    x11::Setup {
        big_endian: false,
        major: 11,
        minor: 0,
        auth_name: Vec::new(),
        auth_data: Vec::new(),
    }
    .with_cookie(Some(cookie))
    .encode()
}

/// An X server here, with its cookie in an authorization file.
fn x_server(dir: &Path) -> (UnixListener, Arc<dyn Service>) {
    let socket = dir.join("X0");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut file = Vec::new();
    x11::Auth {
        family: x11::FAMILY_WILD,
        address: Vec::new(),
        number: b"0".to_vec(),
        name: x11::COOKIE.to_vec(),
        data: b"the real cookie!".to_vec(),
    }
    .encode(&mut file);
    std::fs::write(dir.join("Xauthority"), file).unwrap();
    let display = format!("{}:0", socket.display());
    let forward = x11::Forward::new(&display, Some(dir.join("Xauthority"))).unwrap();
    (listener, Arc::new(forward))
}

#[test]
fn an_x11_client_there_reaches_the_server_here_with_its_cookie() {
    let dir = tempfile::tempdir().unwrap();
    let (listener, service) = x_server(dir.path());
    let agent = agent();
    let host = host(&agent, vec![service]);
    let seen = pane_saying(&host, &["DISPLAY", "XAUTHORITY"]);
    let display = seen.line("DISPLAY=");
    let xauthority = seen.line("XAUTHORITY=");
    let ours = if cfg!(target_os = "linux") {
        ":"
    } else {
        "localhost:"
    };
    assert!(display.starts_with(ours), "{display}");

    let fake = cookie_in(&xauthority, &display);
    assert_ne!(
        fake, b"the real cookie!",
        "the cookie there is not the one here"
    );
    let real = setup_with(b"the real cookie!");
    let server = serve_once(listener, real.len() + 8);
    let mut client = x_client(&display);
    client.write_all(&setup_with(&fake)).unwrap();
    client.write_all(b"requests").unwrap();

    let (got, mut at_server) = server.join().unwrap();
    assert_eq!(got[..real.len()], real, "the server gets the cookie here");
    assert_eq!(&got[real.len()..], b"requests");
    at_server.write_all(b"replies").unwrap();
    let mut back = [0; 7];
    client.read_exact(&mut back).unwrap();
    assert_eq!(&back, b"replies");

    // The client goes; the server sees it go.
    drop(client);
    let mut rest = Vec::new();
    at_server.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
}

#[test]
fn an_x11_client_without_the_cookie_is_refused_there() {
    let dir = tempfile::tempdir().unwrap();
    let (_listener, service) = x_server(dir.path());
    let agent = agent();
    let host = host(&agent, vec![service]);
    let display = pane_saying(&host, &["DISPLAY"]).line("DISPLAY=");
    let mut client = x_client(&display);
    client.write_all(&setup_with(b"a guess at it...")).unwrap();
    let mut answer = Vec::new();
    client.read_to_end(&mut answer).unwrap();
    assert_eq!(answer[0], 0, "failed");
    let reason = String::from_utf8_lossy(&answer[8..8 + usize::from(answer[1])]).into_owned();
    assert!(reason.contains("cookie"), "{reason}");
}

#[test]
fn an_x11_client_is_told_when_no_hof_is_connected() {
    let dir = tempfile::tempdir().unwrap();
    let (_listener, service) = x_server(dir.path());
    let agent = agent();
    let host = host(&agent, vec![service]);
    let seen = pane_saying(&host, &["DISPLAY", "XAUTHORITY"]);
    let display = seen.line("DISPLAY=");
    let fake = cookie_in(&seen.line("XAUTHORITY="), &display);
    // Hof goes; the pane, and its display, stay.
    host.finish(true, PATIENCE);
    let deadline = Instant::now() + PATIENCE;
    loop {
        let mut client = x_client(&display);
        client.write_all(&setup_with(&fake)).unwrap();
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).unwrap();
        let reason = String::from_utf8_lossy(answer.get(8..).unwrap_or_default()).into_owned();
        if reason.contains("no Hof") {
            break;
        }
        // The agent may not have seen the connection go yet.
        assert!(Instant::now() < deadline, "{answer:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

/// A client's authorization, as libpulse sends it: offering to share
/// memory, with the cookie it has.
fn pulse_auth(cookie: u8, shares: bool) -> Vec<u8> {
    let version: u32 = 35 | if shares { 0xc000_0000 } else { 0 };
    let mut payload = vec![b'L'];
    payload.extend(8u32.to_be_bytes());
    payload.push(b'L');
    payload.extend(1u32.to_be_bytes());
    payload.push(b'L');
    payload.extend(version.to_be_bytes());
    payload.push(b'x');
    payload.extend(256u32.to_be_bytes());
    payload.extend([cookie; 256]);
    let mut packet = (payload.len() as u32).to_be_bytes().to_vec();
    packet.extend(u32::MAX.to_be_bytes());
    packet.extend([0; 12]);
    packet.extend(payload);
    packet
}

#[test]
fn a_sound_client_there_reaches_the_server_here_with_its_cookie() {
    let dir = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(dir.path().join("native")).unwrap();
    std::fs::write(dir.path().join("cookie"), [7; 256]).unwrap();
    let service = pulse::Forward::new(dir.path().join("native"), vec![dir.path().join("cookie")]);
    let agent = agent();
    let host = host(&agent, vec![Arc::new(service)]);
    let server = seen_server(listener);
    let pulse_server = pane_saying(&host, &["PULSE_SERVER"]).line("PULSE_SERVER=");
    let path = pulse_server.strip_prefix("unix:").expect("a socket");

    let mut client = UnixStream::connect(path).unwrap();
    client.set_read_timeout(Some(PATIENCE)).unwrap();
    client.write_all(&pulse_auth(1, true)).unwrap();
    client.write_all(b"play").unwrap();
    let (got, mut at_server) = server.join().unwrap();
    let wanted = pulse_auth(7, false);
    assert!(
        got[..wanted.len()] == wanted,
        "the cookie here, and no shared memory"
    );
    assert_eq!(&got[wanted.len()..], b"play");
    at_server.write_all(b"ok").unwrap();
    let mut back = [0; 2];
    client.read_exact(&mut back).unwrap();
    assert_eq!(&back, b"ok");
}

fn seen_server(listener: UnixListener) -> thread::JoinHandle<(Vec<u8>, UnixStream)> {
    serve_once(listener, pulse_auth(0, false).len() + 4)
}

/// The X server and the sound server of the desktop the test runs on,
/// with real programs: `xdpyinfo` and `pactl`.
#[test]
#[ignore = "needs a desktop with X11 and PulseAudio or PipeWire, and xdpyinfo and pactl"]
fn programs_there_use_the_desktop_here() {
    let x = x11::Forward::from_env().expect("a DISPLAY");
    let sound = pulse::Forward::from_env().expect("a sound server");
    let agent = agent();
    let host = host(&agent, vec![Arc::new(x), Arc::new(sound)]);
    let seen = Arc::new(Seen::default());
    let start = Start {
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "echo X=$(xdpyinfo | grep -c 'number of screens'); \
             echo SOUND=$(pactl info | grep -c 'Server Name'); sleep 30"
                .into(),
        ],
        cols: 200,
        rows: 10,
        ..Start::default()
    };
    let _ = host.open(start, seen.clone());
    assert_eq!(seen.line("X="), "1", "xdpyinfo reached the X server here");
    assert_eq!(
        seen.line("SOUND="),
        "1",
        "pactl reached the sound server here"
    );
}
