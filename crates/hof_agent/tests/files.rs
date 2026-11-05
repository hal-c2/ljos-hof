//! The agent's machine's files, reached from Hof's over the files channel:
//! a real client and agent between, both machines this one.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::{Agent, Options};
use hof_remote::client::{self, Connection, Host, Screen, Start, State, Transport};
use hof_remote::files::{self, Client, Mirror, Opened};

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

struct Blank;

impl Screen for Blank {
    fn output(&self, _: &[u8]) {}
    fn replace(&self, _: &[u8]) {}
    fn exited(&self, _: Option<i32>) {}
    fn state(&self, _: State) {}
}

/// A host whose connection is up, as it is while a pane is open there.
fn host() -> (Agent, Host) {
    let agent = Agent::new(Options {
        name: "test".into(),
        ..Options::default()
    });
    let host = Host::new(Pipe(agent.clone()), client::Options::default());
    let start = Start {
        command: vec!["/bin/sh".into(), "-c".into(), "sleep 30".into()],
        cols: 80,
        rows: 10,
        ..Start::default()
    };
    // Dropped, the pane runs on: the agent ends with the test.
    let _ = host.open(start, Arc::new(Blank));
    (agent, host)
}

fn bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < deadline, "waiting for {what}");
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_client_reads_lists_and_writes_the_agents_files() {
    let (agent, host) = host();
    let there = tempfile::tempdir().unwrap();
    std::fs::write(there.path().join("a.txt"), "hello from there\n").unwrap();
    std::fs::create_dir(there.path().join("sub")).unwrap();

    let socket = host.connect(files::SERVICE, Vec::new()).unwrap();
    socket.set_read_timeout(Some(PATIENCE)).unwrap();
    let mut client = Client::new(socket, "me").unwrap();

    let mut got = Vec::new();
    let attr = client
        .read_file(&bytes(&there.path().join("a.txt")), &mut got)
        .unwrap();
    assert_eq!(got, b"hello from there\n");
    assert_eq!(attr.size, got.len() as u64);

    let mut names: Vec<_> = client
        .list(&bytes(there.path()))
        .unwrap()
        .into_iter()
        .map(|e| String::from_utf8(e.name).unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "sub"]);

    // Made, as it was gone; and made again shorter.
    let new = there.path().join("sub/new.txt");
    client.write_file(&bytes(&new), b"a longer first").unwrap();
    client.write_file(&bytes(&new), b"second").unwrap();
    assert_eq!(std::fs::read(&new).unwrap(), b"second");

    let missing = client.stat(&bytes(&there.path().join("nope")));
    assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
    // A failed request leaves the session as it was.
    assert!(!client.is_broken());
    client.stat(&bytes(there.path())).unwrap();
    agent.remove_files();
}

#[test]
fn an_opened_file_is_copied_here_and_kept_the_same_both_ways() {
    let (agent, host) = host();
    let there = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let remote = there.path().join("notes.md");
    std::fs::write(&remote, "# first\n").unwrap();

    let mirror = Mirror::new(cache.path().join("hosts/test"), "test", move || {
        host.connect(files::SERVICE, Vec::new())
    });
    let local = mirror.fetch(&bytes(&remote)).unwrap();
    assert!(
        local.starts_with(cache.path().join("hosts/test")),
        "{local:?}"
    );
    assert!(local.ends_with("notes.md"));
    assert_eq!(std::fs::read(&local).unwrap(), b"# first\n");
    let mode = std::fs::metadata(&local).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "only this user's, and not to be run");

    // Saved here: written back there.
    std::fs::write(&local, "# edited here\n").unwrap();
    until("the edit to reach the file there", || {
        std::fs::read(&remote).unwrap() == b"# edited here\n"
    });

    // Changed there: copied here again.
    thread::sleep(Duration::from_millis(20));
    std::fs::write(&remote, "# changed there, longer\n").unwrap();
    until("the change there to reach the copy", || {
        std::fs::read(&local).unwrap() == b"# changed there, longer\n"
    });

    // Opened again: the same copy.
    assert_eq!(mirror.fetch(&bytes(&remote)).unwrap(), local);

    // Gone from here: no longer watched.
    std::fs::remove_file(&local).unwrap();
    until("the copy to be forgotten", || {
        !mirror.is_watched(&bytes(&remote))
    });
    agent.remove_files();
}

#[test]
fn a_file_that_is_not_there_is_said_so() {
    let (agent, host) = host();
    let cache = tempfile::tempdir().unwrap();
    let mirror = Mirror::new(cache.path().join("hosts/test"), "test", move || {
        host.connect(files::SERVICE, Vec::new())
    });
    let err = mirror.fetch(b"/no/such/file/here").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound, "{err}");
    let dir = mirror.fetch(b"/").unwrap_err();
    assert_eq!(dir.kind(), io::ErrorKind::InvalidInput, "{dir}");
    agent.remove_files();
}

#[test]
fn an_opened_folder_is_copied_here_read_only_and_a_file_of_it_opened_alone_is_kept_the_same() {
    let (agent, host) = host();
    let there = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let folder = there.path().join("project");
    std::fs::create_dir_all(folder.join("sub/deeper")).unwrap();
    std::fs::write(folder.join("a.txt"), "a").unwrap();
    std::fs::write(folder.join("sub/b.txt"), "b").unwrap();
    std::fs::write(folder.join("sub/deeper/c.txt"), "c").unwrap();
    std::os::unix::fs::symlink("a.txt", folder.join("link")).unwrap();

    let mirror = Mirror::new(cache.path().join("hosts/test"), "test", move || {
        host.connect(files::SERVICE, Vec::new())
    });
    let Opened::Folder(local, copied) = mirror.open(&bytes(&folder)).unwrap() else {
        panic!("a folder is copied as one");
    };
    assert!(local.ends_with("project"), "{local:?}");
    assert_eq!(copied.files, 3);
    assert_eq!(copied.left_out, 1, "the link is left out");
    assert_eq!(std::fs::read(local.join("sub/deeper/c.txt")).unwrap(), b"c");
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode(&local.join("a.txt")),
        0o400,
        "read-only: not kept the same"
    );
    assert!(!mirror.is_watched(&bytes(&folder.join("a.txt"))));

    // One of its files opened by itself: writable, and kept the same.
    let a = mirror.open(&bytes(&folder.join("a.txt"))).unwrap();
    assert_eq!(a, Opened::File(local.join("a.txt")));
    assert_eq!(mode(&local.join("a.txt")), 0o600);
    std::fs::write(local.join("a.txt"), "a, edited here").unwrap();
    until("the edit to reach the file there", || {
        std::fs::read(folder.join("a.txt")).unwrap() == b"a, edited here"
    });

    // Copied again: what went there goes here, the file kept the same stays.
    std::fs::remove_dir_all(folder.join("sub/deeper")).unwrap();
    std::fs::write(folder.join("new.txt"), "new").unwrap();
    let Opened::Folder(_, copied) = mirror.open(&bytes(&folder)).unwrap() else {
        panic!("still a folder");
    };
    assert_eq!(copied.files, 3, "a.txt, new.txt and sub/b.txt");
    assert!(!local.join("sub/deeper").exists());
    assert_eq!(std::fs::read(local.join("new.txt")).unwrap(), b"new");
    assert_eq!(mode(&local.join("a.txt")), 0o600, "still kept the same");
    agent.remove_files();
}
