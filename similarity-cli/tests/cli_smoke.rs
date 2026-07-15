use assert_cmd::Command;
use predicates::prelude::*;
use similarity_core::build_scope_manifest;
use similarity_core::validate_analysis_output;

#[test]
fn analyze_help_lists_story_path() {
    Command::cargo_bin("similarity-cli")
        .unwrap()
        .arg("analyze")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--story-path"));
}

#[test]
fn analyze_stdin_json_with_test_embedder() {
    let text = [
        "alpha beta gamma delta epsilon alpha beta gamma delta epsilon",
        "alpha beta gamma delta epsilon alpha beta gamma delta epsilon",
        "alpha beta gamma delta epsilon alpha beta gamma delta epsilon",
    ]
    .join("\n\n");
    let manifest = build_scope_manifest(1, &text, 0);
    let request = serde_json::json!({
        "text": text,
        "scope_manifest": manifest,
        "params": {
            "window_size": 5,
            "stride": 5,
            "min_repetitions": 2,
            "min_samples": 2,
            "enable_hdbscan": false,
            "link_subphrases": false
        }
    });

    let output = Command::cargo_bin("similarity-cli")
        .unwrap()
        .arg("analyze")
        .arg("--test-embedder")
        .write_stdin(request.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: similarity_core::AnalysisOutput =
        serde_json::from_slice(&output).expect("stdout is AnalysisOutput JSON");
    validate_analysis_output(&parsed).expect("valid v1 contract");
    assert_eq!(parsed.schema_version, "1");
}

#[test]
fn analyze_lexical_only_pass_config_without_onnx() {
    let text = [
        "I've found it, she declared, her voice echoing through the vast chamber like a prophecy fulfilled and ancient drums.",
        "Bridge prose keeps the copies apart in this smoke fixture.",
        "I've found it, she declared, her voice echoing through the vast chamber like a prophecy fulfilled and ancient drums.",
    ]
    .join("\n\n");
    let manifest = build_scope_manifest(1, &text, 0);
    let request = serde_json::json!({
        "text": text,
        "scope_manifest": manifest,
        "params": {
            "window_size": 5,
            "stride": 5,
            "min_repetitions": 2,
            "min_samples": 2,
            "enable_hdbscan": false
        }
    });
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/pass_config_lexical_smoke.yaml");

    let output = Command::cargo_bin("similarity-cli")
        .unwrap()
        .arg("analyze")
        .arg("--pass-config")
        .arg(&fixture)
        .write_stdin(request.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: similarity_core::AnalysisOutput =
        serde_json::from_slice(&output).expect("stdout is AnalysisOutput JSON");
    validate_analysis_output(&parsed).expect("valid v1 contract");
    assert_eq!(parsed.passes[0].pass_id, "chapter_lexical");
    assert!(parsed.merged_repetition_report.stats.cluster_count >= 1);
}
