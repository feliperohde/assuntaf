//! Pure diarization logic: speech segments from segmentation frames, speaker
//! clustering, and mapping speaker turns onto transcript segments.

/// Samples per 10 s window at 16 kHz, the segmentation model's input size.
pub const WINDOW_SAMPLES: usize = 160_000;
/// Model output frame step, and the offset of the first frame, in samples
/// (values from the pyannote segmentation-3.0 receptive field, as used by pyannote-rs).
pub const FRAME_STEP: usize = 270;
pub const FRAME_START: usize = 721;

/// Converts per-frame "is speech" flags (across all windows, in order) into
/// (start, end) seconds. Short gaps are bridged and very short blips dropped.
pub fn frames_to_segments(
    speech: &[bool],
    sample_rate: u32,
    max_gap_seconds: f64,
    min_duration_seconds: f64,
) -> Vec<(f64, f64)> {
    let frame_time = |i: usize| (FRAME_START + i * FRAME_STEP) as f64 / sample_rate as f64;
    let mut raw: Vec<(f64, f64)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &is_speech) in speech.iter().enumerate() {
        match (is_speech, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                raw.push((frame_time(s), frame_time(i)));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        raw.push((frame_time(s), frame_time(speech.len())));
    }

    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in raw {
        match merged.last_mut() {
            Some(last) if s - last.1 <= max_gap_seconds => last.1 = e,
            _ => merged.push((s, e)),
        }
    }
    merged.retain(|(s, e)| e - s >= min_duration_seconds);
    merged
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        v.to_vec()
    } else {
        v.iter().map(|x| x / norm).collect()
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Agglomerative clustering with average linkage on cosine similarity: clusters
/// are merged while the most similar pair is above `threshold`. Returns one label
/// per embedding (0-based, ordered by first appearance) and the normalized
/// centroid of each cluster.
pub fn cluster_embeddings(embeddings: &[Vec<f32>], threshold: f32) -> (Vec<usize>, Vec<Vec<f32>>) {
    let n = embeddings.len();
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let normalized: Vec<Vec<f32>> = embeddings.iter().map(|e| normalize(e)).collect();
    // Pairwise similarities between items
    let sim: Vec<Vec<f32>> = (0..n)
        .map(|i| (0..n).map(|j| cosine(&normalized[i], &normalized[j])).collect())
        .collect();

    let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for a in 0..clusters.len() {
            for b in (a + 1)..clusters.len() {
                let mut total = 0.0;
                for &i in &clusters[a] {
                    for &j in &clusters[b] {
                        total += sim[i][j];
                    }
                }
                let avg = total / (clusters[a].len() * clusters[b].len()) as f32;
                if best.map_or(true, |(_, _, s)| avg > s) {
                    best = Some((a, b, avg));
                }
            }
        }
        match best {
            Some((a, b, s)) if s >= threshold => {
                let merged = clusters.remove(b);
                clusters[a].extend(merged);
            }
            _ => break,
        }
    }

    // Label clusters in order of their first member so "Speaker 1" speaks first
    clusters.sort_by_key(|c| *c.iter().min().unwrap());
    let mut labels = vec![0; n];
    let mut centroids = Vec::with_capacity(clusters.len());
    for (label, members) in clusters.iter().enumerate() {
        let dims = normalized[members[0]].len();
        let mut centroid = vec![0.0f32; dims];
        for &m in members {
            labels[m] = label;
            for (c, v) in centroid.iter_mut().zip(&normalized[m]) {
                *c += v;
            }
        }
        centroids.push(normalize(&centroid));
    }
    (labels, centroids)
}

/// A span of speech attributed to one speaker (index into the clusters).
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerTurn {
    pub start: f64,
    pub end: f64,
    pub speaker: usize,
}

/// For each transcript segment (start, end), the speaker with the most overlap,
/// or the nearest turn in time when nothing overlaps (within `max_distance`).
pub fn assign_speakers(
    segments: &[(Option<f64>, Option<f64>)],
    turns: &[SpeakerTurn],
    max_distance: f64,
) -> Vec<Option<usize>> {
    segments
        .iter()
        .map(|&(start, end)| {
            let start = start?;
            let end = end.unwrap_or(start).max(start);
            let mut overlap_by_speaker: Vec<(usize, f64)> = Vec::new();
            for turn in turns {
                let overlap = end.min(turn.end) - start.max(turn.start);
                if overlap > 0.0 {
                    match overlap_by_speaker.iter_mut().find(|(s, _)| *s == turn.speaker) {
                        Some(entry) => entry.1 += overlap,
                        None => overlap_by_speaker.push((turn.speaker, overlap)),
                    }
                }
            }
            if let Some((speaker, _)) = overlap_by_speaker
                .iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            {
                return Some(*speaker);
            }
            turns
                .iter()
                .map(|t| {
                    let distance = if end < t.start { t.start - end } else { start - t.end };
                    (t.speaker, distance.max(0.0))
                })
                .filter(|(_, d)| *d <= max_distance)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(s, _)| s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_become_merged_segments() {
        // 16 kHz: a frame is 270 samples ≈ 16.9 ms
        let mut speech = vec![false; 10];
        speech.extend(vec![true; 100]); // ~1.7 s
        speech.extend(vec![false; 5]); // short gap → bridged
        speech.extend(vec![true; 60]);
        speech.extend(vec![false; 200]); // long gap
        speech.extend(vec![true; 3]); // blip → dropped
        speech.extend(vec![false; 10]);
        let segments = frames_to_segments(&speech, 16_000, 0.3, 0.25);
        assert_eq!(segments.len(), 1);
        let (s, e) = segments[0];
        assert!((s - (721.0 + 10.0 * 270.0) / 16_000.0).abs() < 1e-9);
        assert!((e - (721.0 + 175.0 * 270.0) / 16_000.0).abs() < 1e-9);
    }

    #[test]
    fn open_segment_at_end_is_closed() {
        let segments = frames_to_segments(&[false, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true], 16_000, 0.3, 0.1);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn clustering_groups_similar_embeddings() {
        let a1 = vec![1.0, 0.1, 0.0];
        let a2 = vec![0.9, 0.2, 0.0];
        let b1 = vec![0.0, 0.1, 1.0];
        let b2 = vec![0.1, 0.0, 0.9];
        let (labels, centroids) = cluster_embeddings(&[b1, a1, b2, a2], 0.7);
        assert_eq!(labels, vec![0, 1, 0, 1]); // first seen speaker is 0
        assert_eq!(centroids.len(), 2);
        assert!(cosine(&centroids[0], &[0.0, 0.0, 1.0]) > 0.9);

        let (labels, _) = cluster_embeddings(&[vec![1.0, 0.0], vec![0.0, 1.0]], 0.7);
        assert_eq!(labels, vec![0, 1]);
        assert!(cluster_embeddings(&[], 0.5).0.is_empty());
    }

    #[test]
    fn transcript_segments_get_majority_or_nearest_speaker() {
        let turns = vec![
            SpeakerTurn { start: 0.0, end: 5.0, speaker: 0 },
            SpeakerTurn { start: 5.0, end: 6.0, speaker: 1 },
            SpeakerTurn { start: 10.0, end: 20.0, speaker: 1 },
        ];
        let segments = vec![
            (Some(1.0), Some(5.5)),   // mostly speaker 0
            (Some(4.8), Some(6.0)),   // mostly speaker 1
            (Some(7.0), Some(8.0)),   // no overlap, nearest is 1 (starts 10.0? 5-6 ends at 6 → 1.0s away)
            (Some(40.0), Some(41.0)), // too far from any turn
            (None, None),             // untimed
        ];
        assert_eq!(assign_speakers(&segments, &turns, 3.0), vec![Some(0), Some(1), Some(1), None, None]);
    }
}
