//! The fire, painted: each frame written to a `Terminal` item in a window
//! with no screen, and the window let run a frame, which brings the grid
//! up to date and builds what the item draws. What is left of a frame's
//! cost is the GPU's.
//!
//! ```text
//! cargo run --release -p hof_qml --example paint_fire
//! ```

use std::time::Instant;

use hof_qml::{Terminal, register};
use ljos::item::Font;
use ljos::prelude::*;
use ljos::testing::TestWindow;

#[path = "../../hof_vt/examples/fire/frames.rs"]
mod frames;

fn main() {
    let (columns, rows) = (200u16, 60u16);
    let font = Font {
        pixel_size: 12,
        ..Font::default()
    };
    let engine = ljos::engine();
    register(&engine);
    let t = engine.create_by_name("Hof", "Terminal", None).unwrap();
    engine.set(t, Terminal::EXTERNAL, true).unwrap();
    engine.set(t, Terminal::FONT, font).unwrap();
    let mut window = TestWindow::new(&engine, 100.0, 100.0);
    window.show(t).unwrap();
    window.settle().unwrap();
    // As big as `columns` by `rows` of its cells.
    let cell = |name| engine.get::<f64>(t, name).unwrap();
    let (w, h) = (
        f64::from(columns) * cell(Terminal::CELL_WIDTH),
        f64::from(rows) * cell(Terminal::CELL_HEIGHT),
    );
    engine.set(t, Item::WIDTH, w).unwrap();
    engine.set(t, Item::HEIGHT, h).unwrap();
    window.resize(w, h).unwrap();
    window.settle().unwrap();
    let got = (
        engine.get::<i32>(t, Terminal::COLUMNS).unwrap(),
        engine.get::<i32>(t, Terminal::ROWS).unwrap(),
    );

    let frames: Vec<String> = frames::frames(columns, rows, 300)
        .into_iter()
        .map(|f| String::from_utf8(f).expect("the frames are text"))
        .collect();
    let bytes: usize = frames.iter().map(String::len).sum();
    let start = Instant::now();
    for frame in &frames {
        engine
            .invoke_by_name(t, "write", &[Value::from(frame.as_str())])
            .unwrap();
        window.advance(16).unwrap();
    }
    let s = start.elapsed().as_secs_f64();
    let ops = window.painted(t).map_or(0, |list| list.ops().len());
    println!(
        "{}x{}: {:.0} frames/s, {:.0} MB/s, {:.2} ms a frame, {ops} draw ops in the last",
        got.0,
        got.1,
        frames.len() as f64 / s,
        bytes as f64 / s / 1e6,
        s * 1e3 / frames.len() as f64,
    );
}
