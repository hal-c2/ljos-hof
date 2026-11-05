//! Hof's sound server, served to PulseAudio's own clients: `pactl` and
//! `paplay`, where they are installed.

use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use hof_remote::sound::{Mixer, Sound};

fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A server on a socket in `dir`, with speakers that are a thread adding
/// up how loud it was.
struct Served {
    socket: String,
    energy: Arc<Mutex<f64>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(dir: &Path) -> Served {
    let path = dir.join("native");
    let listener = UnixListener::bind(&path).unwrap();
    let mixer = Arc::new(Mixer::new(48_000));
    let sound = Arc::new(Sound::new(mixer.clone()));
    std::thread::spawn(move || {
        for socket in listener.incoming().flatten() {
            let sound = sound.clone();
            std::thread::spawn(move || sound.serve(socket));
        }
    });
    let energy = Arc::new(Mutex::new(0.0));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (energy, stop) = (energy.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut out = vec![0.0; 480 * 2];
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                mixer.render(&mut out, 2, 48_000);
                let sum: f64 = out.iter().map(|&v| f64::from(v * v)).sum();
                *energy.lock().unwrap_or_else(PoisonError::into_inner) += sum;
            }
        });
    }
    Served {
        socket: format!("unix:{}", path.display()),
        energy,
        stop,
    }
}

fn run(served: &Served, program: &str, args: &[&str]) -> Output {
    let output = Command::new(program)
        .args(args)
        .env("PULSE_SERVER", &served.socket)
        .env("PULSE_COOKIE", "/nonexistent")
        .env_remove("PULSE_RUNTIME_PATH")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// A second of a 440 Hz tone, as a WAV of 16-bit stereo at 44.1 kHz.
fn tone(path: &Path) {
    let rate = 44_100u32;
    let mut data = Vec::new();
    for i in 0..rate {
        let v = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 12_000.0) as i16;
        data.extend(v.to_le_bytes());
        data.extend(v.to_le_bytes());
    }
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((data.len() as u32).to_le_bytes());
    wav.extend(data);
    std::fs::write(path, wav).unwrap();
}

#[test]
fn pactl_is_told_of_the_server_and_its_one_sink() {
    if !installed("pactl") {
        eprintln!("pactl is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let served = serve(dir.path());
    let info = String::from_utf8(run(&served, "pactl", &["info"]).stdout).unwrap();
    assert!(info.contains("Default Sink: hof"), "{info}");
    let sinks = String::from_utf8(run(&served, "pactl", &["list", "sinks"]).stdout).unwrap();
    assert!(sinks.contains("Name: hof"), "{sinks}");
    run(
        &served,
        "pactl",
        &["set-sink-volume", "@DEFAULT_SINK@", "50%"],
    );
    run(&served, "pactl", &["set-sink-mute", "@DEFAULT_SINK@", "0"]);
}

#[test]
fn paplay_plays_a_file_to_its_end() {
    if !installed("paplay") {
        eprintln!("paplay is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let served = serve(dir.path());
    let wav = dir.path().join("tone.wav");
    tone(&wav);
    let started = std::time::Instant::now();
    run(&served, "paplay", &[wav.to_str().unwrap()]);
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(900),
        "played as long as the file: {took:?}"
    );
    assert!(took < Duration::from_secs(5), "and no longer: {took:?}");
    let energy = *served.energy.lock().unwrap();
    // A second at an amplitude of 0.37 in both channels is near 13000.
    assert!(energy > 5_000.0, "it sounded: {energy}");
}
