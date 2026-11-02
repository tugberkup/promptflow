//! Microphone recording. The stream lives on its own thread (audio streams aren't Send on every OS);
//! start() and stop() talk to it through a channel.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream};

pub const SAMPLE_RATE: u32 = 16_000;

enum Command {
    Start(Option<String>, Sender<Result<String>>),
    Stop(Sender<Vec<f32>>),
}

pub struct Recorder {
    commands: Sender<Command>,
    /// Loudness of the last audio block (f32 bits), for the level meter
    pub level: Arc<AtomicU32>,
}

fn device_name(device: &Device) -> String {
    device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "Unknown".into())
}

/// Names of the microphones, for the picker.
pub fn list_mics() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(devices) = cpal::default_host().input_devices() {
        for device in devices {
            let name = device_name(&device);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn find_device(wanted: Option<&str>) -> Option<Device> {
    let host = cpal::default_host();
    if let Some(wanted) = wanted.filter(|w| !w.is_empty()) {
        let wanted = wanted.to_lowercase();
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                if device_name(&device).to_lowercase().contains(&wanted) {
                    return Some(device);
                }
            }
        }
    }
    host.default_input_device()
}

struct Recording {
    _stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}

fn open(wanted: Option<&str>, level: Arc<AtomicU32>) -> Result<(Recording, String)> {
    let device = find_device(wanted).ok_or_else(|| anyhow!("no microphone found"))?;
    let name = device_name(&device);
    let config = device.default_input_config()?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate();
    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));

    // Mix every frame down to mono and keep it; resampling to 16 kHz happens once at the end
    fn push<T: Copy>(data: &[T], channels: usize, convert: impl Fn(T) -> f32, samples: &Mutex<Vec<f32>>, level: &AtomicU32) {
        let mut buf = samples.lock().unwrap();
        let mut sum = 0.0f32;
        for frame in data.chunks(channels) {
            let v = frame.iter().map(|s| convert(*s)).sum::<f32>() / channels as f32;
            sum += v * v;
            buf.push(v);
        }
        let frames = (data.len() / channels).max(1) as f32;
        level.store((sum / frames).sqrt().to_bits(), Ordering::Relaxed);
    }

    let err = |e| eprintln!("[audio] {e}");
    let (s, l) = (samples.clone(), level.clone());
    let stream = match config.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            config.into(), move |d: &[f32], _: &_| push(d, channels, |x| x, &s, &l), err, None)?,
        SampleFormat::I16 => device.build_input_stream(
            config.into(), move |d: &[i16], _: &_| push(d, channels, |x| x as f32 / 32768.0, &s, &l), err, None)?,
        SampleFormat::I32 => device.build_input_stream(
            config.into(), move |d: &[i32], _: &_| push(d, channels, |x| x as f32 / 2147483648.0, &s, &l), err, None)?,
        SampleFormat::U16 => device.build_input_stream(
            config.into(), move |d: &[u16], _: &_| push(d, channels, |x| (x as f32 - 32768.0) / 32768.0, &s, &l), err, None)?,
        other => return Err(anyhow!("unsupported microphone sample format {other}")),
    };
    stream.play()?;
    Ok((Recording { _stream: stream, samples, rate }, name))
}

/// Reads a 16-bit PCM WAV file as 16 kHz mono (used by tests in place of the microphone).
pub fn read_wav(path: &std::path::Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    let (mut pos, mut channels, mut rate, mut bits) = (12usize, 1usize, SAMPLE_RATE, 16u16);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = bytes.get(pos + 8..(pos + 8 + size).min(bytes.len()))?;
        if id == b"fmt " {
            channels = u16::from_le_bytes(body[2..4].try_into().ok()?) as usize;
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
        } else if id == b"data" && bits == 16 {
            let mono: Vec<f32> = body
                .chunks_exact(2 * channels)
                .map(|f| f.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0).sum::<f32>() / channels as f32)
                .collect();
            return Some(resample(&mono, rate));
        }
        pos += 8 + size + (size & 1);
    }
    None
}

/// Linear resampling to 16 kHz; plenty for speech.
pub fn resample(input: &[f32], from: u32) -> Vec<f32> {
    if from == SAMPLE_RATE || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / SAMPLE_RATE as f64;
    let len = (input.len() as f64 / ratio) as usize;
    (0..len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos as usize;
            let frac = (pos - j as f64) as f32;
            let a = input[j];
            let b = *input.get(j + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

impl Recorder {
    pub fn new() -> Self {
        let (tx, rx) = channel::<Command>();
        let level = Arc::new(AtomicU32::new(0));
        let lvl = level.clone();
        thread::spawn(move || {
            let mut current: Option<Recording> = None;
            for command in rx {
                match command {
                    Command::Start(wanted, reply) => {
                        current = None;
                        match open(wanted.as_deref(), lvl.clone()) {
                            Ok((rec, name)) => {
                                current = Some(rec);
                                let _ = reply.send(Ok(name));
                            }
                            Err(e) => {
                                let _ = reply.send(Err(e));
                            }
                        }
                    }
                    Command::Stop(reply) => {
                        let audio = current
                            .take()
                            .map(|rec| {
                                let samples = std::mem::take(&mut *rec.samples.lock().unwrap());
                                resample(&samples, rec.rate)
                            })
                            .unwrap_or_default();
                        lvl.store(0, Ordering::Relaxed);
                        let _ = reply.send(audio);
                    }
                }
            }
        });
        Self { commands: tx, level }
    }

    /// Starts recording; returns the microphone's name.
    pub fn start(&self, mic: Option<String>) -> Result<String> {
        let (tx, rx) = channel();
        self.commands.send(Command::Start(mic, tx))?;
        rx.recv()?
    }

    /// Stops recording; returns 16 kHz mono audio.
    pub fn stop(&self) -> Vec<f32> {
        let (tx, rx) = channel();
        if self.commands.send(Command::Stop(tx)).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resample_48k_to_16k() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48.0).sin()).collect();
        let out = super::resample(&input, 48_000);
        assert_eq!(out.len(), 16_000);
    }
}
