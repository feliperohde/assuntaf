//! Offline speaker diarization of a recorded meeting:
//! pyannote segmentation-3.0 (speech activity) → CAM++ speaker embeddings per
//! speech segment → agglomerative clustering → speaker turns.
//! Models are the ONNX exports published with pyannote-rs.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use ndarray::{Array1, Axis};
use ort::session::{builder::GraphOptimizationLevel, Session};
use pyannote_rs::EmbeddingExtractor;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use super::cluster::{cluster_embeddings, frames_to_segments, SpeakerTurn, WINDOW_SAMPLES};

pub const SAMPLE_RATE: u32 = 16_000;
pub const SEGMENTATION_MODEL: &str = "segmentation-3.0.onnx";
pub const EMBEDDING_MODEL: &str = "wespeaker_en_voxceleb_CAM++.onnx";
const MODEL_BASE_URL: &str = "https://github.com/thewh1teagle/pyannote-rs/releases/download/v0.1.0";

/// Speech closer than this is joined into one segment.
const MAX_GAP_SECONDS: f64 = 0.3;
/// Segments shorter than this carry too little voice for a reliable embedding;
/// they get the speaker of the nearest longer segment.
const MIN_EMBED_SECONDS: f64 = 0.8;
/// Segments longer than this are split so one segment rarely spans two speakers.
const MAX_SEGMENT_SECONDS: f64 = 6.0;
/// Average-linkage cosine similarity above which two clusters are the same speaker.
/// Errs towards splitting: two labels for one person can be merged by assigning
/// both to the same member, while two people merged into one label cannot be split.
pub const CLUSTER_THRESHOLD: f32 = 0.6;

#[derive(Debug, Clone)]
pub struct DiarizationResult {
    pub turns: Vec<SpeakerTurn>,
    /// Normalized voice centroid per speaker (index = SpeakerTurn::speaker).
    pub centroids: Vec<Vec<f32>>,
    /// Seconds of speech per speaker.
    pub speaking_seconds: Vec<f64>,
}

pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("models").join("diarization")
}

pub fn models_present(dir: &Path) -> bool {
    dir.join(SEGMENTATION_MODEL).exists() && dir.join(EMBEDDING_MODEL).exists()
}

/// Downloads any missing model into `dir` (written to a temp file, then renamed).
pub async fn ensure_models(dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    for name in [SEGMENTATION_MODEL, EMBEDDING_MODEL] {
        let target = dir.join(name);
        if target.exists() {
            continue;
        }
        let url = format!("{MODEL_BASE_URL}/{}", name.replace('+', "%2B"));
        log::info!("Diarization: downloading {}", url);
        let response = reqwest::get(&url).await?.error_for_status()?;
        let partial = dir.join(format!("{name}.part"));
        let mut file = tokio::fs::File::create(&partial).await?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk?).await?;
        }
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&partial, &target).await?;
    }
    Ok(())
}

fn create_session(path: &Path) -> Result<Session> {
    Ok(Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(2)?
        .commit_from_file(path)
        .with_context(|| format!("Failed to load {}", path.display()))?)
}

/// Runs the segmentation model over 10 s windows and returns one speech flag per
/// output frame. Unlike pyannote-rs' iterator this never stops at a silent window.
fn speech_frames(session: &mut Session, samples: &[i16]) -> Result<Vec<bool>> {
    let mut frames = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let end = (start + WINDOW_SAMPLES).min(samples.len());
        let mut window: Vec<f32> = samples[start..end].iter().map(|&s| s as f32).collect();
        window.resize(WINDOW_SAMPLES, 0.0);
        let input = Array1::from_vec(window).insert_axis(Axis(0)).insert_axis(Axis(0));

        let outputs = session.run(ort::inputs![ort::value::TensorRef::from_array_view(input.view().into_dyn())?])?;
        let (shape, data) = outputs
            .get("output")
            .ok_or_else(|| anyhow!("segmentation output missing"))?
            .try_extract_tensor::<f32>()?;
        // shape: [1, frames, classes]; class 0 = no speech (powerset encoding)
        let classes = shape[shape.len() - 1] as usize;
        let window_frames = data.len() / classes;
        // Only keep frames that cover real (non-padded) audio
        let real_frames = ((end - start) * window_frames).div_ceil(WINDOW_SAMPLES);
        for frame in data.chunks_exact(classes).take(real_frames) {
            let best = frame
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            frames.push(best != 0);
        }
        start = end;
    }
    Ok(frames)
}

/// Splits long segments into pieces of at most MAX_SEGMENT_SECONDS.
fn split_long(segments: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for (s, e) in segments {
        let pieces = ((e - s) / MAX_SEGMENT_SECONDS).ceil().max(1.0) as usize;
        let step = (e - s) / pieces as f64;
        for i in 0..pieces {
            out.push((s + i as f64 * step, s + (i + 1) as f64 * step));
        }
    }
    out
}

/// Diarizes 16 kHz mono audio in [-1, 1]. CPU-bound: call from a blocking task.
pub fn diarize(audio: &[f32], models: &Path) -> Result<DiarizationResult> {
    let samples: Vec<i16> = audio.iter().map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();

    let mut segmentation = create_session(&models.join(SEGMENTATION_MODEL))?;
    let frames = speech_frames(&mut segmentation, &samples)?;
    let segments = split_long(frames_to_segments(&frames, SAMPLE_RATE, MAX_GAP_SECONDS, 0.25));
    log::info!("Diarization: {} speech segments", segments.len());
    if segments.is_empty() {
        return Ok(DiarizationResult { turns: Vec::new(), centroids: Vec::new(), speaking_seconds: Vec::new() });
    }

    let mut extractor = EmbeddingExtractor::new(models.join(EMBEDDING_MODEL))
        .map_err(|e| anyhow!("Failed to load speaker model: {e}"))?;
    let mut embedded: Vec<usize> = Vec::new();
    let mut embeddings: Vec<Vec<f32>> = Vec::new();
    for (i, &(seg_start, seg_end)) in segments.iter().enumerate() {
        if seg_end - seg_start < MIN_EMBED_SECONDS {
            continue;
        }
        let from = ((seg_start * SAMPLE_RATE as f64) as usize).min(samples.len());
        let to = ((seg_end * SAMPLE_RATE as f64) as usize).min(samples.len());
        if to <= from {
            continue;
        }
        match extractor.compute(&samples[from..to]) {
            Ok(embedding) => {
                embedded.push(i);
                embeddings.push(embedding.collect());
            }
            Err(err) => log::warn!("Diarization: embedding failed for {:.1}-{:.1}s: {}", seg_start, seg_end, err),
        }
    }
    if embeddings.is_empty() {
        return Ok(DiarizationResult { turns: Vec::new(), centroids: Vec::new(), speaking_seconds: Vec::new() });
    }

    let (labels, centroids) = cluster_embeddings(&embeddings, CLUSTER_THRESHOLD);

    // Label every segment: embedded ones by cluster, short ones by nearest embedded segment
    let mut turns = Vec::with_capacity(segments.len());
    for (i, &(start, end)) in segments.iter().enumerate() {
        let speaker = match embedded.iter().position(|&j| j == i) {
            Some(k) => labels[k],
            None => {
                embedded
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, &j)| j.abs_diff(i))
                    .map(|(k, _)| labels[k])
                    .unwrap_or(0)
            }
        };
        turns.push(SpeakerTurn { start, end, speaker });
    }

    let mut speaking_seconds = vec![0.0; centroids.len()];
    for turn in &turns {
        speaking_seconds[turn.speaker] += turn.end - turn.start;
    }
    Ok(DiarizationResult { turns, centroids, speaking_seconds })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_segments_are_split_evenly() {
        let pieces = split_long(vec![(0.0, 13.0), (20.0, 21.0)]);
        assert_eq!(pieces.len(), 4);
        assert!((pieces[2].1 - 13.0).abs() < 1e-9);
        assert!(pieces.iter().all(|(s, e)| e - s <= MAX_SEGMENT_SECONDS + 1e-9));
    }

    /// End-to-end check with the real models on a synthetic dialogue produced by
    /// scripts/make_diarization_fixture.sh (voice A speaks utterances 1, 3, 5 and
    /// voice B utterances 2, 4, 6). Needs DIARIZATION_MODELS_DIR (both ONNX files)
    /// and DIARIZATION_TEST_WAV; skipped otherwise. Synthetic voices are not stable
    /// enough to assert B's utterances all cluster together, so only A is checked.
    #[test]
    fn diarizes_synthetic_dialogue_when_models_available() {
        let (Ok(models), Ok(wav)) = (std::env::var("DIARIZATION_MODELS_DIR"), std::env::var("DIARIZATION_TEST_WAV")) else {
            eprintln!("skipping: DIARIZATION_MODELS_DIR / DIARIZATION_TEST_WAV not set");
            return;
        };
        let (samples, rate) = pyannote_rs::read_wav(&wav).unwrap();
        assert_eq!(rate, SAMPLE_RATE);
        let audio: Vec<f32> = samples.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
        let result = diarize(&audio, Path::new(&models)).unwrap();
        for t in &result.turns {
            eprintln!("{:6.2}-{:6.2} speaker {}", t.start, t.end, t.speaker);
        }
        let speaker_at = |time: f64| {
            result.turns.iter().find(|t| t.start <= time && time <= t.end).map(|t| t.speaker)
        };
        // Midpoints of utterances 1, 3, 5 (voice A) and 4 (voice B) in the fixture
        let a = speaker_at(3.0).expect("speech at 3s");
        assert_eq!(speaker_at(19.0), Some(a));
        assert_eq!(speaker_at(32.0), Some(a));
        assert_ne!(speaker_at(25.5).expect("speech at 25.5s"), a);
        assert!(result.centroids.len() >= 2);
        assert_eq!(result.speaking_seconds.len(), result.centroids.len());
    }
}
