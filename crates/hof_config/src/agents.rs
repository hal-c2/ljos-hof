//! `[agents.<name>]`: how Hof tells, from the outside, that an AI agent runs
//! in a pane and how it is doing.
//!
//! A table names the processes that are the agent, and the text on the
//! bottom rows of its screen that means it is waiting for the user or
//! working. Hof ships tables for the agents it knows; the user's, and then
//! the plugins', replace them by name. An agent that says how it is, with
//! `hof ctl agent-state` or Hof's escape sequence, needs no text here.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One agent's table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Agent {
    /// The program names that are this agent, as the process calls itself
    /// or as its file is named; empty means the table's name.
    pub process: Vec<String>,
    /// Text that, on the bottom rows of the screen, means the agent is
    /// waiting for an answer: a question, a permission prompt.
    pub waiting: Vec<String>,
    /// Text that there means it is working. With neither, it is idle.
    pub working: Vec<String>,
}

impl Agent {
    /// The program names that are this agent, the table being `name`.
    pub fn processes<'a>(&'a self, name: &'a str) -> Vec<&'a str> {
        if self.process.is_empty() {
            vec![name]
        } else {
            self.process.iter().map(String::as_str).collect()
        }
    }
}

/// The tables Hof ships. Agents change what they show from one release to
/// the next, so these are a best guess; a table in the config replaces one
/// here, and `hof ctl agent-state` from the agent's own hooks needs none.
fn shipped() -> BTreeMap<String, Agent> {
    let table = |process: &[&str], waiting: &[&str], working: &[&str]| Agent {
        process: process.iter().map(|&s| s.into()).collect(),
        waiting: waiting.iter().map(|&s| s.into()).collect(),
        working: working.iter().map(|&s| s.into()).collect(),
    };
    [
        (
            "claude",
            table(
                &["claude"],
                &["Do you want to", "❯ 1. Yes"],
                &["esc to interrupt"],
            ),
        ),
        (
            "codex",
            table(
                &["codex"],
                &["Would you like to", "Allow command?"],
                &["esc to interrupt"],
            ),
        ),
        (
            "gemini",
            table(
                &["gemini"],
                &[
                    "Allow execution",
                    "Apply this change?",
                    "Do you want to proceed",
                ],
                &["esc to cancel"],
            ),
        ),
        ("aider", table(&["aider"], &["(Y)es/(N)o"], &[])),
        (
            "opencode",
            table(&["opencode"], &[], &["esc to interrupt", "esc interrupt"]),
        ),
    ]
    .into_iter()
    .map(|(name, agent)| (name.to_owned(), agent))
    .collect()
}

/// Every table in effect: Hof's, with `mine` replacing them by name.
pub(crate) fn in_effect(mine: &BTreeMap<String, Agent>) -> BTreeMap<String, Agent> {
    let mut all = shipped();
    all.extend(mine.iter().map(|(n, a)| (n.clone(), a.clone())));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_with_no_processes_is_the_program_of_its_name() {
        let agent = Agent::default();
        assert_eq!(agent.processes("goose"), ["goose"]);
    }

    #[test]
    fn the_user_s_table_replaces_the_shipped_one_of_its_name() {
        let mine = BTreeMap::from([(
            "claude".to_owned(),
            Agent {
                waiting: vec!["?".into()],
                ..Agent::default()
            },
        )]);
        let all = in_effect(&mine);
        assert_eq!(all["claude"].waiting, ["?"]);
        assert!(all["claude"].working.is_empty());
        assert!(all.contains_key("codex"), "the rest stay");
    }
}
