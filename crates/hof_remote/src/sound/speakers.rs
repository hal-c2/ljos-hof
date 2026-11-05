//! The speakers Hof's sound server plays through on a Mac: whatever
//! output macOS has chosen, followed as the user changes it.
//!
//! The output is opened when something first sounds and closed once all
//! has been quiet a while, so a Mac with nothing playing is not kept
//! awake by Hof.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig};

use super::mixer::Mixer;

/// How long all may be quiet before the output is closed.
const QUIET: Duration = Duration::from_secs(10);
/// How often an open output is looked at.
const LOOK: Duration = Duration::from_millis(250);

/// The rate the default output plays at, if there is one.
pub fn rate() -> Option<u32> {
    let device = cpal::default_host().default_output_device()?;
    Some(device.default_output_config().ok()?.sample_rate())
}

/// Plays `mixer` through the default output, for as long as Hof runs.
pub fn start(mixer: Arc<Mixer>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("hof speakers".into())
        .spawn(move || run(&mixer))?;
    Ok(())
}

fn run(mixer: &Arc<Mixer>) {
    loop {
        mixer.wait_for_sound();
        let broken = Arc::new(AtomicBool::new(false));
        match open(mixer, broken.clone()) {
            Ok(stream) => {
                while !broken.load(Ordering::Relaxed) && mixer.quiet_for() < QUIET {
                    std::thread::sleep(LOOK);
                }
                drop(stream);
            }
            Err(e) => {
                eprintln!("sound: could not open the speakers: {e}");
                // Wait before trying again, for an output to come.
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn open(mixer: &Arc<Mixer>, broken: Arc<AtomicBool>) -> Result<cpal::Stream, cpal::Error> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| cpal::Error::new(ErrorKind::DeviceNotAvailable))?;
    let supported = device.default_output_config()?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::I16 => build::<i16>(&device, config, mixer.clone(), broken)?,
        SampleFormat::I32 => build::<i32>(&device, config, mixer.clone(), broken)?,
        SampleFormat::U16 => build::<u16>(&device, config, mixer.clone(), broken)?,
        _ => build::<f32>(&device, config, mixer.clone(), broken)?,
    };
    stream.play()?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mixer: Arc<Mixer>,
    broken: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    let mut scratch: Vec<f32> = Vec::new();
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            scratch.resize(data.len(), 0.0);
            mixer.render(&mut scratch, channels, rate);
            for (out, &v) in data.iter_mut().zip(&scratch) {
                *out = T::from_sample(v);
            }
        },
        move |e: cpal::Error| {
            // A change of output is followed by the stream itself; anything
            // else is opened again.
            if !matches!(e.kind(), ErrorKind::DeviceChanged | ErrorKind::Xrun) {
                eprintln!("sound: the speakers failed: {e}");
                broken.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}
