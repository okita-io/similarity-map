# Integration Contract — RepetitionReport v1

Pipeline-consumable JSON contract shared by the Similarity Map UI, CLI, PyO3 bindings, and Romance Factory surgical editor. **Rust serde types in `similarity-core/src/contract.rs` are the source of truth**; `similarity-core/schemas/analysis_output_v1.schema.json` mirrors them for Python validation.

## Envelope: `AnalysisOutput`

| Field | Type | Description |
|---|---|---|
| `schema_version` | `"1"` | Contract version; reject unknown values in consumers |
| `scope` | `AnalysisScope` | Chapter (and optional act) analyzed |
| `scope_manifest` | `ScopeManifest` | Act/paragraph index with stable segment ids |
| `passes` | `AnalysisPassRecord[]` | One entry per analysis pass (act window, chapter window/stride bundle, …) |
| `merged_repetition_report` | `RepetitionReportV1` | Deterministic merge of all passes — **primary RF input** |

Example fixture: [`similarity-core/fixtures/analysis_output_v1.example.json`](../../similarity-core/fixtures/analysis_output_v1.example.json)

JSON Schema: [`similarity-core/schemas/analysis_output_v1.schema.json`](../../similarity-core/schemas/analysis_output_v1.schema.json)

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
| `suggested_op` | `"keep"` \| `"rewrite"` \| `"remove"` \| `"bridge"` | Editorial hint for surgical dedupe |
| `cross_act` | `bool` | Duplicate instances span more than one act |
| `needs_bridge` | `bool` | Cross-act echo with similarity ≥ 0.85 — insert bridge prose before rewrite |

### `suggested_op` derivation (default rules)

1. **`bridge`** — `needs_bridge == true`
2. **`rewrite`** — `cross_act == true` (without bridge threshold)
3. **`remove`** — same-act duplicates with all `similarity_to_centroid ≥ 0.95`
4. **`rewrite`** — default for remaining same-act fuzzy echoes

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
    DeterministicTestEmbedder, SCHEMA_VERSION, TextEmbedder,
};
```

Headless entry point (no Tauri / LanceDB): `analyze_prose(input, options, embedder)` runs paginate → window → embed → cluster → report in memory and returns contract v1 `AnalysisOutput`. Use `DeterministicTestEmbedder` in unit tests without ONNX.

## Related tasks

- **THE-315** — Define RepetitionReport v1 JSON contract (this document)
- Downstream: PyO3 export, RF `settings.yaml` multi-pass bundles, surgical editor hooks
