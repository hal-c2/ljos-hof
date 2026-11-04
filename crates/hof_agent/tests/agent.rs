//! The agent over a socket pair, spoken to as Hof would: panes opened,
//! watched, typed into, dropped and picked up again.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::{Agent, Options};
use hof_remote::wire::{Decoder, Message, PaneId, VERSION};

const PATIENCE: Duration = Duration::from_secs(10);

fn agent() -> Agent {
    Agent::new(Options {
        name: "test".into(),
        ..Options::default()
    })
}

/// One connection to the agent.
struct Client {
    stream: UnixStream,
    decoder: Decoder,
}

impl Client {
    fn to(agent: &Agent) -> Self {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let agent = agent.clone();
        thread::spawn(move || {
            let reader = theirs.try_clone().unwrap();
            let _ = agent.serve(reader, theirs);
        });
        ours.set_read_timeout(Some(PATIENCE)).unwrap();
        Self {
            stream: ours,
            decoder: Decoder::new(),
        }
    }

    fn send(&mut self, message: Message) {
        let mut frame = Vec::new();
        message.encode(&mut frame);
        self.stream.write_all(&frame).unwrap();
    }

    fn next(&mut self) -> Message {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(m) = self.decoder.next_message() {
                return m.unwrap();
            }
            assert!(Instant::now() < deadline, "the agent said nothing");
            let mut buf = [0; 4096];
            let n = self
                .stream
                .read(&mut buf)
                .expect("the agent says something");
            assert!(n > 0, "the agent hung up");
            self.decoder.push(&buf[..n]);
        }
    }

    fn open(&mut self, command: &[&str]) -> PaneId {
        self.send(Message::Open {
            request: 1,
            command: command.iter().map(|s| s.to_string()).collect(),
            directory: String::new(),
            environment: vec!["HOF_TEST=yes".into()],
            cols: 40,
            rows: 5,
        });
        match self.next() {
            Message::Opened { request: 1, pane } => pane,
            other => panic!("not opened: {other:?}"),
        }
    }

    /// Attaches and returns the input count the agent says it has.
    fn attach(&mut self, pane: PaneId, have: u64) -> u64 {
        self.send(Message::Attach { pane, have });
        match self.next() {
            Message::Attached { pane: p, input, .. } if p == pane => input,
            other => panic!("not attached: {other:?}"),
        }
    }

    /// The pane's output until it shows `text`: everything that came, the
    /// number the first piece started at, and how far it got.
    ///
    /// A snapshot starts it all again: what came before it is no longer the
    /// screen, and the stream goes on from where the snapshot says.
    fn output_until(&mut self, pane: PaneId, text: &str) -> Seen {
        let mut seen = Seen::default();
        while !String::from_utf8_lossy(&seen.bytes).contains(text) {
            match self.next() {
                Message::Output { pane: p, at, bytes } if p == pane => {
                    if seen.first.is_some() {
                        assert_eq!(at, seen.end, "the pieces follow on");
                    }
                    seen.first.get_or_insert(at);
                    seen.end = at + bytes.len() as u64;
                    seen.bytes.extend(bytes);
                }
                Message::Snapshot { pane: p, at, bytes } if p == pane => {
                    seen.first = Some(at);
                    seen.end = at;
                    seen.bytes = bytes;
                }
                other => panic!("waiting for {text:?}, got {other:?}"),
            }
        }
        seen
    }
}

#[derive(Default, Debug)]
struct Seen {
    bytes: Vec<u8>,
    first: Option<u64>,
    end: u64,
}

impl Seen {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

#[test]
fn the_agent_says_who_it_is_and_answers_pings() {
    let agent = agent();
    let mut c = Client::to(&agent);
    c.send(Message::Hello {
        version: VERSION,
        name: "hof".into(),
        instance: 0,
    });
    assert_eq!(
        c.next(),
        Message::Hello {
            version: VERSION,
            name: "test".into(),
            instance: agent.instance(),
        }
    );
    c.send(Message::Ping { nonce: 42 });
    assert_eq!(c.next(), Message::Pong { nonce: 42 });
    // Another agent is another instance.
    assert_ne!(Agent::new(Options::default()).instance(), agent.instance());
}

#[test]
fn a_pane_runs_its_program_takes_input_and_says_how_it_ended() {
    let agent = agent();
    let mut c = Client::to(&agent);
    let pane = c.open(&[
        "/bin/sh",
        "-c",
        "echo env:$HOF_TEST; read x; echo got:$x; exit 3",
    ]);
    assert_eq!(c.attach(pane, 0), 0);
    let seen = c.output_until(pane, "env:yes");
    assert_eq!(seen.first, Some(0));
    c.send(Message::Input {
        pane,
        at: 0,
        bytes: b"hi\r".to_vec(),
    });
    c.output_until(pane, "got:hi");
    // Everything it wrote came before its end.
    loop {
        match c.next() {
            Message::Output { .. } => {}
            Message::Exited { pane: p, code } => {
                assert_eq!((p, code), (pane, Some(3)));
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    c.send(Message::List);
    match c.next() {
        Message::Panes { panes } => {
            assert_eq!(panes.len(), 1);
            assert_eq!(panes[0].pane, pane);
            assert_eq!((panes[0].cols, panes[0].rows), (40, 5));
            assert_eq!(panes[0].exited, Some(Some(3)));
        }
        other => panic!("{other:?}"),
    }
    c.send(Message::Close { pane });
    c.send(Message::List);
    assert_eq!(c.next(), Message::Panes { panes: vec![] });
}

#[test]
fn a_client_that_comes_back_gets_only_what_it_missed_and_types_nothing_twice() {
    let agent = agent();
    let mut first = Client::to(&agent);
    let pane = first.open(&[
        "/bin/sh",
        "-c",
        "echo ready; read a; echo A:$a; read b; echo B:$b; read c",
    ]);
    first.attach(pane, 0);
    let had = first.output_until(pane, "ready").end;
    // Typed, and the connection drops before it hears anything back.
    first.send(Message::Input {
        pane,
        at: 0,
        bytes: b"one\r".to_vec(),
    });
    drop(first);

    let mut again = Client::to(&agent);
    // The agent has the first line typed: the client sends it again all
    // the same, not knowing, with the next.
    let input = again.attach(pane, had);
    assert!(input == 0 || input == 4, "{input}");
    again.send(Message::Input {
        pane,
        at: 0,
        bytes: b"one\rtwo\r".to_vec(),
    });
    let seen = again.output_until(pane, "B:two");
    assert_eq!(seen.first, Some(had), "carries on where it left off");
    let text = seen.text();
    assert!(!text.contains("ready"), "{text:?}");
    assert!(text.contains("A:one"), "{text:?}");
    assert_eq!(
        text.matches("one").count(),
        2,
        "echoed and printed once: {text:?}"
    );
}

#[test]
fn a_client_too_far_behind_gets_the_screen_as_it_is() {
    let agent = Agent::new(Options {
        backlog: 64,
        ..Options::default()
    });
    let mut first = Client::to(&agent);
    let pane = first.open(&["/bin/sh", "-c", "seq 1 50; echo done; read x"]);
    first.attach(pane, 0);
    let end = first.output_until(pane, "done").end;
    drop(first);

    let mut again = Client::to(&agent);
    again.attach(pane, 0);
    match again.next() {
        Message::Snapshot { pane: p, at, bytes } => {
            assert_eq!((p, at), (pane, end));
            let copy = hof_vt::Session::detached(
                hof_vt::Options {
                    size: hof_vt::Size::new(40, 5),
                    ..hof_vt::Options::default()
                },
                || {},
            )
            .unwrap();
            copy.feed(&bytes);
            let screen = copy.screen_text();
            assert!(
                screen.contains("50") && screen.contains("done"),
                "{screen:?}"
            );
        }
        other => panic!("not a snapshot: {other:?}"),
    }
    // And the stream carries on from there.
    again.send(Message::Input {
        pane,
        at: 0,
        bytes: b"z\r".to_vec(),
    });
    let seen = again.output_until(pane, "z");
    assert_eq!(seen.first, Some(end));
}

#[test]
fn a_pane_taken_by_another_connection_is_detached_from_the_first() {
    let agent = agent();
    let mut first = Client::to(&agent);
    let pane = first.open(&["/bin/sh", "-c", "read x"]);
    first.attach(pane, 0);
    let mut second = Client::to(&agent);
    second.attach(pane, 0);
    loop {
        match first.next() {
            Message::Detach { pane: p } => {
                assert_eq!(p, pane);
                break;
            }
            Message::Output { .. } => {}
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_pane_the_agent_does_not_have_is_gone() {
    let agent = agent();
    let mut c = Client::to(&agent);
    c.send(Message::Attach { pane: 99, have: 0 });
    assert_eq!(c.next(), Message::Gone { pane: 99 });
    c.send(Message::Input {
        pane: 98,
        at: 0,
        bytes: b"x".to_vec(),
    });
    assert_eq!(c.next(), Message::Gone { pane: 98 });
}

#[test]
fn a_program_that_cannot_start_is_refused() {
    let agent = agent();
    let mut c = Client::to(&agent);
    c.send(Message::Open {
        request: 5,
        command: vec!["/nonexistent/program".into()],
        directory: String::new(),
        environment: vec![],
        cols: 10,
        rows: 2,
    });
    match c.next() {
        Message::Refused { request: 5, .. } => {}
        // A pty starts the program after the fork; it may only fail there.
        Message::Opened { request: 5, pane } => {
            c.attach(pane, 0);
            loop {
                if let Message::Exited { code, .. } = c.next() {
                    assert_ne!(code, Some(0));
                    break;
                }
            }
        }
        other => panic!("{other:?}"),
    }
}
