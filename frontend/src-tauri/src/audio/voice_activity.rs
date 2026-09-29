//! Who was talking, microphone or computer, over the course of a recording.
//!
//! The recording and the transcription both use the mixed stream, so a
//! transcript line does not know whether it came from the user (microphone) or
//! from the other participants (system audio). While mixing, the pipeline
//! records the level of each stream in 100 ms slices; the track is saved next to
//! the recording as `voice_activity.json` and later tells, for any time range,
//! how much of the sound came from the microphone.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

pub const FILE_NAME: &str = "voice_activity.json";
/// Resolution of the track.
pub const SLICE_SECONDS: f64 = 0.1;
/// Levels are stored as 0..=255 over this range of dBFS.
const FLOOR_DB: f64 = -60.0;

/// Mic share above which a line is the user's, below which it is someone else's.
pub const MIC_THRESHOLD: f64 = 0.65;
pub const SYSTEM_THRESHOLD: f64 = 0.35;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VoiceActivity {
    pub slice_seconds: f64,
    /// Level per slice, 0 = at or below -60 dBFS, 255 = 0 dBFS.
    pub mic: Vec<u8>,
    pub system: Vec<u8>,
}

static TRACK: Lazy<Mutex<VoiceActivity>> = Lazy::new(|| Mutex::new(VoiceActivity::new()));

fn level(samples: &[f32]) -> u8 {
    if samples.is_empty() {
        return 0;
    }
    let mean_square = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / samples.len() as f64;
    if mean_square <= 0.0 {
        return 0;
    }
    let db = 10.0 * mean_square.log10();
    (((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) * 255.0).round() as u8
}

fn power(level: u8) -> f64 {
    if level == 0 {
        return 0.0;
    }
    let db = FLOOR_DB + (level as f64 / 255.0) * -FLOOR_DB;
    10f64.powf(db / 10.0)
}

impl VoiceActivity {
    pub fn new() -> Self {
        Self { slice_seconds: SLICE_SECONDS, mic: Vec::new(), system: Vec::new() }
    }

    /// Appends the levels of one mixing window (both streams, same length).
    pub fn push_window(&mut self, mic: &[f32], system: &[f32], sample_rate: u32) {
        let slice = ((sample_rate as f64 * self.slice_seconds) as usize).max(1);
        let len = mic.len().max(system.len());
        let mut start = 0;
        while start < len {
            let end = (start + slice).min(len);
            self.mic.push(level(mic.get(start..end.min(mic.len())).unwrap_or(&[])));
            self.system.push(level(system.get(start..end.min(system.len())).unwrap_or(&[])));
            start = end;
        }
    }

    /// Share of the sound in [start, end) seconds that came from the microphone,
    /// or None when the range is silent or outside the track.
    pub fn mic_share(&self, start: f64, end: f64) -> Option<f64> {
        if self.slice_seconds <= 0.0 || end <= start {
            return None;
        }
        let from = (start / self.slice_seconds).floor().max(0.0) as usize;
        let to = ((end / self.slice_seconds).ceil() as usize).min(self.mic.len().min(self.system.len()));
        if from >= to {
            return None;
        }
        let mic: f64 = self.mic[from..to].iter().map(|l| power(*l)).sum();
        let system: f64 = self.system[from..to].iter().map(|l| power(*l)).sum();
        let total = mic + system;
        // Below about -50 dBFS on average: silence, no reliable answer
        if total / (to - from) as f64 <= 1e-5 {
            return None;
        }
        Some(mic / total)
    }

    /// "mic", "system" or "both" for a time range.
    pub fn source_of(&self, start: f64, end: f64) -> Option<&'static str> {
        self.mic_share(start, end).map(|share| {
            if share >= MIC_THRESHOLD {
                "mic"
            } else if share <= SYSTEM_THRESHOLD {
                "system"
            } else {
                "both"
            }
        })
    }

    pub fn load(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join(FILE_NAME)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string(self)?;
        let temp = folder.join(format!(".{FILE_NAME}.tmp"));
        std::fs::write(&temp, json)?;
        std::fs::rename(temp, folder.join(FILE_NAME))
    }
}

/// Starts a new track (recording start).
pub fn reset() {
    if let Ok(mut track) = TRACK.lock() {
        *track = VoiceActivity::new();
    }
}

/// Records one mixing window of the current recording.
pub fn record_window(mic: &[f32], system: &[f32], sample_rate: u32) {
    if let Ok(mut track) = TRACK.lock() {
        track.push_window(mic, system, sample_rate);
    }
}

/// Writes the current recording's track into its folder.
pub fn save_current(folder: &Path) -> std::io::Result<()> {
    let track = TRACK.lock().map(|t| t.clone()).unwrap_or_default();
    if track.mic.is_empty() {
        return Ok(());
    }
    track.save(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, samples: usize) -> Vec<f32> {
        (0..samples).map(|i| amplitude * ((i as f32) * 0.1).sin()).collect()
    }

    #[test]
    fn tells_who_is_talking_per_range() {
        let rate = 16_000;
        let window = rate as usize * 6 / 10; // 600 ms, like the mixer
        let mut track = VoiceActivity::new();
        // 0–1.8 s: user talks (loud mic, faint echo on system)
        for _ in 0..3 {
            track.push_window(&tone(0.3, window), &tone(0.01, window), rate);
        }
        // 1.8–3.6 s: someone else (system loud, echo on mic)
        for _ in 0..3 {
            track.push_window(&tone(0.02, window), &tone(0.4, window), rate);
        }
        // 3.6–4.2 s: silence
        track.push_window(&vec![0.0; window], &vec![0.0; window], rate);

        assert_eq!(track.mic.len(), 42); // 100 ms slices
        assert_eq!(track.source_of(0.2, 1.6), Some("mic"));
        assert_eq!(track.source_of(2.0, 3.4), Some("system"));
        assert_eq!(track.source_of(1.2, 2.4).is_some(), true);
        assert_eq!(track.source_of(3.7, 4.1), None);
        assert_eq!(track.source_of(10.0, 12.0), None);
    }

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("va-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut track = VoiceActivity::new();
        track.push_window(&tone(0.3, 1600), &tone(0.01, 1600), 16_000);
        track.save(&dir).unwrap();
        assert_eq!(VoiceActivity::load(&dir), Some(track));
        let _ = std::fs::remove_dir_all(dir);
    }
}
