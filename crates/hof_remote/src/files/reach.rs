//! A host's files service, as its copies and its mount reach it: one 9P
//! session at a time, opened when it is first wanted, and opened again
//! when the one there was has gone.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::client::Client;

/// Reaching the files failed: not tried again for this long.
const RETRY: Duration = Duration::from_secs(5);
/// How long a request may take before the session is taken to be gone.
const PATIENCE: Duration = Duration::from_secs(30);

/// What opens a new channel to the files service.
pub(crate) type Connect = dyn Fn() -> io::Result<UnixStream> + Send + Sync;

pub(crate) struct Reach {
    host: String,
    /// Where on the host the session is attached.
    root: Vec<u8>,
    connect: Box<Connect>,
    /// One request at a time.
    session: Mutex<Session>,
}

#[derive(Default)]
struct Session {
    client: Option<Client<UnixStream>>,
    /// Reaching the files failed; not tried again before this.
    retry_at: Option<Instant>,
    /// Counts the sessions opened: what was opened in one is gone in the
    /// next.
    number: u64,
}

impl Reach {
    pub(crate) fn new(
        host: &str,
        root: &[u8],
        connect: impl Fn() -> io::Result<UnixStream> + Send + Sync + 'static,
    ) -> Self {
        Self {
            host: host.to_owned(),
            root: root.to_vec(),
            connect: Box::new(connect),
            session: Mutex::default(),
        }
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tries at once, the next time, however recently it failed: the
    /// user asked.
    pub(crate) fn retry_now(&self) {
        self.session().retry_at = None;
    }

    /// Lets the session go, while nothing needs it.
    pub(crate) fn hang_up(&self) {
        self.session().client = None;
    }

    fn connect_client(&self) -> io::Result<Client<UnixStream>> {
        let socket = (self.connect)()?;
        socket.set_read_timeout(Some(PATIENCE))?;
        socket.set_write_timeout(Some(PATIENCE))?;
        let user = std::env::var("USER").unwrap_or_else(|_| "hof".into());
        Client::attached(socket, &user, &self.root).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "the agent on {} does not share files: it is older than this Hof; \
                         update Hof there",
                        self.host
                    ),
                )
            } else {
                e
            }
        })
    }

    /// Does `op` with the session, opened if there is none; again on a new
    /// one if the one there was turns out to have gone.
    pub(crate) fn with_client<T>(
        &self,
        op: impl Fn(&mut Client<UnixStream>) -> io::Result<T>,
    ) -> io::Result<T> {
        self.with_session(true, |client, _| op(client))
    }

    /// Does `op` with the session and its number, opened if there is
    /// none. If the session there was turns out to have gone, `op` is done
    /// again on a new one only when `again`: when doing it twice is the
    /// same as doing it once.
    pub(crate) fn with_session<T>(
        &self,
        again: bool,
        op: impl Fn(&mut Client<UnixStream>, u64) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut session = self.session();
        if session.retry_at.is_some_and(|at| Instant::now() < at) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!("the files on {} can't be reached just now", self.host),
            ));
        }
        let mut fresh = false;
        loop {
            let Session { client, number, .. } = &mut *session;
            let client = match client {
                Some(client) => client,
                none => {
                    fresh = true;
                    match self.connect_client() {
                        Ok(client) => {
                            *number += 1;
                            none.insert(client)
                        }
                        Err(e) => {
                            session.retry_at = Some(Instant::now() + RETRY);
                            return Err(e);
                        }
                    }
                }
            };
            let done = op(client, *number);
            if done.is_ok() || !client.is_broken() {
                return done;
            }
            session.client = None;
            if fresh {
                session.retry_at = Some(Instant::now() + RETRY);
                return done;
            }
            if !again {
                return done;
            }
        }
    }
}
