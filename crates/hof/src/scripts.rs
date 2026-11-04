//! Commands and hooks from the config, as the window runs them: what the
//! QML reads of them, and the programs they start outside any pane.

use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use hof_config::{Config, Hooks, Plugin, Program};
use serde_json::{Map, Value, json};

/// `[commands]` as JSON for the QML: by name, `{run, send, in,
/// description, pluginDir}`, `run` the program and its arguments.
pub(crate) fn commands(config: &Config) -> String {
    let commands: Map<String, Value> = config
        .commands
        .iter()
        .map(|(name, c)| {
            let command = json!({
                "run": c.run.as_ref().map(Program::argv),
                "send": c.send,
                "in": c.place.name(),
                "description": c.description,
                "pluginDir": plugin_dir(c.plugin.as_ref()),
            });
            (name.clone(), command)
        })
        .collect();
    Value::Object(commands).to_string()
}

/// `[hooks]` as JSON for the QML: by event, a list of `{run, pluginDir}`.
/// Events with no hooks are left out, so the QML can tell at a glance.
pub(crate) fn hooks(hooks: &Hooks) -> String {
    let events: Map<String, Value> = Hooks::EVENTS
        .iter()
        .filter(|event| !hooks.of(event).is_empty())
        .map(|event| {
            let runs = hooks
                .of(event)
                .iter()
                .map(|h| {
                    json!({
                        "run": ["sh", "-c", h.run],
                        "pluginDir": plugin_dir(h.plugin.as_ref()),
                    })
                })
                .collect();
            ((*event).to_owned(), Value::Array(runs))
        })
        .collect();
    Value::Object(events).to_string()
}

fn plugin_dir(plugin: Option<&Plugin>) -> String {
    plugin
        .map(|p| p.dir.display().to_string())
        .unwrap_or_default()
}

/// Starts `command` with Hof's environment and `environment` (`NAME=value`
/// each) on top, in `directory` when it is one, and forgets it: its output
/// is Hof's, and a thread reaps it when it ends.
pub(crate) fn spawn(command: &[String], environment: &[String], directory: &str) -> io::Result<()> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no program to run"))?;
    let mut child = Command::new(program);
    child
        .args(arguments)
        .envs(environment.iter().filter_map(|v| variable(v)))
        .stdin(Stdio::null())
        // Its own process group: a ctrl+c for Hof is not one for it.
        .process_group(0);
    if !directory.is_empty() && Path::new(directory).is_dir() {
        child.current_dir(directory);
    }
    let mut child = child.spawn()?;
    std::thread::Builder::new()
        .name("hof-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

/// `NAME=value` as the two; `None` when there is no name.
fn variable(text: &str) -> Option<(OsString, OsString)> {
    let (name, value) = text.split_once('=')?;
    (!name.is_empty()).then(|| (name.into(), value.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_for_the_qml_are_shell_runs_and_only_for_events_that_have_some() {
        let loaded = hof_config::parse("[hooks]\nbell = \"echo ding\"\n").unwrap();
        let json: Value = serde_json::from_str(&hooks(&loaded.config.hooks)).unwrap();
        assert_eq!(
            json,
            json!({ "bell": [{ "run": ["sh", "-c", "echo ding"], "pluginDir": "" }] })
        );
    }

    #[test]
    fn a_command_for_the_qml_says_where_it_runs() {
        let loaded =
            hof_config::parse("[commands.top]\nrun = [\"htop\"]\nin = \"split-down\"\n").unwrap();
        let json: Value = serde_json::from_str(&commands(&loaded.config)).unwrap();
        assert_eq!(json["top"]["run"], json!(["htop"]));
        assert_eq!(json["top"]["in"], "split-down");
        assert_eq!(json["top"]["send"], Value::Null);
    }

    #[test]
    fn a_variable_without_a_name_is_dropped() {
        assert_eq!(variable("=x"), None);
        assert_eq!(variable("A=b=c"), Some(("A".into(), "b=c".into())));
    }
}
