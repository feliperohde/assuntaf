//! Offline speaker diarization of a recorded meeting:
//! pyannote segmentation-3.0 (speech activity and speaker changes) → WeSpeaker
//! speaker embeddings per single-speaker segment → agglomerative clustering
//! (optionally to a known speaker count) → speaker turns.
//! Models are the ONNX exports published with pyannote-rs.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use ndarray::{Array1, Axis};
use ort::session::{builder::GraphOptimizationLevel, Session};
use pyannote_rs::EmbeddingExtractor;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use super::cluster::{
    cluster_speakers, labeled_frames_to_segments, ClusterOptions, LabeledSegment, SpeakerTurn, WINDOW_SAMPLES,
};

pub const SAMPLE_RATE: u32 = 16_000;
pub const SEGMENTATION_MODEL: &str = "segmentation-3.0.onnx";
/// WeSpeaker ResNet34-LM: separates similar voices much better than the CAM++
/// model pyannote-rs ships (on a 4-speaker test recording CAM++ found 7 people
/// or missed speaker changes, depending on the threshold).
pub const EMBEDDING_MODEL: &str = "wespeaker_en_voxceleb_resnet34_LM.onnx";
const SEGMENTATION_URL: &str = "https://github.com/thewh1teagle/pyannote-rs/releases/download/v0.1.0/segmentation-3.0.onnx";
const EMBEDDING_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx";

/// Speech closer than this is joined into one segment.
const MAX_GAP_SECONDS: f64 = 0.3;
/// Segments shorter than this carry too little voice for a reliable embedding;
/// they get the speaker of the nearest longer segment.
const MIN_EMBED_SECONDS: f64 = 0.8;
/// Segments longer than this are split so one segment rarely spans two speakers.
const MAX_SEGMENT_SECONDS: f64 = 6.0;
/// Average-linkage cosine similarity above which two clusters are the same speaker.
/// Tuned on a real 4-speaker recording: 0.2–0.3 finds the 4 speakers, 0.4 starts
/// splitting one of them and 0.6 finds 9 (pyannote's pipeline, with the same
/// embedding family, also merges down to ≈0.3).
pub const CLUSTER_THRESHOLD: f32 = 0.3;

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
    // Replaced by EMBEDDING_MODEL; its vectors are not comparable anyway
    let _ = tokio::fs::remove_file(dir.join("wespeaker_en_voxceleb_CAM++.onnx")).await;
    for (name, url) in [(SEGMENTATION_MODEL, SEGMENTATION_URL), (EMBEDDING_MODEL, EMBEDDING_URL)] {
        let target = dir.join(name);
        if target.exists() {
            continue;
        }
        log::info!("Diarization: downloading {}", url);
        let response = reqwest::get(url).await?.error_for_status()?;
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

/// Local speakers per model window (segmentation-3.0 tracks up to 3).
const LOCAL_SPEAKERS: u32 = 3;

/// Maps a segmentation-3.0 powerset class to the local speakers it contains:
/// 0 = silence, 1..=3 = one speaker, 4..=6 = two overlapping speakers.
fn powerset_speakers(class: usize) -> &'static [u32] {
    match class {
        1 => &[0],
        2 => &[1],
        3 => &[2],
        4 => &[0, 1],
        5 => &[0, 2],
        6 => &[1, 2],
        _ => &[],
    }
}

/// Runs the segmentation model over 10 s windows and returns, per output frame,
/// the speaking local speaker as `window * 3 + local` (None = silence). During
/// overlap the speaker who was already talking keeps the frame, so turns are
/// not chopped by interjections. Unlike pyannote-rs' iterator this never stops
/// at a silent window.
fn speaker_frames(session: &mut Session, samples: &[i16]) -> Result<Vec<Option<u32>>> {
    let mut frames = Vec::new();
    let mut start = 0;
    let mut window_index: u32 = 0;
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
        let mut previous: Option<u32> = None;
        for frame in data.chunks_exact(classes).take(real_frames) {
            let best = frame
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            let speakers = powerset_speakers(best);
            let local = match speakers {
                [] => None,
                [only] => Some(*only),
                [first, ..] => previous.filter(|p| speakers.contains(p)).or(Some(*first)),
            };
            previous = local.or(previous);
            frames.push(local.map(|l| window_index * LOCAL_SPEAKERS + l));
        }
        start = end;
        window_index += 1;
    }
    Ok(frames)
}

#[derive(Debug, Clone, Default)]
pub struct DiarizeOptions {
    /// Number of people in the meeting, when the user knows it.
    pub num_speakers: Option<usize>,
    /// Overrides CLUSTER_THRESHOLD (tuning and tests).
    pub cluster_threshold: Option<f32>,
}

/// Splits long segments into pieces of at most MAX_SEGMENT_SECONDS.
fn split_long(segments: Vec<LabeledSegment>) -> Vec<LabeledSegment> {
    let mut out = Vec::new();
    for segment in segments {
        let (s, e) = (segment.start, segment.end);
        let pieces = ((e - s) / MAX_SEGMENT_SECONDS).ceil().max(1.0) as usize;
        let step = (e - s) / pieces as f64;
        for i in 0..pieces {
            out.push(LabeledSegment { start: s + i as f64 * step, end: s + (i + 1) as f64 * step, label: segment.label });
        }
    }
    out
}

/// Clusters below this share of the meeting's speech (capped at
/// MIN_SPEAKER_SECONDS) are treated as strays of another speaker.
const MIN_SPEAKER_SHARE: f64 = 0.05;
const MIN_SPEAKER_SECONDS: f64 = 8.0;

/// Diarizes 16 kHz mono audio in [-1, 1]. CPU-bound: call from a blocking task.
pub fn diarize(audio: &[f32], models: &Path, options: &DiarizeOptions) -> Result<DiarizationResult> {
    let samples: Vec<i16> = audio.iter().map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();

    let mut segmentation = create_session(&models.join(SEGMENTATION_MODEL))?;
    let frames = speaker_frames(&mut segmentation, &samples)?;
    let segments = split_long(labeled_frames_to_segments(&frames, SAMPLE_RATE, MAX_GAP_SECONDS, 0.25));
    log::info!("Diarization: {} speech segments", segments.len());
    if segments.is_empty() {
        return Ok(DiarizationResult { turns: Vec::new(), centroids: Vec::new(), speaking_seconds: Vec::new() });
    }

    let mut extractor = EmbeddingExtractor::new(models.join(EMBEDDING_MODEL))
        .map_err(|e| anyhow!("Failed to load speaker model: {e}"))?;
    let mut embedded: Vec<usize> = Vec::new();
    let mut embeddings: Vec<Vec<f32>> = Vec::new();
    let mut weights: Vec<f64> = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let (seg_start, seg_end) = (segment.start, segment.end);
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
                weights.push(seg_end - seg_start);
            }
            Err(err) => log::warn!("Diarization: embedding failed for {:.1}-{:.1}s: {}", seg_start, seg_end, err),
        }
    }
    if embeddings.is_empty() {
        return Ok(DiarizationResult { turns: Vec::new(), centroids: Vec::new(), speaking_seconds: Vec::new() });
    }

    let total_speech: f64 = weights.iter().sum();
    let (labels, centroids) = cluster_speakers(
        &embeddings,
        &weights,
        &ClusterOptions {
            threshold: options.cluster_threshold.unwrap_or(CLUSTER_THRESHOLD),
            num_speakers: options.num_speakers.filter(|&k| k > 0),
            min_cluster_weight: (total_speech * MIN_SPEAKER_SHARE).min(MIN_SPEAKER_SECONDS),
        },
    );

    // Label every segment: embedded ones by cluster; short ones like an embedded
    // segment of the same local speaker (same model window), else the nearest one
    let mut turns = Vec::with_capacity(segments.len());
    for (i, segment) in segments.iter().enumerate() {
        let speaker = match embedded.iter().position(|&j| j == i) {
            Some(k) => labels[k],
            None => embedded
                .iter()
                .enumerate()
                .min_by_key(|(_, &j)| (segments[j].label != segment.label, j.abs_diff(i)))
                .map(|(k, _)| labels[k])
                .unwrap_or(0),
        };
        turns.push(SpeakerTurn { start: segment.start, end: segment.end, speaker });
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
        let segment = |start, end, label| LabeledSegment { start, end, label };
        let pieces = split_long(vec![segment(0.0, 13.0, 4), segment(20.0, 21.0, 7)]);
        assert_eq!(pieces.len(), 4);
        assert!((pieces[2].end - 13.0).abs() < 1e-9);
        assert_eq!(pieces[2].label, 4);
        assert!(pieces.iter().all(|p| p.end - p.start <= MAX_SEGMENT_SECONDS + 1e-9));
    }

    #[test]
    fn powerset_classes_map_to_local_speakers() {
        assert!(powerset_speakers(0).is_empty());
        assert_eq!(powerset_speakers(2), &[1]);
        assert_eq!(powerset_speakers(5), &[0, 2]);
    }

    /// End-to-end check with the real models on a synthetic dialogue produced by
    /// scripts/make_diarization_fixture.sh (voice A speaks utterances 1, 3, 5 and
    /// voice B utterances 2, 4, 6). Needs DIARIZATION_MODELS_DIR (both ONNX files)
    /// and DIARIZATION_TEST_WAV; skipped otherwise.
    /// Real recording with 4 speakers (sherpa-onnx's 0-four-speakers-zh.wav, from
    /// its speaker-segmentation-models release) in DIARIZATION_EVAL_WAV.
    #[test]
    fn finds_the_speakers_of_a_real_recording_when_available() {
        let (Ok(models), Ok(wav)) = (std::env::var("DIARIZATION_MODELS_DIR"), std::env::var("DIARIZATION_EVAL_WAV")) else {
            eprintln!("skipping: DIARIZATION_MODELS_DIR / DIARIZATION_EVAL_WAV not set");
            return;
        };
        let (samples, _) = pyannote_rs::read_wav(&wav).unwrap();
        let audio: Vec<f32> = samples.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
        let result = diarize(&audio, Path::new(&models), &DiarizeOptions::default()).unwrap();
        assert_eq!(result.centroids.len(), 4);
        let speaker_at = |time: f64| result.turns.iter().find(|t| t.start <= time && time <= t.end).map(|t| t.speaker);
        // Speaker changes around 6.8 s and 13.7 s
        assert_ne!(speaker_at(3.0), speaker_at(9.0));
        assert_ne!(speaker_at(9.0), speaker_at(15.0));

        let pinned = DiarizeOptions { num_speakers: Some(3), ..Default::default() };
        assert_eq!(diarize(&audio, Path::new(&models), &pinned).unwrap().centroids.len(), 3);
    }

    /// Prints the speakers found at several thresholds for a real recording
    /// (DIARIZATION_EVAL_WAV); for tuning, not an assertion.
    #[test]
    #[ignore]
    fn threshold_sweep() {
        let (Ok(models), Ok(wav)) = (std::env::var("DIARIZATION_MODELS_DIR"), std::env::var("DIARIZATION_EVAL_WAV")) else {
            return;
        };
        let (samples, _) = pyannote_rs::read_wav(&wav).unwrap();
        let audio: Vec<f32> = samples.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
        for threshold in [0.2, 0.3, 0.4, 0.5, 0.6] {
            let options = DiarizeOptions { cluster_threshold: Some(threshold), ..Default::default() };
            let result = diarize(&audio, Path::new(&models), &options).unwrap();
            let turns: Vec<String> = result.turns.iter().map(|t| format!("{:.1}-{:.1}:{}", t.start, t.end, t.speaker)).collect();
            eprintln!("threshold {threshold}: {} speakers {:?}\n  {}", result.centroids.len(),
                result.speaking_seconds.iter().map(|s| s.round()).collect::<Vec<_>>(), turns.join(" "));
        }
    }

    #[test]
    fn diarizes_synthetic_dialogue_when_models_available() {
        let (Ok(models), Ok(wav)) = (std::env::var("DIARIZATION_MODELS_DIR"), std::env::var("DIARIZATION_TEST_WAV")) else {
            eprintln!("skipping: DIARIZATION_MODELS_DIR / DIARIZATION_TEST_WAV not set");
            return;
        };
        let (samples, rate) = pyannote_rs::read_wav(&wav).unwrap();
        assert_eq!(rate, SAMPLE_RATE);
        let audio: Vec<f32> = samples.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
        let result = diarize(&audio, Path::new(&models), &DiarizeOptions::default()).unwrap();
        for t in &result.turns {
            eprintln!("{:6.2}-{:6.2} speaker {}", t.start, t.end, t.speaker);
        }
        let speaker_at = |time: f64| {
            result.turns.iter().find(|t| t.start <= time && time <= t.end).map(|t| t.speaker)
        };
        // Midpoints of utterances 1, 3, 5 (voice A) and 2, 4, 6 (voice B) in the fixture
        let a = speaker_at(3.0).expect("speech at 3s");
        assert_eq!(speaker_at(19.0), Some(a));
        assert_eq!(speaker_at(32.0), Some(a));
        let b = speaker_at(25.5).expect("speech at 25.5s");
        assert_ne!(b, a);
        assert_eq!(speaker_at(12.0), Some(b));
        assert_eq!(speaker_at(38.0), Some(b));
        assert_eq!(result.centroids.len(), 2);
        assert_eq!(result.speaking_seconds.len(), result.centroids.len());
    }
}
