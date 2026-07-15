# Lexical Shingle Primary Pass Roadmap

Paired implementation checklist for the lexical-first Romance Factory analysis pass.
Status mirrors the active milestone task list and is updated at every milestone boundary.

## Status convention

- `[ ]` not started
- `[-]` in progress
- `[x]` completed

## Locked decisions

- Target `similarity-core` and Romance Factory-facing CLI, PyO3, Tauri RF chapter command, presets, and YAML export first.
- Defer the legacy LanceDB/file-grid path in `src-tauri/src/pipeline.rs` to a clearly marked follow-up milestone.
- Keep `manuscript.txt` local and untracked. Commit only small focused excerpts plus an anchor manifest containing its SHA-256 and expected ranges (`test-data/lexical/`).
- Add a backward-compatible `PassMethod` to input configuration (`embedding` remains the serde default), but keep `AnalysisOutput` schema v1 unchanged. Lexical provenance uses stable `pass_id` / `pass_label` conventions.
- Prepend one chapter-scoped lexical primary pass and set the RF bundle default to `min_repetitions: 2`; existing embedding passes remain secondary.

## Milestone 0 — Roadmap and corpus baseline

- [x] Create the roadmap document first and initialize the paired milestone task list.
- [x] Record the local manuscript fingerprint and ground-truth anchors without committing the full story.
- [x] Add focused fixtures for the exact duplicate, near duplicate, shorter sentence/paragraph cases, and one known negative pair.
- [x] Document acceptance expectations: two occurrences count, contained hits collapse into one maximal issue, and unrelated stylistic prose is not promoted.

## Milestone 1 — Core lexical detector

- [x] Add `similarity-core/src/lexical.rs` with offset-preserving Unicode-aware normalization.
- [x] Generate sentence, paragraph, and short sliding-phrase candidates; use token shingles with an inverted index, document-frequency pruning, and bounded candidate fanout.
- [x] Verify candidates primarily with shingle Jaccard, supplemented by token-order and token-frequency lexical scores for minor LLM variations.
- [x] Chain aligned adjacent paragraph matches and merge overlapping hits into maximal multi-paragraph blocks.
- [x] Count geographically separated instances rather than overlapping stride windows; require `min_repetitions: 2`.
- [x] Emit deterministic cluster IDs, real per-instance lexical scores, and existing `RepetitionReport` / `SpanLocation` structures.
- [x] Fix the current report score stub in `report.rs`, which currently turns every overlapping clustered span into similarity `1.0`.

## Milestone 2 — First-class multi-pass orchestration

- [x] Add backward-compatible `PassMethod` and lexical parameters to `multi_pass.rs`.
- [x] Branch act/chapter execution between lexical and embedding stages without requiring ONNX for lexical-only bundles.
- [x] Prepend the chapter-scoped lexical primary pass to `default_rf_multi_pass_config()` and set the RF default occurrence count to two.
- [x] Keep embedding passes at 50/10, 100/25, 200/50, and 400/100 as secondary recall passes.
- [x] Reuse the existing 50%-overlap pass merge and validate the result against `AnalysisOutput` v1 without a schema bump.

## Milestone 3 — Romance Factory adapters and configuration

- [x] Extend `similarity-cli/src/pass_config.rs` so old YAML defaults to embedding and new YAML accepts `method: lexical`.
- [x] Make lexical-only CLI runs work without an ONNX model and add a lexical smoke configuration.
- [x] Pass lexical configuration through `similarity-core-py/src/lib.rs`.
- [x] Update `src-tauri/src/commands.rs` so RF chapter analysis applies the UI occurrence settings to the actual multi-pass config.
- [x] Prepend the lexical pass in `src/rf-chapter-presets.js`, set the UI/RF default to two, and export `method: lexical` from `src/settings-yaml-export.js`.
- [x] Preserve existing embedding-only configurations and document the external Romance Factory parser follow-up if it does not yet retain `method`.

## Milestone 4 — Manuscript-driven acceptance

- [x] Add deterministic unit tests for normalization, exact pairs, minor variations, separated-instance counting, and false-positive pruning.
- [x] Assert the exact ~336-word block collapses to one issue rather than paragraph/window fragments.
- [x] Assert the ~489-word, 17-paragraph mixed exact/near block is recovered as one maximal family.
- [x] Assert the 9-paragraph and 5-paragraph near scenes are found at calibrated thresholds.
- [x] Assert representative repeated sentences and intra-paragraph loops are found.
- [x] Assert the known formulaic-but-unrelated negative pair is rejected.
- [x] Add an ignored/local acceptance test that verifies the full manuscript hash, scans it when present, checks anchor overlap rather than global cluster counts, and records runtime/candidate statistics.

## Milestone 5 — Verification and documentation

- [x] Run formatting, workspace check, core/CLI/PyO3/Tauri tests, and the local manuscript acceptance scan.
- [x] Repair the existing CLI unit-test import defect so `cargo test --workspace` is a meaningful aggregate gate.
- [x] Update `README.md`, `CURRENT-STATE.md`, `ARCHITECTURE.md`, and the integration contract with lexical semantics, thresholds, and limitations.
- [x] Mark completed roadmap checkboxes and leave the desktop LanceDB/grid integration as the next unstarted milestone.

## Deferred milestone — Desktop persistent/grid integration

- [x] Route file-based LanceDB analysis through the lexical primary pass.
- [x] Decide persistence schema and visualization behavior for lexical clusters.
- [x] Expose lexical provenance and scores in the desktop grid/detail UI.

**Persistence decision:** keep LanceDB windows embedding-only; write
`sessions/<job_id>.analysis_output.json` sidecars for lexical/contract output.
`get_visualization_payload` loads the sidecar, attaches `analysis_output`, and merges
lexical highlights. `get_page_detail` is implemented from page sub-grids + windows.

## RF pipeline readiness (this repo)

- [x] PyO3 helpers: `build_scope_manifest`, `default_rf_pass_config`, `load_rf_chapter`,
  `validate_analysis_output`, `to_export_json`, `pass_config_needs_embedder`
- [x] CLI accepts nested `generate.similarity_map` YAML and flat pass configs
- [x] Lexical-only CLI/PyO3 smoke coverage without ONNX
- [x] Optional `method` on `AnalysisPassRecord` (schema v1 additive)
- [ ] External RF monorepo: retain `method` in settings.yaml parser (out of repo)

## Acceptance gates

- Two-copy exact and near-lexical loops are eligible; three occurrences are not required.
- Sentence, paragraph, and multi-paragraph outputs contain stable document/act/paragraph locations.
- A long repeated run produces one maximal issue per repetition family, not dozens of contained hits.
- Lexical-only analysis is deterministic and runs without ONNX.
- Existing embedding-only YAML and `AnalysisOutput` v1 consumers remain valid.
- The full local manuscript recovers the agreed anchor families without broad chapter-level false positives.

## Corpus baseline notes

Focused fixtures and the manuscript anchor manifest live under `test-data/lexical/`.
See `test-data/lexical/anchors.json` for SHA-256, expected ranges, and fixture mapping.

## Calibration defaults (current)

| Unit | Min tokens | Shingle size | Near threshold |
|---|---|---|---|
| Sentence / phrase | 12 | 3 | 0.82 |
| Paragraph | 8 | 5 | 0.72 |
| Multi-paragraph block | ≥2 paras or ≥80 words | chained paras | 0.70 |

Exact normalized token-sequence matches always qualify. Common shingles are DF-pruned before pair expansion; exact fingerprints bypass pruning.
