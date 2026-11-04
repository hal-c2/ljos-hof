//! A fire that fills the screen: a stress test of the hardest kind of
//! output a terminal gets, every cell a new 24-bit colour every frame.
//!
//! ```text
//! cargo run --release -p hof_vt --example fire               # in this terminal, 10 s
//! cargo run --release -p hof_vt --example fire -- 30         # for 30 s
//! cargo run --release -p hof_vt --example fire -- headless   # Hof's parser and grid alone
//! ```
//!
//! In a terminal it draws for a while and then says how many frames the
//! terminal took per second; run it in Hof and in others to compare.
//! Headless, it makes the frames first and then times feeding them to a
//! session, and bringing a grid up to date after each, which is all a
//! frame costs Hof before painting.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use hof_vt::{Grid, Options, Session};

mod frames;

use frames::Fire;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("headless") {
        return headless();
    }
    let seconds = args.first().and_then(|s| s.parse().ok()).unwrap_or(10.0);
    in_terminal(Duration::from_secs_f64(seconds))
}

fn in_terminal(length: Duration) -> io::Result<()> {
    let size = rustix::termios::tcgetwinsize(rustix::stdio::stdout()).map_err(io::Error::from)?;
    let (columns, rows) = (size.ws_col.max(1), size.ws_row.max(1));
    let mut fire = Fire::new(columns, rows);
    let mut stdout = io::stdout().lock();
    // The other screen, without a cursor, and back after.
    stdout.write_all(b"\x1b[?1049h\x1b[?25l")?;
    let mut frame = Vec::new();
    let (mut frames, mut bytes) = (0u64, 0u64);
    let start = Instant::now();
    while start.elapsed() < length {
        fire.step();
        frame.clear();
        fire.draw(&mut frame);
        stdout.write_all(&frame)?;
        stdout.flush()?;
        frames += 1;
        bytes += frame.len() as u64;
    }
    let took = start.elapsed().as_secs_f64();
    stdout.write_all(b"\x1b[?25h\x1b[?1049l")?;
    stdout.flush()?;
    println!(
        "{columns}x{rows}: {frames} frames in {took:.1} s, {:.1} frames/s, {:.1} MB/s",
        frames as f64 / took,
        bytes as f64 / took / 1e6,
    );
    Ok(())
}

fn headless() -> io::Result<()> {
    let (columns, rows) = (200, 60);
    let frames = frames::frames(columns, rows, 300);
    let bytes: usize = frames.iter().map(Vec::len).sum();
    let open = || {
        let options = Options {
            size: hof_vt::Size::new(columns, rows),
            ..Options::default()
        };
        Session::detached(options, || {}).map_err(io::Error::other)
    };
    println!(
        "{columns}x{rows}, {} frames of {} kB each",
        frames.len(),
        bytes / frames.len() / 1000
    );

    let session = open()?;
    let start = Instant::now();
    for frame in &frames {
        session.feed(frame);
    }
    report("parsing", frames.len(), bytes, start.elapsed());

    let session = open()?;
    let mut grid = Grid::new().map_err(io::Error::other)?;
    let start = Instant::now();
    for frame in &frames {
        session.feed(frame);
        grid.update(&session).map_err(io::Error::other)?;
    }
    report("parsing and the grid", frames.len(), bytes, start.elapsed());
    Ok(())
}

fn report(what: &str, frames: usize, bytes: usize, took: Duration) {
    let s = took.as_secs_f64();
    println!(
        "{what}: {:.0} frames/s, {:.0} MB/s, {:.2} ms a frame",
        frames as f64 / s,
        bytes as f64 / s / 1e6,
        s * 1e3 / frames as f64
    );
}
