use std::fs;
use std::path::{Path, PathBuf};

use hof_session::{
    Error, SavedAxis, SavedNode, SavedPane, SavedTab, SavedWindow, Session, VERSION, clear, load,
    save,
};

fn pane(id: u32, dir: &str) -> SavedNode {
    SavedNode::Pane(SavedPane {
        id,
        working_directory: Some(PathBuf::from(dir)),
        command: Vec::new(),
        title: None,
        scrollback: None,
        host: None,
        remote: None,
    })
}

fn split(axis: SavedAxis, ratio: f64, first: SavedNode, second: SavedNode) -> SavedNode {
    SavedNode::Split {
        axis,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

fn window(tabs: Vec<SavedTab>) -> SavedWindow {
    SavedWindow {
        tabs,
        current_tab: 0,
        width: 1200,
        height: 800,
        dropdown: false,
    }
}

fn tab(root: SavedNode, focused: u32) -> SavedTab {
    SavedTab {
        root,
        focused,
        zoomed: false,
    }
}

fn with_scrollback(mut node: SavedNode, text: &str) -> SavedNode {
    if let SavedNode::Pane(p) = &mut node {
        p.scrollback = Some(text.to_owned());
    }
    node
}

/// Two windows, the first with a split tab and a second tab, the second a
/// dropdown, with a pane that ran a command and has scrollback.
fn sample() -> Session {
    let mut htop = pane(2, "/tmp");
    if let SavedNode::Pane(p) = &mut htop {
        p.command = vec!["htop".into(), "-d".into(), "10".into()];
        p.title = Some("htop".into());
        p.scrollback = Some("\x1b[1mbold\x1b[0m\r\nline two\r\n".into());
    }
    Session::new(vec![
        SavedWindow {
            current_tab: 1,
            ..window(vec![
                SavedTab {
                    zoomed: true,
                    ..tab(
                        split(
                            SavedAxis::Horizontal,
                            0.3,
                            pane(1, "/home/me"),
                            split(SavedAxis::Vertical, 0.6, htop, pane(3, "/srv")),
                        ),
                        2,
                    )
                },
                tab(pane(4, "/"), 4),
            ])
        },
        SavedWindow {
            dropdown: true,
            ..window(vec![tab(
                with_scrollback(pane(1, "/home/me"), "$ ls\r\n"),
                1,
            )])
        },
    ])
}

fn session_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("hof").join("session.json")
}

fn scrollback_files(session: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(session.parent().unwrap().join("scrollback"))
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn write_json(path: &Path, json: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, json).unwrap();
}

fn one_tab_json(tab: &str) -> String {
    format!(r#"{{"version": 1, "windows": [{{"tabs": [{tab}]}}]}}"#)
}

#[test]
fn a_saved_session_reads_back_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    assert_eq!(load(&path).unwrap(), Some(sample()));
}

#[test]
fn a_missing_session_file_loads_as_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load(&session_path(&dir)).unwrap(), None);
}

#[test]
fn saving_again_replaces_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    let smaller = Session::new(vec![window(vec![tab(pane(7, "/opt"), 7)])]);
    save(&path, &smaller).unwrap();
    assert_eq!(load(&path).unwrap(), Some(smaller));
}

#[test]
fn the_json_is_readable_and_holds_no_scrollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    let json = fs::read_to_string(&path).unwrap();
    assert!(json.contains("\n  \"windows\""), "pretty-printed: {json}");
    assert!(json.contains("\"working_directory\": \"/srv\""));
    assert!(!json.contains("line two"));
    assert!(!json.contains("scrollback"));
}

#[test]
fn scrollback_is_kept_beside_the_json_one_file_per_pane() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    assert_eq!(scrollback_files(&path), ["0-2.vt", "1-1.vt"]);
    let written = fs::read_to_string(path.parent().unwrap().join("scrollback/0-2.vt")).unwrap();
    assert_eq!(written, "\x1b[1mbold\x1b[0m\r\nline two\r\n");
}

#[test]
fn a_session_without_scrollback_makes_no_scrollback_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(
        &path,
        &Session::new(vec![window(vec![tab(pane(1, "/"), 1)])]),
    )
    .unwrap();
    assert!(!path.parent().unwrap().join("scrollback").exists());
}

#[test]
fn a_pane_whose_scrollback_file_is_gone_loads_without_scrollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    fs::remove_file(path.parent().unwrap().join("scrollback/0-2.vt")).unwrap();
    let loaded = load(&path).unwrap().unwrap();
    let panes = loaded.windows[0].tabs[0].panes();
    assert_eq!(panes[1].id, 2);
    assert_eq!(panes[1].scrollback, None);
    assert_eq!(panes[1].command, ["htop", "-d", "10"]);
}

#[test]
fn scrollback_of_panes_that_no_longer_exist_is_removed_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    let mut fewer = sample();
    fewer.windows.pop();
    save(&path, &fewer).unwrap();
    assert_eq!(scrollback_files(&path), ["0-2.vt"]);
}

#[test]
fn a_pane_saved_without_scrollback_does_not_get_its_old_scrollback_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    let mut forgotten = sample();
    forgotten.windows[1].tabs[0] = tab(pane(1, "/home/me"), 1);
    save(&path, &forgotten).unwrap();
    let loaded = load(&path).unwrap().unwrap();
    assert_eq!(loaded.windows[1].tabs[0].panes()[0].scrollback, None);
}

#[test]
fn files_in_the_scrollback_directory_that_are_not_scrollback_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    let notes = path.parent().unwrap().join("scrollback/notes.txt");
    fs::write(&notes, "mine").unwrap();
    save(&path, &Session::default()).unwrap();
    assert!(notes.exists());
}

#[test]
fn saving_leaves_no_temporary_files_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    save(&path, &sample()).unwrap();
    let mut names: Vec<String> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["scrollback", "session.json"]);
    assert_eq!(scrollback_files(&path), ["0-2.vt", "1-1.vt"]);
}

#[cfg(unix)]
#[test]
fn the_session_and_its_scrollback_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state/hof/session.json");
    save(&path, &sample()).unwrap();
    let hof = path.parent().unwrap();
    assert_eq!(mode(&dir.path().join("state")), 0o700);
    assert_eq!(mode(hof), 0o700);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&hof.join("scrollback")), 0o700);
    assert_eq!(mode(&hof.join("scrollback/0-2.vt")), 0o600);
}

#[cfg(unix)]
#[test]
fn a_working_directory_that_is_not_utf8_survives_a_save() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let odd = PathBuf::from(OsString::from_vec(b"/home/me/caf\xe9".to_vec()));
    let mut session = Session::new(vec![window(vec![tab(pane(1, "/"), 1)])]);
    if let SavedNode::Pane(p) = &mut session.windows[0].tabs[0].root {
        p.working_directory = Some(odd.clone());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &session).unwrap();
    let loaded = load(&path).unwrap().unwrap();
    assert_eq!(
        loaded.windows[0].tabs[0].panes()[0].working_directory,
        Some(odd)
    );
}

#[test]
fn a_session_from_a_newer_hof_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    // A newer format may look nothing like this one.
    write_json(&path, r#"{"version": 99, "everything": "different"}"#);
    let err = load(&path).unwrap_err();
    assert!(
        matches!(err, Error::FutureVersion { found: 99, .. }),
        "{err}"
    );
    assert!(err.to_string().contains(&path.display().to_string()));
}

#[test]
fn corrupt_json_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    write_json(&path, r#"{"version": 1, "windows": [{"tabs": [{"#);
    let err = load(&path).unwrap_err();
    assert!(matches!(err, Error::Corrupt { .. }), "{err}");
    assert!(err.to_string().contains(&path.display().to_string()));
}

#[test]
fn a_tab_without_a_pane_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    write_json(&path, &one_tab_json(r#"{"focused": 1}"#));
    assert!(matches!(load(&path), Err(Error::Corrupt { .. })));
}

#[test]
fn split_ratios_are_clamped_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    let leaf = |id| format!(r#"{{"pane": {{"id": {id}}}}}"#);
    let tree = format!(
        r#"{{"split": {{"axis": "horizontal", "ratio": 1.5, "first": {}, "second": {{"split": {{"axis": "vertical", "ratio": -2, "first": {}, "second": {}}}}}}}}}"#,
        leaf(1),
        leaf(2),
        leaf(3)
    );
    write_json(
        &path,
        &one_tab_json(&format!(r#"{{"root": {tree}, "focused": 1}}"#)),
    );
    let loaded = load(&path).unwrap().unwrap();
    let SavedNode::Split { ratio, second, .. } = &loaded.windows[0].tabs[0].root else {
        panic!("not a split");
    };
    assert_eq!(*ratio, 0.95);
    let SavedNode::Split { ratio, .. } = second.as_ref() else {
        panic!("not a split");
    };
    assert_eq!(*ratio, 0.05);
}

#[test]
fn a_ratio_that_is_not_a_number_is_saved_as_an_even_split() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    let session = Session::new(vec![window(vec![tab(
        split(SavedAxis::Vertical, f64::NAN, pane(1, "/"), pane(2, "/")),
        1,
    )])]);
    save(&path, &session).unwrap();
    let loaded = load(&path).unwrap().unwrap();
    assert!(matches!(
        loaded.windows[0].tabs[0].root,
        SavedNode::Split { ratio: 0.5, .. }
    ));
}

#[test]
fn a_current_tab_past_the_end_becomes_the_last_tab() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    let mut session = sample();
    session.windows[0].current_tab = 9;
    save(&path, &session).unwrap();
    assert_eq!(load(&path).unwrap().unwrap().windows[0].current_tab, 1);
}

#[test]
fn focus_on_a_pane_not_in_the_tab_moves_to_its_first_pane() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    let mut session = sample();
    session.windows[0].tabs[0].focused = 42;
    save(&path, &session).unwrap();
    assert_eq!(load(&path).unwrap().unwrap().windows[0].tabs[0].focused, 1);
}

#[test]
fn windows_without_tabs_are_dropped_and_keep_their_neighbours_scrollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    let mut session = sample();
    session.windows.insert(0, window(Vec::new()));
    save(&path, &session).unwrap();
    assert_eq!(load(&path).unwrap(), Some(sample()));
}

#[test]
fn missing_optional_fields_take_their_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    write_json(
        &path,
        &one_tab_json(r#"{"root": {"pane": {"id": 3}}, "focused": 3, "colour": "blue"}"#),
    );
    let loaded = load(&path).unwrap().unwrap();
    let w = &loaded.windows[0];
    assert_eq!(
        (w.current_tab, w.width, w.height, w.dropdown),
        (0, 0, 0, false)
    );
    assert!(!w.tabs[0].zoomed);
    assert_eq!(
        *w.tabs[0].panes()[0],
        SavedPane {
            id: 3,
            ..SavedPane::default()
        }
    );
}

#[test]
fn a_pane_id_twice_in_one_window_is_an_error_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    write_json(
        &path,
        r#"{"version": 1, "windows": [{"tabs": [
            {"root": {"pane": {"id": 5}}, "focused": 5},
            {"root": {"pane": {"id": 5}}, "focused": 5}
        ]}]}"#,
    );
    let err = load(&path).unwrap_err();
    assert!(
        matches!(
            err,
            Error::DuplicatePane {
                window: 0,
                id: 5,
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains(&path.display().to_string()));
}

#[test]
fn saving_a_pane_id_twice_in_one_window_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    let session = Session::new(vec![window(vec![tab(
        split(SavedAxis::Horizontal, 0.5, pane(1, "/"), pane(1, "/tmp")),
        1,
    )])]);
    assert!(matches!(
        save(&path, &session),
        Err(Error::DuplicatePane { id: 1, .. })
    ));
    assert!(!path.exists());
}

#[test]
fn the_same_pane_id_in_two_windows_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    // Both windows of the sample have a pane 1, each with its own scrollback.
    save(&path, &sample()).unwrap();
    let loaded = load(&path).unwrap().unwrap();
    assert_eq!(loaded.windows[0].tabs[0].panes()[0].scrollback, None);
    assert_eq!(
        loaded.windows[1].tabs[0].panes()[0].scrollback.as_deref(),
        Some("$ ls\r\n")
    );
}

#[test]
fn the_caller_s_version_is_not_what_gets_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    let mut session = sample();
    session.version = 99;
    save(&path, &session).unwrap();
    assert_eq!(load(&path).unwrap().unwrap().version, VERSION);
}

#[test]
fn clear_forgets_the_session_and_its_scrollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_path(&dir);
    save(&path, &sample()).unwrap();
    clear(&path).unwrap();
    assert_eq!(load(&path).unwrap(), None);
    assert!(!path.parent().unwrap().join("scrollback").exists());
}

#[test]
fn clearing_when_nothing_was_saved_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    clear(&session_path(&dir)).unwrap();
}
