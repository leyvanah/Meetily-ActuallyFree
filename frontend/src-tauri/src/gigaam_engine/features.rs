// gigaam_engine/features.rs
//
// Log-mel feature extraction for GigaAM v3.
//
// The GigaAM v3 encoder expects exactly what the reference ONNX preprocessor
// produces (istupakov/onnx-asr, MIT): a 320-sample periodic Hann window, hop
// 160, 320-point FFT, 64 HTK mel bands over 0-8000 Hz, no centring/padding and
// no normalisation, followed by log(clip(x, 1e-9, 1e9)). The window and the mel
// matrix are rounded to bfloat16 there, so we round them the same way - the
// constants the model was exported against are the rounded ones.

use realfft::RealFftPlanner;

pub const SAMPLE_RATE: u32 = 16_000;
pub const N_MELS: usize = 64;
/// FFT size and window length: 20 ms at 16 kHz.
pub const WIN_LENGTH: usize = 320;
/// Frame shift: 10 ms at 16 kHz.
pub const HOP_LENGTH: usize = 160;
const N_FREQS: usize = WIN_LENGTH / 2 + 1;
const F_MIN: f64 = 0.0;
const F_MAX: f64 = 8_000.0;
const CLAMP_MIN: f32 = 1e-9;
const CLAMP_MAX: f32 = 1e9;

/// Round a f32 to bfloat16 precision (round half to even) and back.
fn to_bf16(value: f32) -> f32 {
    let bits = value.to_bits();
    if value.is_nan() {
        return value;
    }
    let lsb = (bits >> 16) & 1;
    let rounded = bits.wrapping_add(0x7fff + lsb);
    f32::from_bits(rounded & 0xffff_0000)
}

fn hz_to_mel(freq: f64) -> f64 {
    2595.0 * (1.0 + freq / 700.0).log10()
}

fn mel_to_hz(mel: f64) -> f64 {
    700.0 * (10f64.powf(mel / 2595.0) - 1.0)
}

/// Triangular HTK mel filters, `[N_FREQS][N_MELS]`, unnormalised.
fn mel_filterbank() -> Vec<Vec<f32>> {
    let nyquist = (SAMPLE_RATE / 2) as f64;
    let all_freqs: Vec<f64> = (0..N_FREQS)
        .map(|k| nyquist * k as f64 / (N_FREQS - 1) as f64)
        .collect();

    let mel_min = hz_to_mel(F_MIN);
    let mel_max = hz_to_mel(F_MAX);
    let points: Vec<f64> = (0..N_MELS + 2)
        .map(|i| {
            let mel = mel_min + (mel_max - mel_min) * i as f64 / (N_MELS + 1) as f64;
            mel_to_hz(mel)
        })
        .collect();

    let mut filters = vec![vec![0f32; N_MELS]; N_FREQS];
    for (k, &freq) in all_freqs.iter().enumerate() {
        for m in 0..N_MELS {
            let up = (freq - points[m]) / (points[m + 1] - points[m]);
            let down = (points[m + 2] - freq) / (points[m + 2] - points[m + 1]);
            filters[k][m] = to_bf16(up.min(down).max(0.0) as f32);
        }
    }
    filters
}

/// Periodic Hann window of `WIN_LENGTH` samples, as numpy's `hanning(n + 1)[:-1]`.
fn hann_window() -> Vec<f32> {
    (0..WIN_LENGTH)
        .map(|i| {
            let value =
                0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / WIN_LENGTH as f64).cos();
            to_bf16(value as f32)
        })
        .collect()
}

/// Number of frames the encoder will see for `samples` input samples.
pub fn frame_count(samples: usize) -> usize {
    if samples < WIN_LENGTH {
        0
    } else {
        (samples - WIN_LENGTH) / HOP_LENGTH + 1
    }
}

/// Reusable feature extractor: the window and mel matrix are built once.
pub struct FeatureExtractor {
    window: Vec<f32>,
    filters: Vec<Vec<f32>>,
}

impl Default for FeatureExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureExtractor {
    pub fn new() -> Self {
        Self {
            window: hann_window(),
            filters: mel_filterbank(),
        }
    }

    /// Log-mel features in the encoder's layout: `[N_MELS][frames]`, flattened
    /// row-major so it can be handed to ONNX Runtime as `[1, 64, T]`.
    pub fn compute(&self, samples: &[f32]) -> (Vec<f32>, usize) {
        let frames = frame_count(samples.len());
        let mut features = vec![0f32; N_MELS * frames];
        if frames == 0 {
            return (features, 0);
        }

        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(WIN_LENGTH);
        let mut fft_in = r2c.make_input_vec();
        let mut fft_out = r2c.make_output_vec();
        let mut power = vec![0f32; N_FREQS];

        for frame in 0..frames {
            let start = frame * HOP_LENGTH;
            for i in 0..WIN_LENGTH {
                fft_in[i] = samples[start + i] * self.window[i];
            }
            if r2c.process(&mut fft_in, &mut fft_out).is_err() {
                continue;
            }
            for (k, bin) in fft_out.iter().enumerate() {
                power[k] = bin.re * bin.re + bin.im * bin.im;
            }

            for mel in 0..N_MELS {
                let mut energy = 0f32;
                for (k, weights) in self.filters.iter().enumerate() {
                    energy += power[k] * weights[mel];
                }
                features[mel * frames + frame] = energy.clamp(CLAMP_MIN, CLAMP_MAX).ln();
            }
        }

        (features, frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic signal both this code and the reference preprocessor can
    /// be run on: 0.5 s of a 440 Hz tone with a slow amplitude ramp.
    fn reference_signal() -> Vec<f32> {
        (0..8000)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let ramp = 0.2 + 0.6 * (i as f32 / 8000.0);
                ramp * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect()
    }

    #[test]
    fn frame_count_matches_the_reference_formula() {
        assert_eq!(frame_count(0), 0);
        assert_eq!(frame_count(319), 0);
        assert_eq!(frame_count(320), 1);
        assert_eq!(frame_count(480), 2);
        assert_eq!(frame_count(8000), 49);
    }

    #[test]
    fn window_and_filters_are_rounded_to_bfloat16() {
        // bfloat16 keeps 8 mantissa bits, so the low 16 bits are always zero.
        assert!(hann_window()
            .iter()
            .all(|value| value.to_bits() & 0xffff == 0));
        assert!(mel_filterbank()
            .iter()
            .all(|row| row.iter().all(|value| value.to_bits() & 0xffff == 0)));
        // Periodic Hann: starts at 0, peaks in the middle, never reaches 1 again.
        let window = hann_window();
        assert_eq!(window[0], 0.0);
        assert!((window[WIN_LENGTH / 2] - 1.0).abs() < 0.01);
    }

    /// Values produced by the reference ONNX preprocessor (gigaam_v3.onnx from
    /// onnx-asr) for `reference_signal()`; see docs/TZ/03-gigaam-stt.md.
    #[test]
    fn features_match_the_reference_preprocessor() {
        let extractor = FeatureExtractor::new();
        let (features, frames) = extractor.compute(&reference_signal());
        assert_eq!(frames, 49);
        assert_eq!(features.len(), N_MELS * frames);

        // Produced by gigaam_v3.onnx (onnx-asr) for the same signal.
        let expected = [
            (3usize, 24usize, -8.662489f32),
            (63, 24, -9.123843),
            (3, 5, -9.589718),
            (3, 45, -7.529559),
            (0, 0, -11.069707),
            (32, 10, -10.812695),
        ];
        for (mel, frame, reference) in expected {
            let value = features[mel * frames + frame];
            assert!(
                (value - reference).abs() < 0.05,
                "f[{}][{}] = {} but the reference says {}",
                mel,
                frame,
                value,
                reference
            );
        }

        let mean = features.iter().sum::<f32>() / features.len() as f32;
        assert!(
            (mean - -7.723981).abs() < 0.02,
            "feature mean drifted: {}",
            mean
        );
    }
}
