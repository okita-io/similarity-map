//! Focused fixture + local manuscript acceptance tests for lexical detection.

use similarity_core::{analyze_lexical, build_scope_manifest, LexicalPassConfig};

fn load_fixture(name: &str) -> String {
    let path = format!(
        "{}/../test-data/lexical/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn run_default(text: &str) -> similarity_core::RepetitionReport {
    let manifest = build_scope_manifest(1, text, 0);
    let (report, _stats) =
        analyze_lexical(text, &manifest, &LexicalPassConfig::default(), "fixture").unwrap();
    report
}

fn cluster_covers_both_halves(report: &similarity_core::RepetitionReport, mid: u32) -> bool {
    report.clusters.iter().any(|c| {
        let before = c
            .spans
            .iter()
            .any(|s| s.doc_char_start < mid && s.doc_char_end <= mid);
        let after = c.spans.iter().any(|s| s.doc_char_start >= mid);
        // Also accept straddling when mid falls inside the separator between copies.
        let left = c.spans.iter().any(|s| s.doc_char_end <= mid + 64);
        let right = c.spans.iter().any(|s| s.doc_char_start + 64 >= mid);
        ((before && after) || (left && right && c.spans.len() >= 2)) && c.instance_count >= 2
    })
}

#[test]
fn adjacent_exact_blocks_stay_two_instances() {
    let text = load_fixture("exact_baldwin_block.txt");
    let first = text
        .split("SCENE BREAK")
        .next()
        .unwrap()
        .lines()
        .filter(|l| !l.contains('—'))
        .collect::<Vec<_>>()
        .join("\n");
    let first = first.trim();
    let adjacent = format!("{first}\n\n{first}");
    let report = run_default(&adjacent);
    let best = report
        .clusters
        .iter()
        .max_by_key(|c| c.total_word_estimate)
        .expect("cluster");
    eprintln!(
        "adjacent spans={:?}",
        best.spans
            .iter()
            .map(|s| (
                s.doc_char_start,
                s.doc_char_end,
                s.text.split_whitespace().count()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(best.instance_count, 2);
    assert!(best.spans[0].doc_char_end <= best.spans[1].doc_char_start);
    assert!(best.canonical.text.split_whitespace().count() >= 300);
}

#[test]
fn exact_baldwin_block_collapses_to_one_family() {
    let text = load_fixture("exact_baldwin_block.txt");
    let break_at = text.find("SCENE BREAK").expect("fixture separator") as u32;
    let report = run_default(&text);
    assert!(
        cluster_covers_both_halves(&report, break_at),
        "expected a multi-instance cluster spanning both Baldwin copies"
    );
    let best = report
        .clusters
        .iter()
        .filter(|c| c.instance_count >= 2)
        .max_by_key(|c| c.total_word_estimate)
        .expect("covering cluster");
    // Maximal collapse: one issue family, not dozens of paragraph fragments.
    assert_eq!(best.instance_count, 2);
    assert!(
        best.canonical.text.split_whitespace().count() >= 300,
        "canonical should be the maximal ~337-word block, got {} words",
        best.canonical.text.split_whitespace().count()
    );
}

#[test]
fn exact_13para_dawn_subblock_is_maximal() {
    let text = load_fixture("exact_13para_dawn_subblock.txt");
    let mid = (text.len() as u32) / 2;
    let report = run_default(&text);
    let best = report
        .clusters
        .iter()
        .filter(|c| {
            c.spans.iter().any(|s| s.doc_char_end <= mid)
                && c.spans.iter().any(|s| s.doc_char_start >= mid)
        })
        .max_by_key(|c| c.total_word_estimate)
        .expect("covering cluster");
    assert_eq!(best.instance_count, 2);
    assert!(best.canonical.text.split_whitespace().count() >= 300);
}

#[test]
fn near_dawn_17para_family_recovered() {
    let text = load_fixture("near_dawn_block.txt");
    let mid = (text.len() as u32) / 2;
    let report = run_default(&text);
    assert!(cluster_covers_both_halves(&report, mid));
    let best = report
        .clusters
        .iter()
        .filter(|c| {
            c.spans.iter().any(|s| s.doc_char_end <= mid)
                && c.spans.iter().any(|s| s.doc_char_start >= mid)
        })
        .max_by_key(|c| c.total_word_estimate)
        .expect("dawn family");
    assert!(
        best.canonical.text.split_whitespace().count() >= 400,
        "expected near-maximal dawn block, got {} words",
        best.canonical.text.split_whitespace().count()
    );
}

#[test]
fn near_found_it_nine_paragraph_scene() {
    let text = load_fixture("near_found_it_scene.txt");
    let mid = (text.len() as u32) / 2;
    let report = run_default(&text);
    assert!(
        cluster_covers_both_halves(&report, mid),
        "9-paragraph near scene should be detected"
    );
}

#[test]
fn near_what_place_five_paragraph_scene() {
    let text = load_fixture("near_what_place_scene.txt");
    let mid = (text.len() as u32) / 2;
    let report = run_default(&text);
    assert!(
        cluster_covers_both_halves(&report, mid),
        "5-paragraph near scene should be detected"
    );
}

#[test]
fn short_sentence_and_paragraph_loops() {
    let text = load_fixture("short_sentence_paragraph.txt");
    let report = run_default(&text);
    assert!(
        report.clusters.iter().any(|c| c.instance_count >= 2),
        "short repeated sentence/paragraph should be found"
    );
}

#[test]
fn negative_formulaic_rejected() {
    let text = load_fixture("negative_formulaic.txt");
    let report = run_default(&text);
    assert!(
        report.clusters.is_empty(),
        "unrelated formulaic prose must not cluster: {:?}",
        report
            .clusters
            .iter()
            .map(|c| c.representative_text.chars().take(80).collect::<String>())
            .collect::<Vec<_>>()
    );
}

/// Local acceptance against the full untracked manuscript.
///
/// Run with: `cargo test -p similarity-core manuscript_anchor_acceptance -- --ignored --nocapture`
#[test]
#[ignore = "requires local untracked manuscript.txt"]
fn manuscript_anchor_acceptance() {
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use std::time::Instant;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let manuscript_path = root.join("manuscript.txt");
    if !manuscript_path.is_file() {
        eprintln!(
            "skip: manuscript.txt not present at {}",
            manuscript_path.display()
        );
        return;
    }
    let text = std::fs::read_to_string(&manuscript_path).expect("read manuscript");
    let digest = Sha256::digest(text.as_bytes());
    let hex = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let anchors_raw =
        std::fs::read_to_string(root.join("test-data/lexical/anchors.json")).expect("anchors.json");
    let anchors: Value = serde_json::from_str(&anchors_raw).expect("anchors json");
    let expected = anchors["manuscript"]["sha256"].as_str().unwrap();
    assert_eq!(hex, expected, "manuscript hash drift — update anchors.json");

    let manifest = build_scope_manifest(1, &text, 0);
    let started = Instant::now();
    let (report, stats) = analyze_lexical(
        &text,
        &manifest,
        &LexicalPassConfig::default(),
        "manuscript",
    )
    .unwrap();
    let elapsed = started.elapsed();
    eprintln!(
        "manuscript lexical: clusters={} candidates={} pairs={} raw_matches={} elapsed_ms={} (stats {:?})",
        report.clusters.len(),
        stats.candidate_count,
        stats.pair_comparisons,
        stats.raw_match_count,
        elapsed.as_millis(),
        stats
    );

    let baldwin_starts = [293536u32, 295482u32];
    let dawn_starts = [270340u32, 284476u32];
    let found_starts = [252771u32, 254868u32];
    let what_place_starts = [122816u32, 139801u32];

    assert!(
        spans_hit_anchor_windows(&report, &baldwin_starts, 1800, 300),
        "Baldwin exact block anchors not recovered"
    );
    assert!(
        spans_hit_anchor_windows(&report, &dawn_starts, 4000, 400),
        "Dawn near/exact family anchors not recovered"
    );
    assert!(
        spans_hit_anchor_windows(&report, &found_starts, 1500, 150),
        "Found-it near scene anchors not recovered"
    );
    assert!(
        spans_hit_anchor_windows(&report, &what_place_starts, 1500, 150),
        "What-place near scene anchors not recovered"
    );
}

fn spans_hit_anchor_windows(
    report: &similarity_core::RepetitionReport,
    starts: &[u32],
    window: u32,
    min_words: usize,
) -> bool {
    report.clusters.iter().any(|c| {
        if c.instance_count < 2 {
            return false;
        }
        let words_ok = c.canonical.text.split_whitespace().count() >= min_words
            || c.total_word_estimate >= min_words as u32
            || min_words <= 200;
        if !words_ok {
            return false;
        }
        let mut used = std::collections::HashSet::new();
        starts.iter().all(|&start| {
            let end = start.saturating_add(window);
            c.spans.iter().enumerate().any(|(idx, s)| {
                if used.contains(&idx) {
                    return false;
                }
                let hit = s.doc_char_start < end && s.doc_char_end > start;
                if hit {
                    used.insert(idx);
                }
                hit
            })
        })
    })
}
