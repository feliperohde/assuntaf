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
    let labels: Vec<Option<u32>> = speech.iter().map(|&s| s.then_some(0)).collect();
    labeled_frames_to_segments(&labels, sample_rate, max_gap_seconds, min_duration_seconds)
        .into_iter()
        .map(|s| (s.start, s.end))
        .collect()
}

/// A span of speech by one local speaker of the segmentation model. `label` is
/// only meaningful within one model window: the same person gets unrelated
/// labels in different windows (clustering links them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabeledSegment {
    pub start: f64,
    pub end: f64,
    pub label: u32,
}

/// Converts per-frame local-speaker labels (None = no speech) into segments.
/// A label change always starts a new segment, so each segment holds one voice;
/// short gaps within the same label are bridged and very short blips dropped.
pub fn labeled_frames_to_segments(
    labels: &[Option<u32>],
    sample_rate: u32,
    max_gap_seconds: f64,
    min_duration_seconds: f64,
) -> Vec<LabeledSegment> {
    let frame_time = |i: usize| (FRAME_START + i * FRAME_STEP) as f64 / sample_rate as f64;
    let mut raw: Vec<LabeledSegment> = Vec::new();
    let mut current: Option<(usize, u32)> = None;
    for (i, &label) in labels.iter().enumerate() {
        match (label, current) {
            (Some(l), None) => current = Some((i, l)),
            (Some(l), Some((s, cl))) if l != cl => {
                raw.push(LabeledSegment { start: frame_time(s), end: frame_time(i), label: cl });
                current = Some((i, l));
            }
            (None, Some((s, cl))) => {
                raw.push(LabeledSegment { start: frame_time(s), end: frame_time(i), label: cl });
                current = None;
            }
            _ => {}
        }
    }
    if let Some((s, cl)) = current {
        raw.push(LabeledSegment { start: frame_time(s), end: frame_time(labels.len()), label: cl });
    }

    let mut merged: Vec<LabeledSegment> = Vec::new();
    for segment in raw {
        match merged.last_mut() {
            Some(last) if last.label == segment.label && segment.start - last.end <= max_gap_seconds => {
                last.end = segment.end
            }
            _ => merged.push(segment),
        }
    }
    merged.retain(|s| s.end - s.start >= min_duration_seconds);
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
    let weights = vec![1.0; embeddings.len()];
    cluster_speakers(
        embeddings,
        &weights,
        &ClusterOptions { threshold, num_speakers: None, min_cluster_weight: 0.0 },
    )
}

#[derive(Debug, Clone)]
pub struct ClusterOptions {
    /// Average-linkage cosine similarity above which clusters are merged.
    pub threshold: f32,
    /// When known, merge until exactly this many speakers remain (threshold ignored).
    pub num_speakers: Option<usize>,
    /// Clusters whose total weight (seconds of speech) is below this are absorbed
    /// by the most similar larger cluster: a few stray segments (laughs, noise,
    /// crosstalk) are far more common than a person who barely speaks.
    pub min_cluster_weight: f64,
}

/// Agglomerative clustering (average linkage, cosine similarity) of speech
/// segment embeddings. `weights` are the segments' durations. Returns one label
/// per embedding (0-based, ordered by first appearance) and each cluster's
/// normalized centroid.
pub fn cluster_speakers(
    embeddings: &[Vec<f32>],
    weights: &[f64],
    options: &ClusterOptions,
) -> (Vec<usize>, Vec<Vec<f32>>) {
    let n = embeddings.len();
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let normalized: Vec<Vec<f32>> = embeddings.iter().map(|e| normalize(e)).collect();
    let sim = |i: usize, j: usize| cosine(&normalized[i], &normalized[j]);

    // sums[a][b]: total pairwise similarity between the members of clusters a and b,
    // updated on merge so each step is a scan instead of a recomputation
    let mut sums: Vec<Vec<f32>> = (0..n).map(|i| (0..n).map(|j| sim(i, j)).collect()).collect();
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut active = vec![true; n];
    let mut count = n;
    let target = options.num_speakers.map(|k| k.max(1));
    loop {
        if count <= target.unwrap_or(1) {
            break;
        }
        let mut best: Option<(usize, usize, f32)> = None;
        for a in (0..n).filter(|&a| active[a]) {
            for b in ((a + 1)..n).filter(|&b| active[b]) {
                let avg = sums[a][b] / (members[a].len() * members[b].len()) as f32;
                if best.map_or(true, |(_, _, s)| avg > s) {
                    best = Some((a, b, avg));
                }
            }
        }
        let Some((a, b, similarity)) = best else { break };
        if target.is_none() && similarity < options.threshold {
            break;
        }
        let moved = std::mem::take(&mut members[b]);
        members[a].extend(moved);
        active[b] = false;
        count -= 1;
        for c in (0..n).filter(|&c| active[c] && c != a) {
            let shared = sums[b][c];
            sums[a][c] += shared;
            sums[c][a] += shared;
        }
    }
    let mut clusters: Vec<Vec<usize>> = (0..n).filter(|&a| active[a]).map(|a| std::mem::take(&mut members[a])).collect();

    if target.is_none() && options.min_cluster_weight > 0.0 {
        let weight = |c: &Vec<usize>| c.iter().map(|&i| weights.get(i).copied().unwrap_or(1.0)).sum::<f64>();
        let (mut large, small): (Vec<_>, Vec<_>) =
            clusters.into_iter().partition(|c| weight(c) >= options.min_cluster_weight);
        if large.is_empty() {
            clusters = small;
        } else {
            // Compare against the large clusters as they were, not as they grow
            let reference: Vec<Vec<usize>> = large.clone();
            for item in small.into_iter().flatten() {
                let nearest = reference
                    .iter()
                    .enumerate()
                    .map(|(k, c)| (k, c.iter().map(|&j| sim(item, j)).sum::<f32>() / c.len() as f32))
                    .max_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(k, _)| k)
                    .unwrap_or(0);
                large[nearest].push(item);
            }
            clusters = large;
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
    fn a_change_of_local_speaker_starts_a_new_segment() {
        let mut labels = vec![None; 5];
        labels.extend(vec![Some(0); 80]);
        labels.extend(vec![Some(1); 80]); // speaker change without pause
        labels.extend(vec![None; 5]); // short gap, but next label differs → no bridge
        labels.extend(vec![Some(0); 80]);
        labels.extend(vec![None; 5]); // short gap, same label → bridged
        labels.extend(vec![Some(0); 40]);
        let segments = labeled_frames_to_segments(&labels, 16_000, 0.3, 0.25);
        assert_eq!(segments.iter().map(|s| s.label).collect::<Vec<_>>(), vec![0, 1, 0]);
        assert!((segments[0].end - segments[1].start).abs() < 1e-9);
        assert!(segments[2].end - segments[2].start > 2.0);
    }

    fn noisy(base: &[f32], k: usize) -> Vec<f32> {
        base.iter().enumerate().map(|(i, v)| v + 0.05 * (((i + k) % 3) as f32 - 1.0)).collect()
    }

    #[test]
    fn known_speaker_count_is_respected() {
        let a = [1.0, 0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0, 0.0];
        let c = [0.0, 0.0, 1.0, 0.0];
        let embeddings: Vec<Vec<f32>> = (0..9).map(|k| noisy([&a, &b, &c][k % 3], k)).collect();
        let weights = vec![3.0; 9];
        let free = cluster_speakers(&embeddings, &weights, &ClusterOptions { threshold: 0.5, num_speakers: None, min_cluster_weight: 0.0 });
        assert_eq!(free.1.len(), 3);
        let two = cluster_speakers(&embeddings, &weights, &ClusterOptions { threshold: 0.99, num_speakers: Some(2), min_cluster_weight: 0.0 });
        assert_eq!(two.1.len(), 2);
        assert_eq!(two.0.iter().filter(|&&l| l == two.0[0]).count() % 3, 0); // whole speakers merged
    }

    #[test]
    fn tiny_clusters_are_absorbed_by_the_nearest_speaker() {
        let a = [1.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        // A stray segment, closer to b than to a, that would otherwise be its own speaker
        let stray = vec![0.1, 0.6, 0.8];
        let mut embeddings: Vec<Vec<f32>> = (0..6).map(|k| noisy(if k % 2 == 0 { &a } else { &b }, k)).collect();
        embeddings.push(stray);
        let mut weights = vec![4.0; 6];
        weights.push(1.0);
        let options = ClusterOptions { threshold: 0.8, num_speakers: None, min_cluster_weight: 0.0 };
        assert_eq!(cluster_speakers(&embeddings, &weights, &options).1.len(), 3);
        let (labels, centroids) = cluster_speakers(&embeddings, &weights, &ClusterOptions { min_cluster_weight: 5.0, ..options });
        assert_eq!(centroids.len(), 2);
        assert_eq!(labels[6], labels[1]);
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
