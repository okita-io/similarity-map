//! Deterministic lexical shingle repetition detector.
//!
//! Primary Romance Factory pass: finds exact and near-exact repeated sentences,
//! paragraphs, and multi-paragraph blocks without requiring ONNX embeddings.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::report::{
    derive_cluster_enrichments, resolve_span_location, AnalysisStats, ClusterSummary, EditSpan,
    RepetitionReport, ScopeManifest,
};
use crate::types::AppError;

/// How candidate units are produced for lexical matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LexicalCandidateKind {
    #[default]
    Sentence,
    Paragraph,
    Phrase,
    Block,
}

/// Tunables for the lexical primary pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexicalPassConfig {
    #[serde(default = "default_min_repetitions")]
    pub min_repetitions: u32,
    #[serde(default = "default_sentence_min_tokens")]
    pub sentence_min_tokens: usize,
    #[serde(default = "default_sentence_shingle_size")]
    pub sentence_shingle_size: usize,
    #[serde(default = "default_paragraph_min_tokens")]
    pub paragraph_min_tokens: usize,
    #[serde(default = "default_paragraph_shingle_size")]
    pub paragraph_shingle_size: usize,
    #[serde(default = "default_phrase_tokens")]
    pub phrase_tokens: usize,
    #[serde(default = "default_phrase_stride")]
    pub phrase_stride: usize,
    #[serde(default = "default_sentence_jaccard")]
    pub sentence_jaccard_threshold: f32,
    #[serde(default = "default_paragraph_jaccard")]
    pub paragraph_jaccard_threshold: f32,
    #[serde(default = "default_block_jaccard")]
    pub block_jaccard_threshold: f32,
    #[serde(default = "default_max_df_fraction")]
    pub max_df_fraction: f32,
    #[serde(default = "default_max_fanout")]
    pub max_candidate_fanout: usize,
    #[serde(default = "default_block_min_paragraphs")]
    pub block_min_paragraphs: usize,
    #[serde(default = "default_block_min_words")]
    pub block_min_words: usize,
}

impl Default for LexicalPassConfig {
    fn default() -> Self {
        Self {
            min_repetitions: default_min_repetitions(),
            sentence_min_tokens: default_sentence_min_tokens(),
            sentence_shingle_size: default_sentence_shingle_size(),
            paragraph_min_tokens: default_paragraph_min_tokens(),
            paragraph_shingle_size: default_paragraph_shingle_size(),
            phrase_tokens: default_phrase_tokens(),
            phrase_stride: default_phrase_stride(),
            sentence_jaccard_threshold: default_sentence_jaccard(),
            paragraph_jaccard_threshold: default_paragraph_jaccard(),
            block_jaccard_threshold: default_block_jaccard(),
            max_df_fraction: default_max_df_fraction(),
            max_candidate_fanout: default_max_fanout(),
            block_min_paragraphs: default_block_min_paragraphs(),
            block_min_words: default_block_min_words(),
        }
    }
}

fn default_min_repetitions() -> u32 {
    2
}
fn default_sentence_min_tokens() -> usize {
    12
}
fn default_sentence_shingle_size() -> usize {
    3
}
fn default_paragraph_min_tokens() -> usize {
    8
}
fn default_paragraph_shingle_size() -> usize {
    5
}
fn default_phrase_tokens() -> usize {
    12
}
fn default_phrase_stride() -> usize {
    6
}
fn default_sentence_jaccard() -> f32 {
    0.82
}
fn default_paragraph_jaccard() -> f32 {
    0.72
}
fn default_block_jaccard() -> f32 {
    0.70
}
fn default_max_df_fraction() -> f32 {
    0.35
}
fn default_max_fanout() -> usize {
    64
}
fn default_block_min_paragraphs() -> usize {
    2
}
fn default_block_min_words() -> usize {
    80
}

/// Runtime statistics useful for calibration / acceptance logs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LexicalAnalysisStats {
    pub candidate_count: u32,
    pub pair_comparisons: u32,
    pub raw_match_count: u32,
    pub cluster_count: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone)]
struct NormToken {
    text: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct Candidate {
    id: usize,
    kind: LexicalCandidateKind,
    start: usize,
    end: usize,
    tokens: Vec<String>,
    shingles: BTreeSet<String>,
    paragraph_index: Option<usize>,
}

#[derive(Debug, Clone)]
struct ScoredPair {
    left: usize,
    right: usize,
    score: f32,
}

#[derive(Debug, Clone)]
struct BlockMatch {
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
    score: f32,
}

#[derive(Debug, Clone)]
struct InstanceSpan {
    start: usize,
    end: usize,
    score: f32,
}

/// Analyze prose with the lexical shingle detector and emit a [`RepetitionReport`].
pub fn analyze_lexical(
    text: &str,
    manifest: &ScopeManifest,
    config: &LexicalPassConfig,
    job_id: &str,
) -> Result<(RepetitionReport, LexicalAnalysisStats), AppError> {
    let started = std::time::Instant::now();
    let min_reps = config.min_repetitions.max(2) as usize;

    let paragraphs = split_paragraphs(text);
    let mut candidates = Vec::new();

    for &(start, end, _) in &paragraphs {
        let para_sentences = split_sentences(&text[start..end]);
        for (rel_s, rel_e, _) in para_sentences {
            let abs_s = start + rel_s;
            let abs_e = start + rel_e;
            let toks = tokenize_span(text, abs_s, abs_e);
            if toks.len() < config.sentence_min_tokens {
                continue;
            }
            if let Some(c) = make_candidate(
                candidates.len(),
                LexicalCandidateKind::Sentence,
                &toks,
                config.sentence_shingle_size,
                None,
            ) {
                candidates.push(c);
            }
        }
    }

    for (para_idx, &(start, end, _)) in paragraphs.iter().enumerate() {
        let toks = tokenize_span(text, start, end);
        if toks.len() >= config.paragraph_min_tokens {
            if let Some(c) = make_candidate(
                candidates.len(),
                LexicalCandidateKind::Paragraph,
                &toks,
                config.paragraph_shingle_size,
                Some(para_idx),
            ) {
                candidates.push(c);
            }
        }
        push_phrase_windows(
            &mut candidates,
            &toks,
            config.phrase_tokens,
            config.phrase_stride,
            config.sentence_shingle_size,
        );
    }

    let mut stats = LexicalAnalysisStats {
        candidate_count: candidates.len() as u32,
        ..Default::default()
    };

    if candidates.len() < min_reps {
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        return Ok((empty_report(job_id), stats));
    }

    let index = build_inverted_index(&candidates, config.max_df_fraction);
    let mut scored_pairs = Vec::new();
    let mut compared = HashSet::new();

    // Exact token-sequence index bypasses DF pruning (critical for long duplicated blocks).
    let mut exact_index: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
    for c in &candidates {
        if c.kind == LexicalCandidateKind::Phrase {
            continue;
        }
        exact_index.entry(c.tokens.clone()).or_default().push(c.id);
    }
    for ids in exact_index.values() {
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                let left = ids[i].min(ids[j]);
                let right = ids[i].max(ids[j]);
                if !compared.insert((left, right)) {
                    continue;
                }
                let a = &candidates[left];
                let b = &candidates[right];
                if spans_overlap(a.start, a.end, b.start, b.end) {
                    continue;
                }
                stats.pair_comparisons += 1;
                scored_pairs.push(ScoredPair {
                    left,
                    right,
                    score: 1.0,
                });
            }
        }
    }

    for cand in &candidates {
        let mut neighbor_hits: HashMap<usize, u32> = HashMap::new();
        for sh in &cand.shingles {
            let Some(posting) = index.get(sh) else {
                continue;
            };
            for &other in posting {
                if other <= cand.id {
                    continue;
                }
                *neighbor_hits.entry(other).or_insert(0) += 1;
            }
        }
        let mut neighbors: Vec<(usize, u32)> = neighbor_hits.into_iter().collect();
        neighbors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        neighbors.truncate(config.max_candidate_fanout);

        for (other_id, _) in neighbors {
            if !compared.insert((cand.id, other_id)) {
                continue;
            }
            stats.pair_comparisons += 1;
            let other = &candidates[other_id];
            if spans_overlap(cand.start, cand.end, other.start, other.end) {
                continue;
            }
            let threshold = threshold_for(cand.kind, other.kind, config);
            let exact = !cand.tokens.is_empty() && cand.tokens == other.tokens;
            let score = if exact {
                1.0
            } else {
                lexical_similarity(cand, other)
            };
            if exact || score + f32::EPSILON >= threshold {
                scored_pairs.push(ScoredPair {
                    left: cand.id,
                    right: other_id,
                    score,
                });
            }
        }
    }
    stats.raw_match_count = scored_pairs.len() as u32;

    let block_matches = chain_paragraph_blocks(&candidates, &scored_pairs, config);

    // Union-find over non-overlapping instance spans.
    let mut instances: Vec<InstanceSpan> = Vec::new();
    let mut edges: Vec<(usize, usize, f32)> = Vec::new();

    let mut add_instance = |span: InstanceSpan| -> usize {
        // Merge into an existing overlapping span when possible (maximal collapse).
        for (idx, existing) in instances.iter_mut().enumerate() {
            if spans_overlap(existing.start, existing.end, span.start, span.end) {
                existing.start = existing.start.min(span.start);
                existing.end = existing.end.max(span.end);
                existing.score = existing.score.max(span.score);
                return idx;
            }
        }
        instances.push(span);
        instances.len() - 1
    };

    for pair in &scored_pairs {
        let left = &candidates[pair.left];
        let right = &candidates[pair.right];
        // Prefer block-level coverage later; still record unit matches.
        let li = add_instance(InstanceSpan {
            start: left.start,
            end: left.end,
            score: pair.score,
        });
        let ri = add_instance(InstanceSpan {
            start: right.start,
            end: right.end,
            score: pair.score,
        });
        if li != ri {
            edges.push((li, ri, pair.score));
        }
    }

    for block in &block_matches {
        let li = add_instance(InstanceSpan {
            start: block.left_start,
            end: block.left_end,
            score: block.score,
        });
        let ri = add_instance(InstanceSpan {
            start: block.right_start,
            end: block.right_end,
            score: block.score,
        });
        if li != ri {
            edges.push((li, ri, block.score));
        }
    }

    // Re-merge overlapping instances after expansions, remapping edges.
    let (instances, edges) = collapse_overlapping_instances(instances, edges);

    let parent = union_find_cluster(&instances, &edges);
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for idx in 0..instances.len() {
        let root = find_root(&parent, idx);
        groups.entry(root).or_default().push(idx);
    }

    let mut cluster_summaries = Vec::new();
    let mut next_cluster_id = 1i32;

    for mut members in groups.into_values() {
        members.sort_by_key(|&i| instances[i].start);
        // Drop contained spans inside the same cluster (keep maximal).
        let maximal = maximal_spans(&instances, &members);
        if maximal.len() < min_reps {
            continue;
        }

        let mut edit_spans = Vec::new();
        for (inst_idx, &member) in maximal.iter().enumerate() {
            let span = &instances[member];
            let start = span.start as u32;
            let end = span.end as u32;
            let snippet = text.get(span.start..span.end).unwrap_or("").to_string();
            let location = resolve_span_location(text, manifest, start, end);
            edit_spans.push(EditSpan {
                cluster_id: next_cluster_id,
                instance_id: (inst_idx + 1) as u32,
                doc_char_start: start,
                doc_char_end: end,
                text: snippet,
                similarity_to_centroid: span.score.clamp(0.0, 1.0),
                member_window_count: 1,
                location: Some(location),
            });
        }

        let (cross_act, needs_bridge, suggested_op) = derive_cluster_enrichments(&edit_spans);
        let total_word_estimate = edit_spans
            .iter()
            .map(|s| s.text.split_whitespace().count() as u32)
            .sum();
        let canonical = edit_spans[0].clone();
        let duplicates = edit_spans.iter().skip(1).cloned().collect::<Vec<_>>();
        let representative_text = canonical.text.clone();

        cluster_summaries.push(ClusterSummary {
            cluster_id: next_cluster_id,
            representative_text,
            instance_count: edit_spans.len() as u32,
            total_word_estimate,
            canonical,
            duplicates,
            spans: edit_spans,
            cross_act,
            suggested_op,
            needs_bridge,
        });
        next_cluster_id += 1;
    }

    cluster_summaries.sort_by_key(|c| c.canonical.doc_char_start);

    // Prefer larger/more specific clusters: drop clusters whose every span is
    // fully contained in a larger cluster's span set (fragment suppression).
    cluster_summaries = suppress_contained_clusters(cluster_summaries);

    let total_duplicate_instances = cluster_summaries
        .iter()
        .map(|c| c.duplicates.len() as u32)
        .sum();
    let total_duplicate_words_estimate = cluster_summaries
        .iter()
        .flat_map(|c| c.duplicates.iter())
        .map(|s| s.text.split_whitespace().count() as u32)
        .sum();

    let report = RepetitionReport {
        job_id: job_id.to_string(),
        stats: AnalysisStats {
            cluster_count: cluster_summaries.len() as u32,
            total_duplicate_instances,
            total_duplicate_words_estimate,
        },
        clusters: cluster_summaries,
        schema_version: Some(crate::report::SCHEMA_VERSION.to_string()),
        scope: None,
        analysis_params: None,
    };

    stats.cluster_count = report.clusters.len() as u32;
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok((report, stats))
}

fn empty_report(job_id: &str) -> RepetitionReport {
    RepetitionReport {
        job_id: job_id.to_string(),
        clusters: vec![],
        stats: AnalysisStats {
            cluster_count: 0,
            total_duplicate_instances: 0,
            total_duplicate_words_estimate: 0,
        },
        schema_version: Some(crate::report::SCHEMA_VERSION.to_string()),
        scope: None,
        analysis_params: None,
    }
}

fn threshold_for(
    a: LexicalCandidateKind,
    b: LexicalCandidateKind,
    config: &LexicalPassConfig,
) -> f32 {
    use LexicalCandidateKind::*;
    match dominant_kind(a, b) {
        Sentence | Phrase => config.sentence_jaccard_threshold,
        Paragraph => config.paragraph_jaccard_threshold,
        Block => config.block_jaccard_threshold,
    }
}

fn dominant_kind(a: LexicalCandidateKind, b: LexicalCandidateKind) -> LexicalCandidateKind {
    use LexicalCandidateKind::*;
    let rank = |k| match k {
        Block => 3,
        Paragraph => 2,
        Sentence => 1,
        Phrase => 0,
    };
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

fn spans_overlap(a0: usize, a1: usize, b0: usize, b1: usize) -> bool {
    a0 < b1 && b0 < a1
}

fn make_candidate(
    id: usize,
    kind: LexicalCandidateKind,
    tokens: &[NormToken],
    shingle_size: usize,
    paragraph_index: Option<usize>,
) -> Option<Candidate> {
    if tokens.is_empty() {
        return None;
    }
    let token_texts: Vec<String> = tokens.iter().map(|t| t.text.clone()).collect();
    Some(Candidate {
        id,
        kind,
        start: tokens.first()?.start,
        end: tokens.last()?.end,
        shingles: make_shingles(&token_texts, shingle_size),
        tokens: token_texts,
        paragraph_index,
    })
}

fn push_phrase_windows(
    out: &mut Vec<Candidate>,
    tokens: &[NormToken],
    phrase_tokens: usize,
    phrase_stride: usize,
    shingle_size: usize,
) {
    if tokens.len() < phrase_tokens {
        return;
    }
    let stride = phrase_stride.max(1);
    let mut i = 0;
    while i + phrase_tokens <= tokens.len() {
        let slice = &tokens[i..i + phrase_tokens];
        if let Some(c) = make_candidate(
            out.len(),
            LexicalCandidateKind::Phrase,
            slice,
            shingle_size,
            None,
        ) {
            out.push(c);
        }
        i += stride;
    }
}

fn make_shingles(tokens: &[String], size: usize) -> BTreeSet<String> {
    let size = size.max(1);
    let mut out = BTreeSet::new();
    if tokens.len() < size {
        out.insert(tokens.join(" "));
        return out;
    }
    for i in 0..=tokens.len() - size {
        out.insert(tokens[i..i + size].join(" "));
    }
    out
}

fn build_inverted_index(
    candidates: &[Candidate],
    max_df_fraction: f32,
) -> HashMap<String, Vec<usize>> {
    let mut df: HashMap<String, usize> = HashMap::new();
    for c in candidates {
        for sh in &c.shingles {
            *df.entry(sh.clone()).or_insert(0) += 1;
        }
    }
    let max_df = ((candidates.len() as f32) * max_df_fraction)
        .ceil()
        .max(2.0) as usize;
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for c in candidates {
        for sh in &c.shingles {
            if df.get(sh).copied().unwrap_or(0) > max_df {
                continue;
            }
            index.entry(sh.clone()).or_default().push(c.id);
        }
    }
    index
}

fn lexical_similarity(a: &Candidate, b: &Candidate) -> f32 {
    let jaccard = shingle_jaccard(&a.shingles, &b.shingles);
    let order = token_order_score(&a.tokens, &b.tokens);
    let freq = token_frequency_score(&a.tokens, &b.tokens);
    0.70 * jaccard + 0.20 * order + 0.10 * freq
}

fn shingle_jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn token_frequency_score(a: &[String], b: &[String]) -> f32 {
    let mut fa: HashMap<&str, usize> = HashMap::new();
    let mut fb: HashMap<&str, usize> = HashMap::new();
    for t in a {
        *fa.entry(t.as_str()).or_insert(0) += 1;
    }
    for t in b {
        *fb.entry(t.as_str()).or_insert(0) += 1;
    }
    let mut keys: HashSet<&str> = fa.keys().copied().collect();
    keys.extend(fb.keys().copied());
    if keys.is_empty() {
        return 0.0;
    }
    let mut num = 0usize;
    let mut den = 0usize;
    for k in keys {
        let x = fa.get(k).copied().unwrap_or(0);
        let y = fb.get(k).copied().unwrap_or(0);
        num += x.min(y);
        den += x.max(y);
    }
    if den == 0 {
        0.0
    } else {
        num as f32 / den as f32
    }
}

fn token_order_score(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let n = a.len();
    let m = b.len();
    let mut prev = vec![0usize; m + 1];
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        for j in 1..=m {
            if a[i - 1] == b[j - 1] {
                cur[j] = prev[j - 1] + 1;
            } else {
                cur[j] = prev[j].max(cur[j - 1]);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.fill(0);
    }
    prev[m] as f32 / n.max(m) as f32
}

fn chain_paragraph_blocks(
    candidates: &[Candidate],
    scored_pairs: &[ScoredPair],
    config: &LexicalPassConfig,
) -> Vec<BlockMatch> {
    let mut by_para: BTreeMap<usize, usize> = BTreeMap::new();
    let mut by_id: HashMap<usize, &Candidate> = HashMap::new();
    for c in candidates {
        if c.kind == LexicalCandidateKind::Paragraph {
            if let Some(pi) = c.paragraph_index {
                by_para.insert(pi, c.id);
                by_id.insert(c.id, c);
            }
        }
    }

    let para_id_set: HashSet<usize> = by_id.keys().copied().collect();
    let mut adj: HashMap<(usize, usize), f32> = HashMap::new();
    for p in scored_pairs {
        if para_id_set.contains(&p.left) && para_id_set.contains(&p.right) {
            adj.insert((p.left.min(p.right), p.left.max(p.right)), p.score);
        }
    }

    let paras: Vec<usize> = by_para.keys().copied().collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for (ia, &pa) in paras.iter().enumerate() {
        let id_a = by_para[&pa];
        for &pb in paras.iter().skip(ia + 1) {
            let id_b = by_para[&pb];
            let key = (id_a.min(id_b), id_a.max(id_b));
            let Some(&first_score) = adj.get(&key) else {
                continue;
            };

            let mut run = 1usize;
            let mut score_sum = first_score;
            loop {
                let next_a = pa + run;
                let next_b = pb + run;
                // Left run must remain strictly before the right run's start.
                if next_a >= pb {
                    break;
                }
                let (Some(&ida), Some(&idb)) = (by_para.get(&next_a), by_para.get(&next_b)) else {
                    break;
                };
                let k = (ida.min(idb), ida.max(idb));
                let Some(&sc) = adj.get(&k) else {
                    break;
                };
                score_sum += sc;
                run += 1;
            }

            let words: usize = (0..run)
                .map(|k| by_id[&by_para[&(pa + k)]].tokens.len())
                .sum();
            if run < config.block_min_paragraphs && words < config.block_min_words {
                continue;
            }
            if !seen.insert((pa, pb, run)) {
                continue;
            }

            let left_start = by_id[&by_para[&pa]].start;
            let left_end = by_id[&by_para[&(pa + run - 1)]].end;
            let right_start = by_id[&by_para[&pb]].start;
            let right_end = by_id[&by_para[&(pb + run - 1)]].end;
            out.push(BlockMatch {
                left_start,
                left_end,
                right_start,
                right_end,
                score: score_sum / run as f32,
            });
        }
    }
    out
}

fn collapse_overlapping_instances(
    instances: Vec<InstanceSpan>,
    edges: Vec<(usize, usize, f32)>,
) -> (Vec<InstanceSpan>, Vec<(usize, usize, f32)>) {
    if instances.is_empty() {
        return (instances, edges);
    }
    let mut order: Vec<usize> = (0..instances.len()).collect();
    order.sort_by_key(|&i| (instances[i].start, usize::MAX - instances[i].end));

    let mut mapping = vec![0usize; instances.len()];
    let mut merged: Vec<InstanceSpan> = Vec::new();
    for idx in order {
        let span = &instances[idx];
        let mut absorbed = false;
        for (m_idx, existing) in merged.iter_mut().enumerate() {
            if spans_overlap(existing.start, existing.end, span.start, span.end) {
                existing.start = existing.start.min(span.start);
                existing.end = existing.end.max(span.end);
                existing.score = existing.score.max(span.score);
                mapping[idx] = m_idx;
                absorbed = true;
                break;
            }
        }
        if !absorbed {
            mapping[idx] = merged.len();
            merged.push(span.clone());
        }
    }

    let mut new_edges = Vec::new();
    let mut seen = HashSet::new();
    for (a, b, score) in edges {
        let ma = mapping[a];
        let mb = mapping[b];
        if ma == mb {
            continue;
        }
        let key = (ma.min(mb), ma.max(mb));
        if seen.insert(key) {
            new_edges.push((key.0, key.1, score));
        }
    }
    (merged, new_edges)
}

fn union_find_cluster(instances: &[InstanceSpan], edges: &[(usize, usize, f32)]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..instances.len()).collect();
    for &(a, b, _) in edges {
        let ra = find_root(&parent, a);
        let rb = find_root(&parent, b);
        if ra != rb {
            parent[rb] = ra;
        }
    }
    parent
}

fn find_root(parent: &[usize], mut i: usize) -> usize {
    while parent[i] != i {
        i = parent[i];
    }
    i
}

fn maximal_spans(instances: &[InstanceSpan], members: &[usize]) -> Vec<usize> {
    let mut kept = Vec::new();
    for &m in members {
        let span = &instances[m];
        let contained = members.iter().any(|&o| {
            o != m
                && instances[o].start <= span.start
                && instances[o].end >= span.end
                && (instances[o].end - instances[o].start) > (span.end - span.start)
        });
        if !contained {
            kept.push(m);
        }
    }
    kept
}

fn suppress_contained_clusters(clusters: Vec<ClusterSummary>) -> Vec<ClusterSummary> {
    let mut keep = vec![true; clusters.len()];
    for i in 0..clusters.len() {
        for j in 0..clusters.len() {
            if i == j || !keep[i] {
                continue;
            }
            // Drop i if every span in i is contained in some span of j and j is larger.
            let i_words = clusters[i].total_word_estimate;
            let j_words = clusters[j].total_word_estimate;
            if j_words < i_words {
                continue;
            }
            let all_contained = clusters[i].spans.iter().all(|si| {
                clusters[j].spans.iter().any(|sj| {
                    sj.doc_char_start <= si.doc_char_start && sj.doc_char_end >= si.doc_char_end
                })
            });
            if all_contained && j_words > i_words {
                keep[i] = false;
            }
        }
    }

    let mut out: Vec<ClusterSummary> = clusters
        .into_iter()
        .enumerate()
        .filter_map(|(idx, mut c)| {
            if keep[idx] {
                // Reassign deterministic cluster ids after suppression.
                Some({
                    c.cluster_id = 0; // temporary
                    c
                })
            } else {
                None
            }
        })
        .collect();
    out.sort_by_key(|c| c.canonical.doc_char_start);
    for (idx, cluster) in out.iter_mut().enumerate() {
        let id = (idx + 1) as i32;
        cluster.cluster_id = id;
        for span in &mut cluster.spans {
            span.cluster_id = id;
        }
        cluster.canonical.cluster_id = id;
        for dup in &mut cluster.duplicates {
            dup.cluster_id = id;
        }
    }
    out
}

fn tokenize_span(text: &str, abs_start: usize, abs_end: usize) -> Vec<NormToken> {
    let slice = match text.get(abs_start..abs_end) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut tok_start = None;
    let mut byte = abs_start;

    for ch in slice.chars() {
        let ch_len = ch.len_utf8();
        let is_token_char = ch.is_alphanumeric() || ch == '\'';
        if is_token_char {
            if tok_start.is_none() {
                tok_start = Some(byte);
            }
            for c in ch.to_lowercase() {
                cur.push(c);
            }
        } else if let Some(start) = tok_start.take() {
            if !cur.is_empty() {
                tokens.push(NormToken {
                    text: std::mem::take(&mut cur),
                    start,
                    end: byte,
                });
            }
        }
        byte += ch_len;
    }
    if let Some(start) = tok_start {
        if !cur.is_empty() {
            tokens.push(NormToken {
                text: cur,
                start,
                end: byte,
            });
        }
    }
    tokens
}

/// Split text into paragraph spans `(start, end, text)`.
pub fn split_paragraphs(text: &str) -> Vec<(usize, usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b'\n' {
                j += 1;
            }
            if j - i >= 2 {
                let end = i;
                let para = text.get(start..end).unwrap_or("");
                if para.chars().any(|c| !c.is_whitespace()) {
                    let trim_start = start
                        + para
                            .char_indices()
                            .find(|(_, c)| !c.is_whitespace())
                            .map(|(idx, _)| idx)
                            .unwrap_or(0);
                    let trim_end = trim_start + para.trim().len();
                    // Prefer byte-accurate trim:
                    let trimmed = text[start..end].trim();
                    if let Some(rel) = text[start..end].find(trimmed) {
                        let ts = start + rel;
                        let te = ts + trimmed.len();
                        out.push((ts, te, trimmed));
                    }
                    let _ = (trim_start, trim_end);
                }
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if start < text.len() {
        let trimmed = text[start..].trim();
        if !trimmed.is_empty() {
            if let Some(rel) = text[start..].find(trimmed) {
                let ts = start + rel;
                let te = ts + trimmed.len();
                out.push((ts, te, trimmed));
            }
        }
    }
    out
}

/// Split text into sentence spans using simple punctuation boundaries.
pub fn split_sentences(text: &str) -> Vec<(usize, usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (idx, &(pos, ch)) in chars.iter().enumerate() {
        if start.is_none() && !ch.is_whitespace() {
            start = Some(pos);
        }
        let is_end = matches!(ch, '.' | '!' | '?');
        let next_is_boundary = chars
            .get(idx + 1)
            .map(|(_, n)| n.is_whitespace() || *n == '"' || *n == '\'')
            .unwrap_or(true);
        if is_end && next_is_boundary {
            if let Some(s) = start.take() {
                let end = pos + ch.len_utf8();
                if let Some(sentence) = text.get(s..end) {
                    let trimmed = sentence.trim();
                    if !trimmed.is_empty() {
                        if let Some(rel) = sentence.find(trimmed) {
                            let ts = s + rel;
                            out.push((ts, ts + trimmed.len(), trimmed));
                        }
                    }
                }
            }
        }
    }
    if let Some(s) = start {
        if let Some(sentence) = text.get(s..) {
            let trimmed = sentence.trim();
            if !trimmed.is_empty() {
                if let Some(rel) = sentence.find(trimmed) {
                    let ts = s + rel;
                    out.push((ts, ts + trimmed.len(), trimmed));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::build_scope_manifest;

    #[test]
    fn tokenize_preserves_offsets() {
        let text = "Hello, WORLD!";
        let toks = tokenize_span(text, 0, text.len());
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].text, "hello");
        assert_eq!(&text[toks[0].start..toks[0].end], "Hello");
        assert_eq!(toks[1].text, "world");
        assert_eq!(&text[toks[1].start..toks[1].end], "WORLD");
    }

    #[test]
    fn exact_sentence_pair_detected() {
        let sentence = "I've found it, she declared, her voice echoing through the vast chamber like a prophecy fulfilled and ancient drums.";
        let text = format!(
            "{sentence}\n\nBridge prose keeps the copies apart in the fixture.\n\n{sentence}"
        );
        let manifest = build_scope_manifest(1, &text, 0);
        let (report, _) =
            analyze_lexical(&text, &manifest, &LexicalPassConfig::default(), "t").unwrap();
        assert!(
            report.clusters.iter().any(|c| c.instance_count >= 2),
            "expected a repeated sentence cluster"
        );
    }

    #[test]
    fn separated_instances_count_not_windows() {
        let para = "Baldwin stood before Rhyannon, his hands clasped tightly around hers as if he could anchor her to this moment forever, away from prophecy and peril and ruin.";
        let text = format!("{para}\n\nMiddle material.\n\n{para}");
        let manifest = build_scope_manifest(1, &text, 0);
        let (report, _) =
            analyze_lexical(&text, &manifest, &LexicalPassConfig::default(), "t").unwrap();
        let cluster = report
            .clusters
            .iter()
            .max_by_key(|c| c.total_word_estimate)
            .expect("cluster");
        assert_eq!(cluster.instance_count, 2);
    }

    #[test]
    fn negative_formulaic_not_clustered() {
        let text = include_str!("../../test-data/lexical/negative_formulaic.txt");
        let manifest = build_scope_manifest(1, text, 0);
        let (report, _) =
            analyze_lexical(text, &manifest, &LexicalPassConfig::default(), "t").unwrap();
        assert!(
            report.clusters.is_empty(),
            "formulaic unrelated prose should not cluster, got {:?}",
            report
                .clusters
                .iter()
                .map(|c| &c.representative_text)
                .collect::<Vec<_>>()
        );
    }
}
