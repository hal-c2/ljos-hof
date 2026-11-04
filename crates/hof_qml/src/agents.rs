//! Which AI agent a terminal runs, if any, and how it is: working, waiting
//! for the user, or idle.
//!
//! Hof tells by three means, the first that knows winning:
//!
//! 1. **What the agent said** of itself, with `hof ctl agent-state` or the
//!    escape sequence [`hof_vt::AGENT_OSC`]. It holds while the program
//!    that was in the foreground when it was said still is.
//! 2. **A table** (`[agents.<name>]`) whose process is the one in the
//!    foreground, and the text on the bottom rows of the screen.
//! 3. Otherwise there is no agent.
//!
//! A terminal looks again a moment after its screen changes, at most
//! every [`LOOK_AFTER`], so an agent drawing as fast as it can costs a few
//! looks a second and a quiet terminal none.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use hof_vt::Process;

/// How long after a change a terminal looks; changes meanwhile wait for
/// the same look.
pub(crate) const LOOK_AFTER: Duration = Duration::from_millis(300);

/// How many rows, from the last that has text, a table's text is looked
/// for on: an agent's prompt sits at the bottom, and what it wrote above
/// may quote anything.
const BOTTOM_ROWS: usize = 15;

/// One `[agents.<name>]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    /// The program names that are the agent.
    pub processes: Vec<String>,
    pub waiting: Vec<String>,
    pub working: Vec<String>,
}

static TABLES: Mutex<Vec<Table>> = Mutex::new(Vec::new());

/// The tables in effect, from the config.
pub fn set_tables(tables: Vec<Table>) {
    *TABLES.lock().unwrap_or_else(PoisonError::into_inner) = tables;
}

/// How an agent is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// There is none.
    #[default]
    None,
    Working,
    /// It wants an answer.
    Waiting,
    /// It is done, and has nothing to ask.
    Idle,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::None => "none",
            State::Working => "working",
            State::Waiting => "waiting",
            State::Idle => "idle",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "none" => State::None,
            "working" => State::Working,
            "waiting" => State::Waiting,
            "idle" => State::Idle,
            _ => return None,
        })
    }
}

/// What a terminal's agent said of itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Said {
    /// Its name, if it gave one.
    pub name: String,
    pub state: State,
    /// The foreground process when it said so; `None` when there is no
    /// knowing, as for a program on another machine.
    pub group: Option<u32>,
}

/// A terminal's watch on its agent.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    /// A look is coming.
    pub due: Option<ljos::prelude::TimerId>,
    pub said: Option<Said>,
}

/// The agent in a terminal whose foreground is `fg` and whose screen is
/// `screen` (asked for only when a table needs it), with what it `said`:
/// its name, empty for none, and its state.
pub(crate) fn judge(
    tables: &[Table],
    fg: Option<&Process>,
    said: Option<&Said>,
    screen: impl FnOnce() -> String,
) -> (String, State) {
    let table = fg.and_then(|p| tables.iter().find(|t| t.processes.iter().any(|n| p.is(n))));
    if let Some(said) = said.filter(|s| s.group == fg.map(|p| p.pid)) {
        if said.state == State::None {
            return (String::new(), State::None);
        }
        let name = match (&said.name, table, fg) {
            (n, _, _) if !n.is_empty() => n.clone(),
            (_, Some(t), _) => t.name.clone(),
            (_, None, Some(p)) => p.name.clone(),
            (_, None, None) => "agent".to_owned(),
        };
        return (name, said.state);
    }
    let Some(table) = table else {
        return (String::new(), State::None);
    };
    let screen = screen();
    let rows = bottom(&screen, BOTTOM_ROWS);
    let shows = |texts: &[String]| texts.iter().any(|t| rows.iter().any(|r| r.contains(t)));
    let state = if shows(&table.waiting) {
        State::Waiting
    } else if shows(&table.working) {
        State::Working
    } else {
        State::Idle
    };
    (table.name.clone(), state)
}

/// [`judge`] with the tables in effect.
pub(crate) fn judge_now(
    fg: Option<&Process>,
    said: Option<&Said>,
    screen: impl FnOnce() -> String,
) -> (String, State) {
    let tables = TABLES.lock().unwrap_or_else(PoisonError::into_inner);
    judge(&tables, fg, said, screen)
}

/// The last `n` rows of `screen` up to the last with text on it.
fn bottom(screen: &str, n: usize) -> Vec<&str> {
    let rows: Vec<&str> = screen.lines().collect();
    let end = rows
        .iter()
        .rposition(|r| !r.trim().is_empty())
        .map_or(0, |i| i + 1);
    rows[end.saturating_sub(n)..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Table {
        Table {
            name: "claude".into(),
            processes: vec!["claude".into()],
            waiting: vec!["Do you want to".into()],
            working: vec!["esc to interrupt".into()],
        }
    }

    fn running(name: &str) -> Process {
        Process {
            pid: 42,
            name: name.into(),
            args: vec![name.into()],
        }
    }

    fn screen(bottom: &str) -> String {
        let mut s = "earlier: Do you want to see this?\n".to_owned();
        s.push_str(&"what it did since\n".repeat(20));
        s.push_str(bottom);
        s.push_str("\n\n\n");
        s
    }

    #[test]
    fn a_table_s_agent_in_the_foreground_is_told_by_its_screen() {
        let tables = [claude()];
        let fg = running("claude");
        let judged = |text: &str| judge(&tables, Some(&fg), None, || screen(text));
        assert_eq!(judged("✻ Thinking… (esc to interrupt)").1, State::Working);
        assert_eq!(judged("Do you want to proceed?").1, State::Waiting);
        assert_eq!(judged("> ").1, State::Idle);
        assert_eq!(judged("> ").0, "claude");
    }

    #[test]
    fn text_far_above_the_bottom_rows_does_not_count() {
        let tables = [claude()];
        let fg = running("claude");
        let (_, state) = judge(&tables, Some(&fg), None, || screen("> "));
        assert_eq!(state, State::Idle, "the old question scrolled up");
    }

    #[test]
    fn a_shell_in_the_foreground_has_no_agent_and_its_screen_is_not_read() {
        let tables = [claude()];
        let fg = running("bash");
        let judged = judge(&tables, Some(&fg), None, || panic!("read the screen"));
        assert_eq!(judged, (String::new(), State::None));
    }

    #[test]
    fn what_an_agent_says_wins_while_it_is_in_the_foreground() {
        let tables = [claude()];
        let fg = running("claude");
        let said = Said {
            name: String::new(),
            state: State::Waiting,
            group: Some(42),
        };
        let judged = judge(&tables, Some(&fg), Some(&said), || {
            screen("esc to interrupt")
        });
        assert_eq!(judged, ("claude".into(), State::Waiting));
    }

    #[test]
    fn what_was_said_by_a_program_that_has_gone_counts_no_more() {
        let tables = [claude()];
        let shell = running("bash");
        let said = Said {
            name: "claude".into(),
            state: State::Working,
            group: Some(7),
        };
        let judged = judge(&tables, Some(&shell), Some(&said), String::new);
        assert_eq!(judged, (String::new(), State::None));
    }

    #[test]
    fn an_agent_with_no_table_is_named_by_what_it_said_or_by_its_process() {
        let fg = running("goose");
        let said = |name: &str| Said {
            name: name.into(),
            state: State::Idle,
            group: Some(42),
        };
        assert_eq!(
            judge(&[], Some(&fg), Some(&said("Goose")), String::new).0,
            "Goose"
        );
        assert_eq!(
            judge(&[], Some(&fg), Some(&said("")), String::new).0,
            "goose"
        );
    }

    #[test]
    fn with_no_foreground_to_know_what_was_said_holds() {
        let said = Said {
            name: "claude".into(),
            state: State::Waiting,
            group: None,
        };
        assert_eq!(
            judge(&[claude()], None, Some(&said), String::new),
            ("claude".into(), State::Waiting)
        );
    }

    #[test]
    fn saying_none_is_no_agent() {
        let fg = running("claude");
        let said = Said {
            name: "claude".into(),
            state: State::None,
            group: Some(42),
        };
        let judged = judge(&[claude()], Some(&fg), Some(&said), || {
            screen("esc to interrupt")
        });
        assert_eq!(judged, (String::new(), State::None));
    }

    #[test]
    fn states_go_by_their_names() {
        for s in [State::None, State::Working, State::Waiting, State::Idle] {
            assert_eq!(State::parse(s.name()), Some(s));
        }
        assert_eq!(State::parse("busy"), None);
    }
}
