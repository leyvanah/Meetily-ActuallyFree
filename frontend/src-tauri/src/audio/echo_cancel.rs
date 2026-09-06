// audio/echo_cancel.rs
//
// Acoustic echo cancellation for the microphone channel.
//
// When the other person is heard through the speakers, their voice reaches the
// microphone as well. Without cancellation the transcript carries every remote
// phrase twice - once from the system channel and once attributed to the local
// speaker. This runs the mixer's aligned mic/system window pair through AEC3
// (`aec3`, a Rust port of WebRTC's canceller) with the system channel as the
// reference signal, and hands back the microphone with that echo removed.
//
// The canceller itself is not `Send` (its graph is built on `Rc`), while the
// audio pipeline runs as a spawned task that must be. So it lives on its own
// thread and is reached through channels; a 50 ms window costs about a
// millisecond of work there, and the pipeline waits for the answer to keep the
// timeline in order.
//
// Noise suppression and automatic gain are deliberately off: the app has its
// own noise handling and level control, and both would alter the archived
// microphone track beyond echo removal.

use aec3::nodes::audio::AudioFormat;
use aec3::pipelines::linear::{self, LinearPipeline};
use log::{info, warn};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

/// AEC3 processes audio in 10 ms frames.
const FRAME_MS: usize = 10;
/// How long the pipeline waits for a cleaned window before giving up on the
/// canceller entirely. Generous: it only ever trips on a stuck worker.
const WORKER_TIMEOUT: Duration = Duration::from_millis(500);

enum Response {
    Ready,
    Window(Vec<f32>),
    Failed(String),
}

pub struct EchoCanceller {
    requests: Option<Sender<(Vec<f32>, Vec<f32>)>>,
    responses: Receiver<Response>,
    worker: Option<JoinHandle<()>>,
    sample_rate: u32,
    /// Once the canceller has failed, audio passes through untouched.
    failed: bool,
    processed_windows: u64,
}

impl EchoCanceller {
    /// AEC3 accepts 16-48 kHz; the capture pipeline runs at 48 kHz.
    pub fn is_supported_rate(sample_rate: u32) -> bool {
        (16_000..=48_000).contains(&sample_rate)
    }

    pub fn new(sample_rate: u32) -> Option<Self> {
        if !Self::is_supported_rate(sample_rate) {
            warn!(
                "Echo cancellation unavailable at {} Hz (needs 16-48 kHz)",
                sample_rate
            );
            return None;
        }

        let (request_tx, request_rx) = mpsc::channel::<(Vec<f32>, Vec<f32>)>();
        let (response_tx, response_rx) = mpsc::channel::<Response>();

        let worker = std::thread::Builder::new()
            .name("echo-canceller".to_string())
            .spawn(move || worker_loop(sample_rate, request_rx, response_tx))
            .map_err(|error| warn!("Failed to start the echo cancellation thread: {}", error))
            .ok()?;

        // The worker reports whether the canceller could be built at all
        match response_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Response::Ready) => {
                info!("🔇 Echo cancellation enabled ({} Hz)", sample_rate);
                Some(Self {
                    requests: Some(request_tx),
                    responses: response_rx,
                    worker: Some(worker),
                    sample_rate,
                    failed: false,
                    processed_windows: 0,
                })
            }
            Ok(Response::Failed(error)) => {
                warn!("Failed to start echo cancellation: {}", error);
                None
            }
            Ok(Response::Window(_)) => {
                warn!("Echo cancellation worker answered out of order on startup");
                None
            }
            Err(error) => {
                warn!("Echo cancellation worker did not start: {}", error);
                None
            }
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Remove the system channel's echo from one aligned microphone window.
    ///
    /// `mic` and `system` cover the same span of wall-clock time, as produced by
    /// the mixer ring buffer. Returns the cleaned microphone samples, or the
    /// input untouched if the canceller cannot run - losing echo suppression is
    /// always better than losing the recording.
    pub fn process(&mut self, mic: &[f32], system: &[f32]) -> Vec<f32> {
        if self.failed || mic.is_empty() {
            return mic.to_vec();
        }

        let Some(requests) = self.requests.as_ref() else {
            return self.give_up(mic, "worker channel closed");
        };
        if requests.send((mic.to_vec(), system.to_vec())).is_err() {
            return self.give_up(mic, "worker stopped");
        }

        match self.responses.recv_timeout(WORKER_TIMEOUT) {
            Ok(Response::Window(cleaned)) if cleaned.len() == mic.len() => {
                self.processed_windows += 1;
                cleaned
            }
            Ok(Response::Window(cleaned)) => self.give_up(
                mic,
                format!(
                    "worker returned {} samples for a {}-sample window",
                    cleaned.len(),
                    mic.len()
                ),
            ),
            Ok(Response::Failed(error)) => self.give_up(mic, error),
            Ok(Response::Ready) => self.give_up(mic, "worker answered out of order"),
            Err(RecvTimeoutError::Timeout) => self.give_up(mic, "worker timed out"),
            Err(RecvTimeoutError::Disconnected) => self.give_up(mic, "worker disconnected"),
        }
    }

    /// Report once, then pass audio through untouched for the rest of the run.
    fn give_up(&mut self, mic: &[f32], error: impl std::fmt::Display) -> Vec<f32> {
        self.failed = true;
        self.requests = None;
        warn!(
            "Echo cancellation stopped after {} windows: {} - continuing without it",
            self.processed_windows, error
        );
        mic.to_vec()
    }
}

impl Drop for EchoCanceller {
    fn drop(&mut self) {
        // Closing the request channel ends the worker's loop
        self.requests = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    sample_rate: u32,
    requests: Receiver<(Vec<f32>, Vec<f32>)>,
    responses: Sender<Response>,
) {
    let format = AudioFormat::ten_ms(sample_rate, 1);
    let mut pipeline = match linear::builder(format, format)
        .enable_noise_suppression(false)
        .enable_gain_controller2(false)
        .build()
    {
        Ok(pipeline) => {
            if responses.send(Response::Ready).is_err() {
                return;
            }
            pipeline
        }
        Err(error) => {
            let _ = responses.send(Response::Failed(error.to_string()));
            return;
        }
    };

    let frame_samples = sample_rate as usize * FRAME_MS / 1000;

    while let Ok((mic, system)) = requests.recv() {
        let response = match process_window(&mut pipeline, &mic, &system, frame_samples) {
            Ok(cleaned) => Response::Window(cleaned),
            Err(error) => Response::Failed(error),
        };
        let failed = matches!(response, Response::Failed(_));
        if responses.send(response).is_err() || failed {
            return;
        }
    }
}

/// Run one window through the canceller, 10 ms at a time.
fn process_window(
    pipeline: &mut LinearPipeline,
    mic: &[f32],
    system: &[f32],
    frame_samples: usize,
) -> Result<Vec<f32>, String> {
    let mut cleaned = Vec::with_capacity(mic.len());
    let mut offset = 0usize;

    while offset < mic.len() {
        pipeline
            .handle_render_frame(&frame_at(system, offset, frame_samples))
            .map_err(|error| error.to_string())?;

        let mut processed = vec![0.0f32; frame_samples];
        let produced = pipeline
            .process_capture_frame(&frame_at(mic, offset, frame_samples), &mut processed)
            .map_err(|error| error.to_string())?;

        // The canceller holds back the first frames while it aligns the two
        // streams; passing the untouched microphone through keeps the timeline
        // intact instead of punching a hole in it.
        let valid = (mic.len() - offset).min(frame_samples);
        if produced {
            cleaned.extend_from_slice(&processed[..valid]);
        } else {
            cleaned.extend_from_slice(&mic[offset..offset + valid]);
        }
        offset += frame_samples;
    }

    Ok(cleaned)
}

/// One frame starting at `offset`, zero-padded when the window ends early.
fn frame_at(samples: &[f32], offset: usize, frame_samples: usize) -> Vec<f32> {
    let mut frame = vec![0.0f32; frame_samples];
    if offset < samples.len() {
        let count = (samples.len() - offset).min(frame_samples);
        frame[..count].copy_from_slice(&samples[offset..offset + count]);
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 48_000;

    /// A window of the mixer's size, as the capture pipeline produces it.
    fn window_samples() -> usize {
        SAMPLE_RATE as usize * 50 / 1000
    }

    fn tone(len: usize, freq: f32, phase: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = (i + phase) as f32 / SAMPLE_RATE as f32;
                0.3 * (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect()
    }

    #[test]
    fn unsupported_rates_are_rejected() {
        assert!(!EchoCanceller::is_supported_rate(8_000));
        assert!(!EchoCanceller::is_supported_rate(96_000));
        assert!(EchoCanceller::is_supported_rate(16_000));
        assert!(EchoCanceller::is_supported_rate(48_000));
        assert!(EchoCanceller::new(8_000).is_none());
    }

    #[test]
    fn windows_keep_their_length_and_timeline() {
        let mut canceller = EchoCanceller::new(SAMPLE_RATE).expect("canceller");
        assert_eq!(canceller.sample_rate(), SAMPLE_RATE);
        let window = window_samples();
        for index in 0..10 {
            let mic = tone(window, 220.0, index * window);
            let system = tone(window, 440.0, index * window);
            let cleaned = canceller.process(&mic, &system);
            assert_eq!(cleaned.len(), mic.len(), "window {} changed length", index);
        }
    }

    #[test]
    fn partial_and_empty_windows_are_handled() {
        let mut canceller = EchoCanceller::new(SAMPLE_RATE).expect("canceller");
        // The flush path hands over whatever is left, which is not a whole window
        let mic = tone(777, 220.0, 0);
        let system = tone(777, 440.0, 0);
        assert_eq!(canceller.process(&mic, &system).len(), 777);
        assert!(canceller.process(&[], &[]).is_empty());
    }

    /// What the canceller costs the capture pipeline. Opt-in because the number
    /// only means something in a release build:
    ///   cargo test --release --lib echo_cancel -- --ignored --nocapture
    #[test]
    #[ignore = "timing measurement, run in release"]
    fn cost_relative_to_realtime() {
        let mut canceller = EchoCanceller::new(SAMPLE_RATE).expect("canceller");
        let window = window_samples();
        let windows = 1200; // 60 seconds of audio
        let mic = tone(window, 220.0, 0);
        let system = tone(window, 440.0, 0);

        let started = std::time::Instant::now();
        for _ in 0..windows {
            let cleaned = canceller.process(&mic, &system);
            assert_eq!(cleaned.len(), window);
        }
        let elapsed = started.elapsed().as_secs_f64();
        let audio_seconds = windows as f64 * 0.05;

        println!(
            "echo cancellation: {:.3} s of work for {:.0} s of audio ({:.2}% of realtime, {:.2} ms per 50 ms window)",
            elapsed,
            audio_seconds,
            elapsed / audio_seconds * 100.0,
            elapsed / windows as f64 * 1000.0
        );
    }

    /// Runs a real recording through the canceller with a silent far end, so
    /// its effect on speech can be measured on its own:
    ///   AEC_IN=<in.wav> AEC_OUT=<out.wav> cargo test --lib echo_cancel -- --ignored --nocapture
    #[test]
    #[ignore = "needs AEC_IN and AEC_OUT"]
    fn passes_a_recording_through_with_a_silent_far_end() {
        let input = std::env::var("AEC_IN").expect("AEC_IN");
        let output = std::env::var("AEC_OUT").expect("AEC_OUT");
        let (samples, sample_rate) =
            crate::diarization::dsp::read_wav(std::path::Path::new(&input)).expect("read wav");

        // Optional far-end track, so the suppressor can be exercised the way a
        // live call drives it rather than against pure silence.
        let far_end = std::env::var("AEC_FAR").ok().map(|path| {
            crate::diarization::dsp::read_wav(std::path::Path::new(&path))
                .expect("read far-end wav")
                .0
        });

        let mut canceller = EchoCanceller::new(sample_rate).expect("canceller");
        let window = sample_rate as usize * 50 / 1000;
        let silence = vec![0.0f32; window];
        let mut cleaned = Vec::with_capacity(samples.len());
        for (index, chunk) in samples.chunks(window).enumerate() {
            let reference: Vec<f32> = match far_end.as_ref() {
                Some(far) => {
                    let start = index * window;
                    let mut block = vec![0.0f32; chunk.len()];
                    if start < far.len() {
                        let count = (far.len() - start).min(chunk.len());
                        block[..count].copy_from_slice(&far[start..start + count]);
                    }
                    block
                }
                None => silence[..chunk.len()].to_vec(),
            };
            cleaned.extend(canceller.process(chunk, &reference));
        }

        // 16-bit PCM so the comparison script can read it back
        let mut bytes = Vec::with_capacity(44 + cleaned.len() * 2);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&((36 + cleaned.len() * 2) as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&((cleaned.len() * 2) as u32).to_le_bytes());
        for sample in &cleaned {
            bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
        }
        std::fs::write(&output, bytes).expect("write wav");
        println!("wrote {} samples to {}", cleaned.len(), output);
    }

    /// The echo of the system channel must lose most of its energy.
    #[test]
    fn speaker_echo_is_removed_from_the_microphone() {
        let mut canceller = EchoCanceller::new(SAMPLE_RATE).expect("canceller");
        let window = window_samples();
        let delay = SAMPLE_RATE as usize * 40 / 1000; // 40 ms of echo delay

        // Continuous far-end signal whose echo arrives late in the microphone
        let total = window * 200; // 10 seconds
        let far = tone(total, 440.0, 0);
        let mut mic = vec![0.0f32; total];
        for n in delay..total {
            mic[n] = 0.5 * far[n - delay];
        }

        let mut residual = 0.0f32;
        let mut original = 0.0f32;
        for index in 0..(total / window) {
            let from = index * window;
            let to = from + window;
            let cleaned = canceller.process(&mic[from..to], &far[from..to]);
            // Measure once the filter has had a few seconds to converge
            if index > 120 {
                residual += cleaned.iter().map(|s| s * s).sum::<f32>();
                original += mic[from..to].iter().map(|s| s * s).sum::<f32>();
            }
        }

        let erle = 10.0 * (original / residual.max(1e-12)).log10();
        assert!(
            erle > 15.0,
            "expected the echo to be strongly suppressed, got {:.1} dB",
            erle
        );
    }
}
