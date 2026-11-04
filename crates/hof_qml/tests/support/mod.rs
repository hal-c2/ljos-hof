//! What the remote tests share: an agent in this process, reached over
//! socket pairs the test can cut, and a terminal on it.

#![allow(dead_code)]

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::Agent;
use hof_qml::{Terminal, register, remote};
use hof_remote::client::{self, Connection, Host, Transport};
use ljos::prelude::*;
use ljos::testing::TestWindow;

pub const PATIENCE: Duration = Duration::from_secs(10);

/// Reaches `agent`, keeping each connection's end for the test to cut.
struct Pipe {
    agent: Agent,
    made: Arc<Mutex<Vec<UnixStream>>>,
}

impl Transport for Pipe {
    fn connect(&mut self) -> io::Result<Connection> {
        let (ours, theirs) = UnixStream::pair()?;
        let agent = self.agent.clone();
        thread::spawn(move || {
            let reader = theirs.try_clone().unwrap();
            let _ = agent.serve(reader, theirs);
        });
        self.made.lock().unwrap().push(ours.try_clone()?);
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

/// A host called `name` on its own agent. Each test has its own name: the
/// hosts are the process's.
pub fn host(name: &str) -> (Agent, Arc<Mutex<Vec<UnixStream>>>) {
    let agent = Agent::new(hof_agent::Options {
        name: name.into(),
        ..hof_agent::Options::default()
    });
    let made = Arc::new(Mutex::new(Vec::new()));
    let pipe = Pipe {
        agent: agent.clone(),
        made: made.clone(),
    };
    let options = client::Options {
        retry_first: Duration::from_millis(10),
        retry_longest: Duration::from_millis(50),
        ..client::Options::default()
    };
    remote::set_host(name, Host::new(pipe, options));
    (agent, made)
}

pub struct Shown {
    pub engine: Engine,
    pub window: TestWindow,
    pub t: ObjectId,
}

/// A terminal running `script` on `host`, or attaching to its pane
/// `pane`.
pub fn shown(host: &str, script: &str, pane: i32) -> Shown {
    let engine = ljos::engine();
    register(&engine);
    let mut window = TestWindow::new(&engine, 400.0, 200.0);
    let t = engine.create_by_name("Hof", "Terminal", None).unwrap();
    engine.set(t, Terminal::HOST, host.to_owned()).unwrap();
    engine.set(t, Terminal::REMOTE_PANE, pane).unwrap();
    engine
        .set(t, Terminal::PROGRAM, "/bin/sh".to_owned())
        .unwrap();
    let args = vec!["-c".to_owned(), script.to_owned()];
    engine.set(t, Terminal::ARGUMENTS, args).unwrap();
    engine.set(t, Item::WIDTH, 400.0).unwrap();
    engine.set(t, Item::HEIGHT, 200.0).unwrap();
    window.show(t).unwrap();
    window.settle().unwrap();
    Shown { engine, window, t }
}

impl Shown {
    pub fn text(&self) -> String {
        let text = self.engine.invoke_by_name(self.t, "text", &[]).unwrap();
        text.to_js_string().as_str().to_owned()
    }

    pub fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "waiting for {what}: {:?}, {:?}",
                self.text(),
                self.engine.get(self.t, Terminal::CONNECTION).unwrap()
            );
            thread::sleep(Duration::from_millis(5));
            self.window.settle().unwrap();
        }
    }

    pub fn until_text(&mut self, text: &str) {
        self.until(text, |s| s.text().contains(text));
    }

    pub fn connection(&self) -> String {
        self.engine.get(self.t, Terminal::CONNECTION).unwrap()
    }

    pub fn remote_pane(&self) -> i32 {
        self.engine.get(self.t, Terminal::REMOTE_PANE).unwrap()
    }

    pub fn type_text(&mut self, text: &str) {
        self.engine
            .invoke_by_name(self.t, "send", &[Value::from(text)])
            .unwrap();
        self.window.settle().unwrap();
    }
}
