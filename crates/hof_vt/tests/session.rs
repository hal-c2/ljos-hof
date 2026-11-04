use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hof_vt::{
    Attrs, Changes, CursorShape, Event, Found, Grid, Highlight, Key, KeyInput, Mods, Options,
    PasteError, PromptMark, Rgb, Scroll, Selecting, Session, Size, Spawn, Theme,
};

fn detached(cols: u16, rows: u16) -> (Session, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let options = Options {
        size: Size::new(cols, rows),
        ..Options::default()
    };
    let session = Session::detached(options, move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    (session, wakes)
}

fn line_text(grid: &Grid, y: usize) -> String {
    let line = &grid.lines[y];
    line.cells
        .iter()
        .filter(|c| c.width > 0)
        .map(|c| {
            let t = line.cell_text(c);
            if t.is_empty() { " " } else { t }
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn output_becomes_lines_of_cells() {
    let (session, wakes) = detached(20, 4);
    session.feed(b"hello\r\n\x1b[1;31mred\x1b[0m world");
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert!(session.take_changes().contains(Changes::SCREEN));

    let mut grid = Grid::new().unwrap();
    let damage = grid.update(&session).unwrap();
    assert!(damage.full);
    assert_eq!((grid.cols, grid.rows()), (20, 4));
    assert_eq!(line_text(&grid, 0), "hello");
    assert_eq!(line_text(&grid, 1), "red world");

    let red = grid.lines[1].cells[0];
    assert!(red.attrs.contains(Attrs::BOLD));
    assert_eq!(red.fg, grid.colors.palette[1]);
    assert_eq!(grid.cursor.at, Some((9, 1)));
}

#[test]
fn only_changed_lines_are_rebuilt() {
    let (session, _) = detached(20, 4);
    session.feed(b"one\r\ntwo\r\nthree");
    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    let before: Vec<u64> = grid.lines.iter().map(|l| l.generation).collect();

    assert!(
        grid.update(&session).unwrap().is_empty(),
        "nothing changed, nothing to do"
    );

    session.feed(b"\x1b[1;1Hune");
    let damage = grid.update(&session).unwrap();
    assert!(!damage.full);
    assert_eq!(line_text(&grid, 0), "une");
    assert_ne!(grid.lines[0].generation, before[0]);
    assert!(damage.cursor);
    assert_eq!(damage.lines, 1);
    assert_eq!(
        grid.lines[1].generation, before[1],
        "untouched lines keep their generation"
    );
    assert_eq!(
        grid.lines[2].generation, before[2],
        "the cursor leaving a line doesn't change it"
    );
}

#[test]
fn wide_characters_take_two_columns() {
    let (session, _) = detached(10, 2);
    session.feed("a世b".as_bytes());
    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    let cells = &grid.lines[0].cells;
    assert_eq!(cells[1].width, 2);
    assert_eq!(grid.lines[0].cell_text(&cells[1]), "世");
    assert_eq!(cells[2].width, 0, "the spacer draws nothing");
    assert_eq!(grid.lines[0].cell_text(&cells[3]), "b");
}

#[test]
fn grapheme_clusters_stay_in_one_cell() {
    let (session, _) = detached(10, 2);
    // Enable grapheme clustering (mode 2027), then a flag: two regional indicators.
    session.feed(b"\x1b[?2027h");
    session.feed("🇮🇸x".as_bytes());
    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    let line = &grid.lines[0];
    assert_eq!(line.cell_text(&line.cells[0]), "🇮🇸");
}

#[test]
fn inverse_swaps_colours() {
    let (session, _) = detached(10, 2);
    let theme = Theme {
        foreground: Some(Rgb::new(200, 200, 200)),
        background: Some(Rgb::new(10, 10, 10)),
        ..Theme::default()
    };
    session.set_theme(&theme).unwrap();
    session.feed(b"\x1b[7mX");
    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    let x = grid.lines[0].cells[0];
    assert_eq!(x.fg, Rgb::new(10, 10, 10));
    assert_eq!(x.bg, Some(Rgb::new(200, 200, 200)));
}

#[test]
fn query_replies_go_back_to_the_program() {
    let (session, _) = detached(10, 2);
    // Cursor position report.
    session.feed(b"ab\x1b[6n");
    assert_eq!(session.take_input(), b"\x1b[1;3R");
    // Primary device attributes answer with what we claim to be.
    session.feed(b"\x1b[c");
    let reply = String::from_utf8(session.take_input()).unwrap();
    assert!(reply.starts_with("\x1b[?62;"), "got {reply:?}");
}

#[test]
fn a_terminal_that_does_not_answer_still_types_and_pastes() {
    let options = Options {
        size: Size::new(10, 2),
        replies: false,
        ..Options::default()
    };
    let session = Session::detached(options, || {}).unwrap();
    session.feed(b"\x1b[6n\x1b[c");
    assert_eq!(session.take_input(), b"");
    session.paste("hi", false).unwrap();
    session.write(b"!");
    assert_eq!(session.take_input(), b"hi!");
    // The paste let only itself out.
    session.feed(b"\x1b[6n");
    assert_eq!(session.take_input(), b"");
}

#[test]
fn keys_follow_the_programs_modes() {
    let (session, _) = detached(10, 2);
    assert!(session.key(&KeyInput::press(Key::ArrowUp)).unwrap());
    assert_eq!(session.take_input(), b"\x1b[A");

    // Application cursor keys (DECCKM).
    session.feed(b"\x1b[?1h");
    session.key(&KeyInput::press(Key::ArrowUp)).unwrap();
    assert_eq!(session.take_input(), b"\x1bOA");

    session
        .key(&KeyInput::press(Key::C).with_mods(Mods::CTRL).with_text("c"))
        .unwrap();
    assert_eq!(session.take_input(), b"\x03");

    session
        .key(&KeyInput::press(Key::A).with_text("a"))
        .unwrap();
    assert_eq!(session.take_input(), b"a");
}

#[test]
fn pasting_respects_bracketed_paste() {
    let (session, _) = detached(10, 2);
    assert!(matches!(
        session.paste("rm -rf ~\n", false),
        Err(PasteError::Unsafe)
    ));
    assert!(session.take_input().is_empty());

    session.feed(b"\x1b[?2004h");
    session.paste("echo hi\n", false).unwrap();
    assert_eq!(session.take_input(), b"\x1b[200~echo hi\n\x1b[201~");
}

#[test]
fn selecting_and_copying() {
    let (session, _) = detached(20, 3);
    session.feed(b"hello brave world\r\nsecond line");
    session.select_start(6, 0, Selecting::Words).unwrap();
    assert_eq!(session.selected_text().as_deref(), Some("brave"));

    session.select_extend(14, 0).unwrap();
    assert_eq!(session.selected_text().as_deref(), Some("brave world"));

    session.select_start(0, 0, Selecting::Cells).unwrap();
    session.select_extend(5, 1).unwrap();
    assert_eq!(
        session.selected_text().as_deref(),
        Some("hello brave world\nsecond")
    );

    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    assert!(grid.lines[1].cells[0].attrs.contains(Attrs::SELECTED));
    assert!(!grid.lines[1].cells[7].attrs.contains(Attrs::SELECTED));

    session.clear_selection();
    assert!(!session.has_selection());
}

#[test]
fn scrollback_and_the_view() {
    let (session, _) = detached(10, 3);
    for i in 0..10 {
        session.feed(format!("line {i}\r\n").as_bytes());
    }
    let bar = session.scrollbar();
    assert!(bar.at_bottom());
    assert_eq!(bar.visible, 3);

    session.scroll(Scroll::Top);
    assert!(!session.scrollbar().at_bottom());
    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    assert_eq!(line_text(&grid, 0), "line 0");

    session.scroll(Scroll::Bottom);
    assert!(session.scrollbar().at_bottom());
    assert!(session.text().contains("line 0\nline 1"));
}

#[test]
fn title_and_shell_integration() {
    let (session, _) = detached(20, 3);
    session.feed(b"\x1b]2;my title\x07");
    assert!(session.take_changes().contains(Changes::TITLE));
    assert_eq!(session.title(), "my title");

    session.feed(b"\x1b]7;file://host/tmp\x07");
    assert!(session.take_changes().contains(Changes::PWD));
    assert_eq!(session.pwd(), "file://host/tmp");

    session.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;3\x07");
    let marks: Vec<_> = session
        .take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Prompt(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(marks.first(), Some(&PromptMark::PromptStart));
    assert_eq!(
        marks.last(),
        Some(&PromptMark::CommandEnd { exit_code: Some(3) })
    );
}

#[test]
fn snapshot_restores_into_a_fresh_session() {
    let (session, _) = detached(20, 3);
    session.feed(b"\x1b[32mgreen\x1b[0m text\r\nnext");
    let snapshot = session.snapshot().unwrap();

    let (restored, _) = detached(20, 3);
    restored.feed(&snapshot);
    let (mut a, mut b) = (Grid::new().unwrap(), Grid::new().unwrap());
    a.update(&session).unwrap();
    b.update(&restored).unwrap();
    assert_eq!(line_text(&a, 0), line_text(&b, 0));
    assert_eq!(a.lines[0].cells[0].fg, b.lines[0].cells[0].fg);
    assert_eq!(a.cursor, b.cursor);
}

#[test]
fn synchronized_output_holds_the_wake() {
    let (session, wakes) = detached(10, 2);
    session.feed(b"\x1b[?2026h");
    session.take_changes();
    let before = wakes.load(Ordering::SeqCst);
    session.feed(b"frame");
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        before,
        "held: no wake mid-frame"
    );
    session.feed(b"\x1b[?2026l");
    assert_eq!(wakes.load(Ordering::SeqCst), before + 1);
}

/// Wait, without spinning, until the session wakes us or the deadline passes.
fn wait_for(
    session: &Session,
    rx: &std::sync::mpsc::Receiver<()>,
    mut done: impl FnMut(&Session) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(session) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "timed out; screen:\n{}", session.text());
        let _ = rx.recv_timeout(left);
        session.take_changes();
    }
}

#[test]
fn a_real_program_on_a_pty() {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let spawn = Spawn::command("/bin/sh", ["-c", "stty size; read line; echo got:$line"]);
    let session = Session::spawn(
        &spawn,
        Options {
            size: Size::new(42, 7),
            ..Options::default()
        },
        move || {
            let _ = tx.lock().unwrap().send(());
        },
    )
    .unwrap();
    assert!(session.pid().is_some());

    wait_for(&session, &rx, |s| s.text().contains("7 42"));
    session.write(b"hof\r");
    wait_for(&session, &rx, |s| s.text().contains("got:hof"));
    wait_for(&session, &rx, |s| !s.is_alive());
    assert!(session.take_events().contains(&Event::Exited(Some(0))));
}

#[test]
fn a_tap_sees_the_output_and_a_snapshot_says_how_much_it_shows() {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tap = seen.clone();
    let spawn = Spawn::command("/bin/sh", ["-c", "printf 'one two'; read _"]);
    let session = Session::spawn_tapped(
        &spawn,
        Options {
            size: Size::new(20, 4),
            ..Options::default()
        },
        move || {
            let _ = tx.lock().unwrap().send(());
        },
        Some(Box::new(move |bytes| {
            tap.lock().unwrap().extend_from_slice(bytes)
        })),
    )
    .unwrap();
    wait_for(&session, &rx, |s| s.text().contains("two"));
    assert_eq!(*seen.lock().unwrap(), b"one two");

    let (snapshot, count) = session
        .snapshot_with(|| seen.lock().unwrap().len())
        .unwrap();
    assert_eq!(count, 7);
    let (copy, _) = detached(20, 4);
    copy.feed(&snapshot);
    assert_eq!(copy.screen_text(), session.screen_text());
    session.write(b"\r");
}

#[test]
fn resizing_reaches_the_program() {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let spawn = Spawn::command("/bin/sh", ["-c", "read _; stty size; read _"]);
    let session = Session::spawn(&spawn, Options::default(), move || {
        let _ = tx.lock().unwrap().send(());
    })
    .unwrap();
    session.resize(Size::new(100, 30)).unwrap();
    session.write(b"\r");
    wait_for(&session, &rx, |s| s.text().contains("30 100"));
}

#[test]
fn finding_counts_every_match_and_starts_at_the_newest() {
    let (session, _) = detached(20, 3);
    session.feed(b"apple\r\nbanana\r\nApple pie\r\ncherry\r\n");
    let found = session.find("apple").unwrap();
    assert_eq!(
        found,
        Found {
            total: 2,
            current: Some(0)
        },
        "either case, newest first"
    );
}

#[test]
fn stepping_through_matches_goes_back_in_time_and_wraps() {
    let (session, _) = detached(20, 3);
    for i in 0..6 {
        session.feed(format!("hit {i}\r\n").as_bytes());
    }
    assert_eq!(session.find("hit").unwrap().current, Some(0));
    assert_eq!(session.find_older().unwrap().current, Some(1));
    assert_eq!(session.find_newer().unwrap().current, Some(0));
    assert_eq!(
        session.find_newer().unwrap().current,
        Some(5),
        "past the newest is the oldest"
    );
}

#[test]
fn the_current_match_is_scrolled_into_view_and_highlighted() {
    let (session, _) = detached(20, 3);
    session.feed(b"needle here\r\n");
    for i in 0..10 {
        session.feed(format!("line {i}\r\n").as_bytes());
    }
    session.find("needle").unwrap();
    assert!(!session.scrollbar().at_bottom(), "the match is in history");

    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    let row = grid.found.first().expect("the match is in view").row;
    assert_eq!(line_text(&grid, usize::from(row)), "needle here");
    assert_eq!(
        grid.found,
        vec![Highlight {
            row,
            start: 0,
            end: 5,
            current: true
        }]
    );
}

#[test]
fn only_the_current_match_is_marked_current() {
    let (session, _) = detached(20, 3);
    session.feed(b"ab ab ab");
    session.find("ab").unwrap();
    let highlights = session.highlights();
    assert_eq!(highlights.len(), 3);
    let current: Vec<_> = highlights.iter().filter(|h| h.current).collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].start, 6, "the newest is the last one written");
}

#[test]
fn a_match_that_wraps_is_highlighted_on_both_rows() {
    let (session, _) = detached(6, 3);
    session.feed(b"xxxxwrapped");
    session.find("wrap").unwrap();
    let rows: Vec<_> = session
        .highlights()
        .iter()
        .map(|h| (h.row, h.start, h.end))
        .collect();
    assert_eq!(rows, vec![(0, 4, 5), (1, 0, 1)]);
}

#[test]
fn new_output_is_found_too() {
    let (session, _) = detached(20, 3);
    session.feed(b"one match\r\n");
    session.find("match").unwrap();
    session.feed(b"another match\r\n");
    assert_eq!(session.found().map(|f| f.total), Some(2));
}

#[test]
fn ending_the_search_clears_the_highlights() {
    let (session, _) = detached(20, 3);
    session.feed(b"findme");
    session.find("findme").unwrap();
    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    assert_eq!(grid.found.len(), 1);

    session.end_find();
    let damage = grid.update(&session).unwrap();
    assert!(grid.found.is_empty());
    assert!(damage.found, "the view knows to draw again");
    assert_eq!(session.found(), None);
    assert_eq!(session.find_older().unwrap(), Found::default());
}

#[test]
fn nothing_found_is_zero_with_none_current() {
    let (session, _) = detached(20, 3);
    session.feed(b"hello");
    assert_eq!(session.find("absent").unwrap(), Found::default());
    assert!(session.highlights().is_empty());
}

#[test]
fn screen_text_is_the_screen_whatever_the_view_is_scrolled_to() {
    let (session, _) = detached(10, 3);
    for i in 0..8 {
        session.feed(format!("line {i}\r\n").as_bytes());
    }
    session.feed(b"last");
    session.scroll(Scroll::Top);
    assert_eq!(session.screen_text(), "line 6\nline 7\nlast");
}

/// A shell's prompt with OSC 133 marks: prompt, input, then what it ran.
fn prompt(session: &Session) {
    session.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07");
}

#[test]
fn a_marked_command_s_output_is_what_follows_its_line() {
    let (session, _) = detached(20, 4);
    session.feed(b"before\r\n");
    prompt(&session);
    session.mark_command().unwrap();
    session.feed(b"seq 3\r\n\x1b]133;C\x07");
    // Enough to push the command's line into the scrollback.
    for i in 1..=6 {
        session.feed(format!("{i}\r\n").as_bytes());
    }
    session.feed(b"\x1b]133;D;0\x07");
    assert_eq!(
        session.command_output().as_deref(),
        Some("1\n2\n3\n4\n5\n6")
    );
}

#[test]
fn a_command_that_prints_nothing_has_empty_output() {
    let (session, _) = detached(20, 4);
    prompt(&session);
    session.mark_command().unwrap();
    session.feed(b"true\r\n\x1b]133;C\x07\x1b]133;D;0\x07");
    assert_eq!(session.command_output().as_deref(), Some(""));
}

#[test]
fn the_next_prompt_is_not_part_of_a_command_s_output() {
    let (session, _) = detached(20, 6);
    prompt(&session);
    session.mark_command().unwrap();
    session.feed(b"echo hi\r\n\x1b]133;C\x07hi\r\n\x1b]133;D;0\x07");
    // The shell is quick: its next prompt is drawn before the command is
    // read as done.
    prompt(&session);
    assert_eq!(session.command_output().as_deref(), Some("hi"));

    session.end_command();
    session.mark_command().unwrap();
    session.feed(b"true\r\n\x1b]133;C\x07\x1b]133;D;0\x07");
    prompt(&session);
    assert_eq!(session.command_output().as_deref(), Some(""));
}

#[test]
fn without_a_mark_there_is_no_output_to_read() {
    let (session, _) = detached(20, 4);
    assert_eq!(session.command_output(), None);
    session.mark_command().unwrap();
    session.end_command();
    assert_eq!(session.command_output(), None);
}

#[test]
fn the_configured_cursor_is_the_one_programs_start_with_and_reset_to() {
    let options = Options {
        size: Size::new(20, 4),
        cursor: CursorShape::Bar,
        cursor_blink: true,
        ..Options::default()
    };
    let session = Session::detached(options, || {}).unwrap();
    let mut grid = Grid::new().unwrap();
    grid.update(&session).unwrap();
    assert_eq!(grid.cursor.shape, CursorShape::Bar);
    assert!(grid.cursor.blinking);

    // A steady block, as an editor asks for in normal mode...
    session.feed(b"\x1b[2 q");
    grid.update(&session).unwrap();
    assert_eq!(grid.cursor.shape, CursorShape::Block);
    assert!(!grid.cursor.blinking);
    // ...and the configured one back when it resets.
    session.feed(b"\x1b[0 q");
    grid.update(&session).unwrap();
    assert_eq!(grid.cursor.shape, CursorShape::Bar);

    // A new config takes over at once.
    session.set_cursor(CursorShape::Underline, false).unwrap();
    let damage = grid.update(&session).unwrap();
    assert!(damage.cursor);
    assert_eq!(grid.cursor.shape, CursorShape::Underline);
    assert!(!grid.cursor.blinking);
}
