//! The fire's frames, shared by the examples that time it: `hof_vt`'s,
//! for the parser and grid, and `hof_qml`'s, for the item that paints.

use std::io::Write;

/// Hottest to coldest: white, yellow, orange, red, black, as a colour for
/// each of `LEVELS` heats.
const STOPS: [(f64, [u8; 3]); 6] = [
    (0.0, [0, 0, 0]),
    (0.2, [70, 8, 4]),
    (0.45, [190, 32, 8]),
    (0.65, [235, 110, 16]),
    (0.85, [250, 210, 60]),
    (1.0, [255, 255, 235]),
];
const LEVELS: usize = 40;

fn palette() -> [[u8; 3]; LEVELS] {
    let mut out = [[0; 3]; LEVELS];
    for (i, colour) in out.iter_mut().enumerate() {
        let t = i as f64 / (LEVELS - 1) as f64;
        let upper = STOPS
            .iter()
            .position(|(at, _)| *at >= t)
            .unwrap_or(0)
            .max(1);
        let ((a, from), (b, to)) = (STOPS[upper - 1], STOPS[upper]);
        let f = (t - a) / (b - a);
        for c in 0..3 {
            colour[c] =
                (f64::from(from[c]) + (f64::from(to[c]) - f64::from(from[c])) * f).round() as u8;
        }
    }
    out
}

/// Heat rising from a burning bottom row, cooling and drifting at random.
/// Two heats a cell: the upper and lower halves of `▀`.
pub struct Fire {
    width: usize,
    height: usize,
    heat: Vec<u8>,
    seed: u64,
    palette: [[u8; 3]; LEVELS],
}

impl Fire {
    pub fn new(columns: u16, rows: u16) -> Self {
        let (width, height) = (usize::from(columns), usize::from(rows) * 2);
        let mut heat = vec![0; width * height];
        heat[(height - 1) * width..].fill((LEVELS - 1) as u8);
        Self {
            width,
            height,
            heat,
            seed: 0x9e37_79b9_7f4a_7c15,
            palette: palette(),
        }
    }

    // xorshift: plenty random for flames, and the same fire every run.
    fn random(&mut self) -> u64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed
    }

    pub fn step(&mut self) {
        let w = self.width;
        for y in 0..self.height - 1 {
            for x in 0..w {
                let r = self.random();
                let cooling = (r & 1) as u8;
                // Up and a little sideways, as a flame flickers.
                let drift = (r >> 1) % 3;
                let to = (x + drift as usize).saturating_sub(1).min(w - 1);
                let below = self.heat[(y + 1) * w + x];
                self.heat[y * w + to] = below.saturating_sub(cooling);
            }
        }
    }

    /// One frame, from the top left: every cell its colours in full.
    pub fn draw(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b[H");
        for row in 0..self.height / 2 {
            if row > 0 {
                out.extend_from_slice(b"\r\n");
            }
            for x in 0..self.width {
                let top = self.palette[usize::from(self.heat[row * 2 * self.width + x])];
                let bottom = self.palette[usize::from(self.heat[(row * 2 + 1) * self.width + x])];
                let _ = write!(
                    out,
                    "\x1b[38;2;{};{};{};48;2;{};{};{}m\u{2580}",
                    top[0], top[1], top[2], bottom[0], bottom[1], bottom[2]
                );
            }
        }
        out.extend_from_slice(b"\x1b[0m");
    }
}

/// `count` frames of a `columns` by `rows` fire, made before any timing.
pub fn frames(columns: u16, rows: u16, count: usize) -> Vec<Vec<u8>> {
    let mut fire = Fire::new(columns, rows);
    (0..count)
        .map(|_| {
            fire.step();
            let mut frame = Vec::new();
            fire.draw(&mut frame);
            frame
        })
        .collect()
}
