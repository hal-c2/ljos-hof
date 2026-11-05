//! This machine's sessions, for a Hof on another: `hof attach --host
//! desktop work` there opens `work` as it was left here, its programs
//! still running under the agent here, and keeps it here as it changes.
//! Back here, `hof attach work` opens it as it was left there.
//!
//! The agent serves it on a channel of its connection, one JSON object a
//! line each way: the Hof asks, the agent answers. A channel holds at most
//! one session, as a window here does, and lets it go when it ends.
//!
//! ```text
//! → "list"                         ← {"listed": [...]}
//! → {"hold": {"name": "work"}}     ← {"held": {"name": "work", "session": ...}}
//! → {"hold-free": {"first": "work"}}  ← {"held": {"name": "default", ...}}
//! → {"save": {...}}                ← "saved"
//! → "forget"                       ← "forgot"
//!                                  ← {"refused": "why"}, for any of them
//! ```

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{Held, Listed, Session};

/// The channel's service name.
pub const SERVICE: &str = "sessions";

/// The longest line either end reads: a session with its scrollback.
const MOST_LINE: u64 = 64 << 20;

/// Where the sessions are kept, and their locks.
#[derive(Clone, Debug)]
pub struct Places {
    /// Where the locks are, as [`crate::runtime_dir`].
    pub locks: PathBuf,
    /// Where the default session is, as [`crate::path`]; the named ones
    /// beside it.
    pub default: PathBuf,
}

impl Places {
    /// This machine's, as a Hof here keeps them.
    pub fn here() -> Self {
        Self {
            locks: crate::runtime_dir(),
            default: crate::path(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Ask {
    List,
    /// `None`: the session saved last that no Hof has open.
    Hold {
        name: Option<String>,
    },
    /// `first`, or when another Hof has it open the first of `default`,
    /// `2`, `3` and on that none has: a plain `hof`'s.
    HoldFree {
        first: String,
    },
    Save(Carried),
    Forget,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Answer {
    Listed(Vec<Listed>),
    Held {
        name: String,
        session: Option<Carried>,
    },
    Saved,
    Forgot,
    Refused(String),
}

/// A session as it goes across: with the scrollback the file keeps apart,
/// by window, then by pane id.
#[derive(Debug, Serialize, Deserialize)]
struct Carried {
    session: Session,
    #[serde(default)]
    scrollback: Vec<Vec<(u32, String)>>,
}

impl Carried {
    fn of(mut session: Session) -> Self {
        let scrollback = session
            .windows
            .iter_mut()
            .map(|w| {
                w.tabs
                    .iter_mut()
                    .flat_map(|t| t.panes_mut())
                    .filter_map(|p| Some((p.id, p.scrollback.take()?)))
                    .collect()
            })
            .collect();
        Self {
            session,
            scrollback,
        }
    }

    fn into_session(self) -> Session {
        let mut session = self.session;
        for (window, scrollback) in session.windows.iter_mut().zip(self.scrollback) {
            let mut panes: Vec<_> = window.tabs.iter_mut().flat_map(|t| t.panes_mut()).collect();
            for (id, text) in scrollback {
                if let Some(p) = panes.iter_mut().find(|p| p.id == id) {
                    p.scrollback = Some(text);
                }
            }
        }
        session
    }
}

/// One JSON object a line, each way.
struct Lines<S> {
    stream: BufReader<S>,
}

impl<S: Read + Write> Lines<S> {
    fn new(stream: S) -> Self {
        Self {
            stream: BufReader::new(stream),
        }
    }

    /// The next, or `None` once the other end is done.
    fn read<T: DeserializeOwned>(&mut self) -> io::Result<Option<T>> {
        let mut line = Vec::new();
        (&mut self.stream)
            .take(MOST_LINE + 1)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() {
            return Ok(None);
        }
        if line.last() != Some(&b'\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                if line.len() as u64 > MOST_LINE {
                    "a line too long"
                } else {
                    "a line cut short"
                },
            ));
        }
        serde_json::from_slice(&line)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn write<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
        line.push(b'\n');
        let stream = self.stream.get_mut();
        stream.write_all(&line)?;
        stream.flush()
    }
}

/// Serves `stream` from the sessions at `places`, until its other end is
/// done. The session it held is let go then.
pub fn serve(stream: impl Read + Write, places: &Places) -> io::Result<()> {
    let mut lines = Lines::new(stream);
    let mut held: Option<Held> = None;
    while let Some(ask) = lines.read::<Ask>()? {
        let answer = match (ask, &held) {
            (Ask::List, _) => Answer::Listed(crate::list_in(&places.locks, &places.default)),
            (Ask::Hold { .. } | Ask::HoldFree { .. }, Some(h)) => {
                Answer::Refused(format!("the session {} is held here already", h.name))
            }
            (Ask::Hold { name }, None) => taken(&mut held, held_here(places, name)),
            (Ask::HoldFree { first }, None) => taken(&mut held, free_here(places, &first)),
            (Ask::Save(_) | Ask::Forget, None) => Answer::Refused("no session is held".into()),
            (Ask::Save(carried), Some(h)) => {
                let file = crate::named_beside(&places.default, &h.name);
                match crate::save(&file, &carried.into_session()) {
                    Ok(()) => Answer::Saved,
                    Err(e) => Answer::Refused(format!("the session was not saved: {e}")),
                }
            }
            (Ask::Forget, Some(h)) => match crate::forget_beside(&places.default, &h.name) {
                Ok(()) => Answer::Forgot,
                Err(e) => Answer::Refused(format!("the session was not forgotten: {e}")),
            },
        };
        lines.write(&answer)?;
    }
    Ok(())
}

/// The answer to a hold: the session it `got`, now `held`, or why not.
fn taken(held: &mut Option<Held>, got: Result<(Held, Option<Session>), String>) -> Answer {
    match got {
        Ok((h, session)) => {
            let name = h.name.clone();
            *held = Some(h);
            Answer::Held {
                name,
                session: session.map(Carried::of),
            }
        }
        Err(why) => Answer::Refused(why),
    }
}

fn held_here(places: &Places, name: Option<String>) -> Result<(Held, Option<Session>), String> {
    let name = match name {
        Some(name) => name,
        None => crate::last_free_in(&places.locks, &places.default)
            .ok_or("no saved session is free to attach to")?,
    };
    let held = crate::hold_in(&places.locks, &name)
        .map_err(|e| format!("the session {name}: {e}"))?
        .ok_or_else(|| format!("another Hof has the session {name} open"))?;
    saved_here(places, held)
}

fn free_here(places: &Places, first: &str) -> Result<(Held, Option<Session>), String> {
    let held = crate::hold_free_in(&places.locks, first)
        .map_err(|e| format!("the session {first}: {e}"))?;
    saved_here(places, held)
}

fn saved_here(places: &Places, held: Held) -> Result<(Held, Option<Session>), String> {
    let name = &held.name;
    let session = crate::load(&crate::named_beside(&places.default, name))
        .map_err(|e| format!("the session {name} could not be read: {e}"))?;
    Ok((held, session))
}

/// The answer to `ask` on `lines`, or why there is none.
fn ask<S: Read + Write>(lines: &mut Lines<S>, ask: &Ask) -> io::Result<Answer> {
    // An agent with no such service hangs the channel up.
    let old = || {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the agent there keeps no sessions: update Hof there",
        )
    };
    match lines.write(ask) {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Err(old()),
        written => written?,
    }
    match lines.read()? {
        Some(Answer::Refused(why)) => Err(io::Error::other(why)),
        Some(answer) => Ok(answer),
        None => Err(old()),
    }
}

fn unexpected(answer: &Answer) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("the agent there answered {answer:?}"),
    )
}

/// The sessions saved on the machine at the other end of `stream`.
pub fn list(stream: impl Read + Write) -> io::Result<Vec<Listed>> {
    let mut lines = Lines::new(stream);
    match ask(&mut lines, &Ask::List)? {
        Answer::Listed(listed) => Ok(listed),
        other => Err(unexpected(&other)),
    }
}

/// A session on the machine at the other end, held open there for as long
/// as this lives.
pub struct Holding<S> {
    lines: Lines<S>,
    /// The session's name there.
    pub name: String,
}

/// Holds the session `name` on the machine at the other end of `stream`,
/// or with `None` the one saved last that no Hof has open there. With it,
/// the session as it was saved there, if it was.
pub fn hold<S: Read + Write>(
    stream: S,
    name: Option<&str>,
) -> io::Result<(Holding<S>, Option<Session>)> {
    held_there(
        stream,
        &Ask::Hold {
            name: name.map(str::to_owned),
        },
    )
}

/// Holds the session `first` on the machine at the other end of `stream`,
/// or when another Hof has it open there the first of `default`, `2`, `3`
/// and on that none has: a plain `hof`'s. With it, the session as it was
/// saved there, if it was.
pub fn hold_free<S: Read + Write>(
    stream: S,
    first: &str,
) -> io::Result<(Holding<S>, Option<Session>)> {
    held_there(
        stream,
        &Ask::HoldFree {
            first: first.to_owned(),
        },
    )
}

fn held_there<S: Read + Write>(
    stream: S,
    wanted: &Ask,
) -> io::Result<(Holding<S>, Option<Session>)> {
    let mut lines = Lines::new(stream);
    match ask(&mut lines, wanted)? {
        Answer::Held { name, session } => {
            Ok((Holding { lines, name }, session.map(Carried::into_session)))
        }
        other => Err(unexpected(&other)),
    }
}

impl<S: Read + Write> Holding<S> {
    /// Keeps `session` as the held one, there.
    pub fn save(&mut self, session: &Session) -> io::Result<()> {
        match ask(&mut self.lines, &Ask::Save(Carried::of(session.clone())))? {
            Answer::Saved => Ok(()),
            other => Err(unexpected(&other)),
        }
    }

    /// Forgets the held session, there: its every pane was closed.
    pub fn forget(&mut self) -> io::Result<()> {
        match ask(&mut self.lines, &Ask::Forget)? {
            Answer::Forgot => Ok(()),
            other => Err(unexpected(&other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::thread;

    use super::*;
    use crate::{SavedNode, SavedPane, SavedTab, SavedWindow};

    fn one_pane(host: Option<&str>) -> Session {
        Session::new(vec![SavedWindow {
            tabs: vec![SavedTab {
                root: SavedNode::Pane(SavedPane {
                    id: 4,
                    scrollback: Some("seen\r\n".into()),
                    host: host.map(str::to_owned),
                    remote: Some(1001),
                    ..SavedPane::default()
                }),
                focused: 4,
                zoomed: false,
            }],
            current_tab: 0,
            width: 0,
            height: 0,
            dropdown: false,
        }])
    }

    struct There {
        _dir: tempfile::TempDir,
        places: Places,
    }

    fn there() -> There {
        let dir = tempfile::tempdir().unwrap();
        let places = Places {
            locks: dir.path().join("run"),
            default: dir.path().join("state/session.json"),
        };
        There { _dir: dir, places }
    }

    /// A channel to `places`, served on a thread of its own.
    fn channel(places: &Places) -> UnixStream {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let places = places.clone();
        thread::spawn(move || serve(theirs, &places));
        ours
    }

    #[test]
    fn a_session_there_is_opened_kept_and_let_go() {
        let there = there();
        let p = &there.places;
        crate::save(&crate::named_beside(&p.default, "work"), &one_pane(None)).unwrap();

        let (mut work, session) = hold(channel(p), Some("work")).unwrap();
        assert_eq!(work.name, "work");
        assert_eq!(session, Some(one_pane(None)), "with its scrollback");
        assert!(crate::is_open_in(&p.locks, "work"), "held there");
        let e = hold(channel(p), Some("work")).err().unwrap();
        assert_eq!(e.to_string(), "another Hof has the session work open");

        let listed = list(channel(p)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].name.as_str(), listed[0].open, listed[0].held),
            ("work", true, Some((1, 1)))
        );

        work.save(&one_pane(Some("laptop"))).unwrap();
        let file = crate::named_beside(&p.default, "work");
        assert_eq!(
            crate::load(&file).unwrap(),
            Some(one_pane(Some("laptop"))),
            "kept there"
        );
        drop(work);
        // The server sees the channel end, and lets the session go.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while crate::is_open_in(&p.locks, "work") {
            assert!(std::time::Instant::now() < deadline, "let go");
            thread::sleep(std::time::Duration::from_millis(20));
        }

        let (mut work, _) = hold(channel(p), None).unwrap();
        assert_eq!(work.name, "work", "the last saved, free");
        work.forget().unwrap();
        assert!(list(channel(p)).unwrap().is_empty(), "forgotten there");
    }

    #[test]
    fn a_name_not_saved_there_starts_a_session() {
        let there = there();
        let (work, session) = hold(channel(&there.places), Some("new")).unwrap();
        assert_eq!((work.name.as_str(), session), ("new", None));
        let e = hold(channel(&there.places), Some("../up")).err().unwrap();
        assert!(e.to_string().contains("no session name"), "{e}");
    }

    #[test]
    fn a_plain_hof_elsewhere_keeps_its_session_or_the_next_free() {
        let there = there();
        let (work, _) = hold_free(channel(&there.places), "work").unwrap();
        let (beside, session) = hold_free(channel(&there.places), "work").unwrap();
        assert_eq!(
            (work.name.as_str(), beside.name.as_str(), session),
            ("work", "default", None)
        );
    }

    #[test]
    fn nothing_is_kept_on_a_channel_that_holds_no_session() {
        let there = there();
        let mut lines = Lines::new(channel(&there.places));
        let e = ask(&mut lines, &Ask::Forget).unwrap_err();
        assert_eq!(e.to_string(), "no session is held");
        let e = hold(channel(&there.places), None).err().unwrap();
        assert_eq!(e.to_string(), "no saved session is free to attach to");
    }

    #[test]
    fn an_agent_without_sessions_is_said_to_be_old() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        drop(theirs);
        let e = list(ours).unwrap_err();
        assert!(e.to_string().contains("update Hof there"), "{e}");
    }
}
