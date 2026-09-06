use super::ffmpeg::find_ffmpeg_path; // Correct path to encode module
use super::AudioDevice;
use std::io::Write;
use std::sync::Arc;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};
use tracing::{debug, error};

pub struct AudioInput {
    pub data: Arc<Vec<f32>>,
    pub sample_rate: u32,
    pub channels: u16,
    pub device: Arc<AudioDevice>,
}

/// The delivery format: AAC-LC in MP4, playable everywhere the app shows audio.
/// Lossy, so it is only ever written once, over the finished recording.
pub const MP4_AAC_OUTPUT_ARGS: [&str; 8] = [
    "-c:a",
    "aac",
    "-b:a",
    "192k", // Increased from 64k for better audio quality (especially for speech)
    "-profile:a",
    "aac_low", // Use AAC-LC profile for better compatibility
    "-movflags",
    "+faststart", // Optimize for web streaming
];

/// The format for pieces that will later be joined into one recording.
///
/// FLAC because the join has to be seamless. A lossy codec pads every file it
/// writes out to a whole frame and prepends its own encoder delay, and joining
/// such files leaves that padding inside the recording: a few tens of
/// milliseconds of dead air at every seam, in the middle of whatever was being
/// said. FLAC stores the exact sample count, so the pieces meet with nothing
/// added between them.
pub const FLAC_OUTPUT_ARGS: [&str; 4] = ["-c:a", "flac", "-sample_fmt", "s16"];

/// Encode raw interleaved `f32` samples into the delivery format.
pub fn encode_single_audio(
    data: &[u8],
    sample_rate: u32,
    channels: u16,
    output_path: &PathBuf,
) -> anyhow::Result<()> {
    encode_raw_audio(data, sample_rate, channels, &MP4_AAC_OUTPUT_ARGS, "mp4", output_path)
}

/// Encode raw interleaved `f32` samples with the given output codec arguments.
pub fn encode_raw_audio(
    data: &[u8],
    sample_rate: u32,
    channels: u16,
    output_args: &[&str],
    format: &str,
    output_path: &PathBuf,
) -> anyhow::Result<()> {
    debug!("Starting FFmpeg process for {} bytes of audio data", data.len());

    if data.is_empty() {
        return Err(anyhow::anyhow!("No audio data provided for encoding"));
    }

    let ffmpeg_path = find_ffmpeg_path().ok_or_else(|| {
        anyhow::anyhow!("FFmpeg not found. Please install FFmpeg to save recordings.")
    })?;
    let output_path = output_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Recording output path is not valid UTF-8"))?;

    debug!("Using FFmpeg at: {:?}", ffmpeg_path);

    let mut command = Command::new(ffmpeg_path);
    command
        .args([
            "-f",
            "f32le",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            &channels.to_string(),
            "-i",
            "pipe:0",
        ])
        .args(output_args)
        .args(["-f", format, output_path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Hide console window on Windows to prevent CMD popup during recording
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    debug!("FFmpeg command: {:?}", command);

    #[allow(clippy::zombie_processes)]
    let mut ffmpeg = command
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn FFmpeg process: {}", e))?;
    debug!("FFmpeg process spawned");
    let mut stdin = ffmpeg
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open FFmpeg stdin"))?;

    stdin.write_all(data)?;

    debug!("Dropping stdin");
    drop(stdin);
    debug!("Waiting for FFmpeg process to exit");
    let output = ffmpeg
        .wait_with_output()
        .map_err(|e| anyhow::anyhow!("Failed while waiting for FFmpeg: {}", e))?;
    let status = output.status;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    debug!("FFmpeg process exited with status: {}", status);
    debug!("FFmpeg stdout: {}", stdout);
    debug!("FFmpeg stderr: {}", stderr);

    if !status.success() {
        error!("FFmpeg process failed with status: {}", status);
        error!("FFmpeg stderr: {}", stderr);
        return Err(anyhow::anyhow!(
            "FFmpeg process failed with status: {}",
            status
        ));
    }

    Ok(())
}
