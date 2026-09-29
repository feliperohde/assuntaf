//! A short piece of a meeting's recording, so the transcript can play back
//! exactly what was said in one line.
//!
//! Only the requested range is decoded: the reader seeks to the start and stops
//! at the end, so this stays cheap on long recordings.

use anyhow::{anyhow, Result};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

/// Heard a little before and after the line, so words at the edges are not cut.
pub const PADDING_SECONDS: f64 = 0.3;
/// Longest clip served at once.
pub const MAX_SECONDS: f64 = 90.0;

/// Mono samples of [start, end) seconds of an audio file, at the file's rate.
pub fn decode_range(path: &Path, start: f64, end: f64) -> Result<(Vec<f32>, u32)> {
    if !(end > start) {
        return Err(anyhow!("Empty time range"));
    }
    let file = File::open(path)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| anyhow!("Unsupported audio file {}: {}", path.display(), e))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("No audio track in {}", path.display()))?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let rate = params.sample_rate.ok_or_else(|| anyhow!("Unknown sample rate"))?;
    let time_base = params.time_base.unwrap_or_else(|| TimeBase::new(1, rate));
    let mut decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default())?;

    // A failed seek (unseekable file) leaves the reader at the beginning,
    // which still works: packets before the start are skipped below.
    if start > 0.0 {
        let _ = format.seek(SeekMode::Accurate, SeekTo::Time { time: Time::from(start), track_id: Some(track_id) });
        decoder.reset();
    }

    let mut out = Vec::with_capacity(((end - start) * rate as f64) as usize);
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let packet_time = time_base.calc_time(packet.ts());
        let packet_start = packet_time.seconds as f64 + packet_time.frac;
        if packet_start >= end {
            break;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        for (i, frame) in buffer.samples().chunks(channels).enumerate() {
            let t = packet_start + i as f64 / rate as f64;
            if t < start {
                continue;
            }
            if t >= end {
                break;
            }
            out.push(frame.iter().sum::<f32>() / channels as f32);
        }
    }
    Ok((out, rate))
}

/// 16-bit PCM WAV bytes.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}

/// The padded, capped range actually served for a transcript line.
pub fn clip_range(start: f64, end: Option<f64>) -> (f64, f64) {
    let start = start.max(0.0);
    // Lines without an end time: play a few seconds
    let end = end.filter(|e| *e > start).unwrap_or(start + 8.0);
    let from = (start - PADDING_SECONDS).max(0.0);
    let to = (end + PADDING_SECONDS).min(from + MAX_SECONDS);
    (from, to)
}

pub mod commands {
    use super::*;
    use crate::state::AppState;

    /// WAV bytes of the recording between `start` and `end` seconds (one
    /// transcript line), sent as a raw ArrayBuffer.
    #[tauri::command]
    pub async fn get_meeting_audio_clip(
        state: tauri::State<'_, AppState>,
        meeting_id: String,
        start: f64,
        end: Option<f64>,
    ) -> Result<tauri::ipc::Response, String> {
        let path = crate::diarization::service::meeting_audio(state.db_manager.pool(), &meeting_id)
            .await
            .map_err(|e| e.to_string())?;
        let (from, to) = clip_range(start, end);
        let wav = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let (samples, rate) = decode_range(&path, from, to)?;
            if samples.is_empty() {
                return Err(anyhow!("No audio at {:.1}s in this recording", from));
            }
            Ok(encode_wav(&samples, rate))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            log::warn!("Audio clip for meeting {} failed: {}", meeting_id, e);
            e.to_string()
        })?;
        Ok(tauri::ipc::Response::new(wav))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_file(seconds: f64, rate: u32) -> std::path::PathBuf {
        // Amplitude grows with time so a clip's position can be checked
        let samples: Vec<f32> = (0..(seconds * rate as f64) as usize)
            .map(|i| {
                let t = i as f64 / rate as f64;
                ((t / seconds) * 0.9 * (i as f64 * 0.05).sin()) as f32
            })
            .collect();
        let path = std::env::temp_dir().join(format!("clip-{}.wav", uuid::Uuid::new_v4()));
        std::fs::write(&path, encode_wav(&samples, rate)).unwrap();
        path
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn decodes_only_the_requested_range() {
        let path = tone_file(10.0, 16_000);
        let (clip, rate) = decode_range(&path, 6.0, 8.0).unwrap();
        assert_eq!(rate, 16_000);
        let len = clip.len() as i64;
        assert!((len - 32_000).abs() <= 1_600, "got {len} samples");
        // 6–8 s of a ramp that reaches 0.9 at 10 s
        let p = peak(&clip);
        assert!(p > 0.5 && p < 0.75, "peak {p}");

        let (start, _) = decode_range(&path, 0.0, 1.0).unwrap();
        assert!(peak(&start) < 0.12);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pads_and_caps_ranges() {
        assert_eq!(clip_range(10.0, Some(12.0)), (9.7, 12.3));
        assert_eq!(clip_range(0.1, Some(2.0)), (0.0, 2.3));
        assert_eq!(clip_range(5.0, None), (4.7, 13.3));
        let (from, to) = clip_range(0.0, Some(500.0));
        assert_eq!((from, to), (0.0, MAX_SECONDS));
    }

    #[test]
    fn empty_range_is_an_error() {
        assert!(decode_range(Path::new("/nonexistent.wav"), 3.0, 3.0).is_err());
    }
}
