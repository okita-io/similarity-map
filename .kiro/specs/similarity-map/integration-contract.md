# Integration Contract — AnalysisOutput v1

Pipeline-consumable JSON contract shared by the Similarity Map UI, CLI, PyO3 bindings, and Romance Factory surgical editor. **Rust serde types in `similarity-core/src/contract.rs` are the source of truth**; `similarity-core/schemas/analysis_output_v1.schema.json` mirrors them for Python validation.

## Envelope: `AnalysisOutput`

| Field | Type | Description |
|---|---|---|
| `schema_version` | `"1"` | Contract version; reject unknown values in consumers |
| `scope` | `AnalysisScope` | Chapter (and optional act) analyzed |
| `scope_manifest` | `ScopeManifest` | Act/paragraph index with stable segment ids |
| `passes` | `AnalysisPassRecord[]` | One entry per analysis pass (act window, chapter window/stride bundle, …) |
| `merged_repetition_report` | `RepetitionReportV1` | Deterministic merge of all passes — **primary RF input** |

Example fixture: [`similarity-core/fixtures/analysis_output_v1.example.json`](../../../similarity-core/fixtures/analysis_output_v1.example.json)

JSON Schema: [`similarity-core/schemas/analysis_output_v1.schema.json`](../../../similarity-core/schemas/analysis_output_v1.schema.json)

## `ScopeManifest` and `segment_id`

Acts are numbered 1-based within a chapter. Paragraphs are numbered 1-based within an act.

**Segment id format:** `ch{NN}_a{MM}_p{PP}` (zero-padded, e.g. `ch01_a02_p03`).

Each `ParagraphIndexEntry` records:

- `scope_char_start` / `scope_char_end` — offsets within the chapter scope text
- `doc_char_start` / `doc_char_end` — absolute manuscript offsets

Romance Factory builds the manifest from draft chapter structure (acts separated by `\n\n`, paragraphs by `\n`) via `build_scope_manifest()`.

## `SpanLocation` on every `EditSpanV1`

Each edit span carries structural location in addition to raw text:

| Field | Description |
|---|---|
| `chapter`, `act`, `paragraph_index` | Structural coordinates |
| `segment_id` | Stable segment reference matching the manifest |
| `sentence_index` | 1-based sentence within the paragraph |
| `scope_char_start` / `scope_char_end` | Offsets within chapter scope |
| `doc_char_start` / `doc_char_end` | Absolute manuscript offsets |

Legacy `RepetitionReport` spans (doc offsets only) upgrade via `repetition_report_to_v1()`.

## Cluster enrichments

| Field | Type | Description |
|---|---|---|
| `suggested_op` | `"delete_span"` \| `"rewrite_span"` \| `"replace_paragraph"` | RF PatchPlanner op — route without re-deriving heuristics |
| `cross_act` | `bool` | Duplicate instances span more than one act |
| `needs_bridge` | `bool` | Mid-act paragraph-sized duplicate (>40 words) — insert bridge prose before destructive edit |
| `boundary_version` | `1` | Sentence-boundary expansion version for RF span expansion parity |

### `suggested_op` derivation (default rules)

1. **`replace_paragraph`** — duplicate blast radius > 40 words (whole-paragraph echo)
2. **`delete_span`** — same-act duplicates with all `similarity_to_centroid ≥ 0.95` and blast radius ≤ 15 words
3. **`rewrite_span`** — cross-act echoes under paragraph size, or remaining same-act fuzzy echoes

Blast radius is the max whitespace-delimited word count across **duplicate** instances (`duplicate_blast_radius_words`).

### `needs_bridge`

Set when `cross_act == false` and duplicate blast radius > 40 words — signals the surgical editor to insert transitional bridge prose before replacing or deleting the paragraph-sized echo mid-act.

## Multi-pass merge rules

When combining `passes[]` into `merged_repetition_report`:

1. **Cluster union** — Two clusters from different passes merge when any instance span pair overlaps ≥ 50% of the shorter span (by character length).
2. **Stable ids** — Merged clusters receive new sequential `cluster_id` values starting at 1 in document order.
3. **Instance deduplication** — Within a merged cluster, drop spans whose doc ranges overlap ≥ 50% with an existing span; keep the earlier pass’s span text when tied.
4. **Canonical selection** — Earliest `doc_char_start` wins; renumber `instance_id` in document order.
5. **Enrichment recompute** — Recompute `cross_act`, `needs_bridge`, and `suggested_op` on the merged span set.
6. **Stats** — `cluster_count`, `total_duplicate_instances`, and `total_duplicate_words_estimate` reflect the merged report.

Pass order does not affect merge outcome except for tie-breaking duplicate span text (first pass wins).

## Romance Factory consumption

```python
import json
from jsonschema import validate
import jsonschema

with open("analysis_output_v1.schema.json") as f:
    schema = json.load(f)

with open("chapter_01.repetition.json") as f:
    output = json.load(f)

validate(instance=output, schema=schema)
report = output["merged_repetition_report"]
for cluster in report["clusters"]:
    op = cluster["suggested_op"]
    for dup in cluster["duplicates"]:
        loc = dup["location"]
        # surgical patch at loc["doc_char_start"]:loc["doc_char_end"]
        ...
```

No ad-hoc field renaming or nested unwrapping is required — the envelope is consumed directly.

## Rust API

```rust
use similarity_core::{
    analyze_prose, analyze_prose_with_model, build_analysis_output,
    build_analysis_output_with_manifest, from_export_json, merge_pass_reports,
    repetition_report_to_v1, AnalysisInput, AnalyzeProseOptions, AnalysisOutput,
    BOUNDARY_VERSION, DeterministicTestEmbedder, derive_cluster_enrichments_v1, SCHEMA_VERSION,
    TextEmbedder,
};
```

Headless entry point (no Tauri / LanceDB): `analyze_prose(input, options, embedder)` runs paginate → window → embed → cluster → report in memory and returns contract v1 `AnalysisOutput`. Use `DeterministicTestEmbedder` in unit tests without ONNX.

Multi-pass orchestration: `analyze_prose_multi_pass(MultiPassInput, embedder)` runs the default RF 4-pass bundle (`default_rf_multi_pass_config()`) — act-scoped passes per act, chapter-scoped passes on the full chapter — then merges via `merge_pass_reports`.

## Desktop IPC

The Tauri adapter exposes the same contract through these commands:

| Command | Contract role |
|---|---|
| `analyze_text` | Single-pass in-memory analysis; the desktop visualization response includes `analysis_output` |
| `list_rf_chapters` | Discovers RF chapter drafts for the story picker |
| `build_rf_chapter_scope` | Returns chapter text and its `ScopeManifest` |
| `estimate_rf_chapter` | Estimates windows for the selected pass preset |
| `analyze_rf_chapter` | Runs the selected multi-pass bundle and returns visualization plus `AnalysisOutput` |
| `serialize_analysis_output` | Validates and pretty-prints v1 JSON for export |

The desktop's `analyze_document` command is a separate LanceDB-backed path used for
checkpointing, cancellation, and session restore. New pipeline integrations should
consume `AnalysisOutput` rather than depending on Tauri commands or LanceDB rows.

## UI YAML round-trip (THE-344)

The Similarity Map app exports a paste-ready `generate:similarity_map:` block
(`src/settings-yaml-export.js`). Romance Factory loads it from
repo-root `settings.yaml` without field renaming.

### Export shape

```yaml
generate:
  similarity_map:
    enabled: true
    expand_to_sentences: true
    pre_editorial_dedupe: true
    inject_editorial_issues: true
    min_repetitions: 3
    min_samples: 3
    enable_hdbscan: true
    link_subphrases: false
    passes:
      - name: act_50_10
        scope: act
        window_size: 50
        stride: 10
      # … additional passes …
```

RF preset **full multi-pass** matches `default_similarity_map_passes()` in the external
Romance Factory monorepo module `romance_factory.generate.similarity_map_config`.

### RF consumption path

1. `parse_similarity_map_config(yaml_block)` → `SimilarityMapConfig`
2. `pass_config_from_similarity_map(config)` → PyO3 `analyze_prose_multi_pass` JSON params
3. `analyze_chapter(story_path, chapter, config)` → v1 `AnalysisOutput`
4. `report_to_patch_ops(merged_report)` → surgical `delete_span` pre-editorial dedupe

Round-trip acceptance: same `pass_config` and `dedupe_outcome_fingerprint` whether
params are parsed from UI YAML or set in Python directly. The referenced fixtures and
procedure live in the **external Romance Factory monorepo**, not this standalone
checkout:

- `tests/fixtures/repetition/ui_export/`
- `docs/design/similarity-map-yaml-roundtrip.md`

## Related tasks

- **THE-315** — Define RepetitionReport v1 JSON contract (this document)
- **THE-326** — Similarity Map UI → settings.yaml export
- **THE-344** — UI YAML → pipeline round-trip validation
- Downstream: PyO3 export, RF `settings.yaml` multi-pass bundles, surgical editor hooks
