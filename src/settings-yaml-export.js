// Paste-ready `generate:similarity_map:` snippets for Romance Factory settings.yaml (THE-326).

import { RF_CHAPTER_PRESETS } from "./rf-chapter-presets.js";

/** Matches similarity-core `report::SCHEMA_VERSION` / contract envelope v1. */
export const SIMILARITY_MAP_SCHEMA_VERSION = "1";

/**
 * @typedef {"rf_preset" | "single"} PassExportMode
 */

/**
 * @typedef {Object} SimilarityMapExportSettings
 * @property {number} phraseLength
 * @property {number} stride
 * @property {number} minRepetitions
 * @property {number} minSamples
 * @property {boolean} enableHdbscan
 * @property {boolean} linkSubphrases
 * @property {string} rfChapterPreset
 * @property {PassExportMode|null} [analysisPassMode]
 * @property {boolean} [enabled]
 * @property {boolean} [expandToSentences]
 * @property {boolean} [preEditorialDedupe]
 * @property {boolean} [injectEditorialIssues]
 */

/**
 * @typedef {Object} ExportPass
 * @property {string} name
 * @property {"act"|"chapter"} scope
 * @property {number} windowSize
 * @property {number} stride
 */

/**
 * Resolve which pass list to emit for the current UI state.
 * @param {SimilarityMapExportSettings} settings
 * @returns {ExportPass[]}
 */
export function resolveExportPasses(settings) {
  const mode = settings.analysisPassMode ?? "single";
  if (mode === "rf_preset") {
    const preset = RF_CHAPTER_PRESETS[settings.rfChapterPreset];
    if (preset?.passes?.length) {
      return preset.passes.map((pass) => ({
        name: pass.name,
        scope: pass.scope,
        windowSize: pass.windowSize,
        stride: pass.stride,
      }));
    }
  }

  const windowSize = settings.phraseLength;
  const stride = settings.stride;
  return [
    {
      name: `chapter_${windowSize}_${stride}`,
      scope: "chapter",
      windowSize,
      stride,
    },
  ];
}

/**
 * @param {string} value
 * @returns {string}
 */
function yamlBool(value) {
  return value ? "true" : "false";
}

/**
 * @param {ExportPass} pass
 * @returns {string}
 */
function formatPassBlock(pass) {
  return [
    "      - name: " + pass.name,
    "        scope: " + pass.scope,
    "        window_size: " + pass.windowSize,
    "        stride: " + pass.stride,
  ].join("\n");
}

/**
 * Build a paste-ready settings.yaml snippet for `generate:similarity_map:`.
 * @param {SimilarityMapExportSettings} settings
 * @returns {string}
 */
export function buildSimilarityMapYamlSnippet(settings) {
  const passes = resolveExportPasses(settings);
  const enabled = settings.enabled ?? true;
  const expandToSentences = settings.expandToSentences ?? true;
  const preEditorialDedupe = settings.preEditorialDedupe ?? true;
  const injectEditorialIssues = settings.injectEditorialIssues ?? true;

  const passMode = settings.analysisPassMode ?? "single";
  const presetLabel =
    passMode === "rf_preset"
      ? RF_CHAPTER_PRESETS[settings.rfChapterPreset]?.label ?? settings.rfChapterPreset
      : "single window/stride";

  const lines = [
    "# Romance Factory settings.yaml snippet — Similarity Map tuned params (THE-326)",
    "# schema_version: " + SIMILARITY_MAP_SCHEMA_VERSION + " (similarity-core contract)",
    "# passes source: " + presetLabel,
    "# Paste under the top-level `generate:` key in settings.yaml.",
    "generate:",
    "  similarity_map:",
    "    enabled: " + yamlBool(enabled),
    "    expand_to_sentences: " + yamlBool(expandToSentences),
    "    pre_editorial_dedupe: " + yamlBool(preEditorialDedupe),
    "    inject_editorial_issues: " + yamlBool(injectEditorialIssues),
    "    min_repetitions: " + settings.minRepetitions,
    "    min_samples: " + settings.minSamples,
    "    enable_hdbscan: " + yamlBool(settings.enableHdbscan),
    "    link_subphrases: " + yamlBool(settings.linkSubphrases),
    "    passes:",
    ...passes.map((pass) => formatPassBlock(pass)),
  ];

  return lines.join("\n") + "\n";
}

/**
 * @param {string} text
 * @returns {Promise<void>}
 */
export async function copyTextToClipboard(text) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.appendChild(textarea);
  textarea.select();
  const ok = document.execCommand("copy");
  document.body.removeChild(textarea);
  if (!ok) {
    throw new Error("Clipboard copy is not available in this environment");
  }
}

/**
 * @param {string} text
 * @param {string} [filename]
 * @returns {Promise<string|null>} Saved path when using Tauri save dialog; null for browser download.
 */
export async function saveTextToFile(text, filename = "similarity-map-settings.yaml") {
  const saveDialog =
    window.__TAURI__?.dialog?.save ??
    (window.__TAURI_INTERNALS__?.invoke
      ? (options) =>
          window.__TAURI_INTERNALS__.invoke("plugin:dialog|save", {
            options,
          })
      : null);

  if (saveDialog) {
    const path = await saveDialog({
      title: "Save settings.yaml snippet",
      defaultPath: filename,
      filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
    });
    if (!path) return null;

    const writeFile =
      window.__TAURI__?.fs?.writeTextFile ??
      (window.__TAURI_INTERNALS__?.invoke
        ? (filePath, contents) =>
            window.__TAURI_INTERNALS__.invoke("plugin:fs|write_text_file", {
              path: filePath,
              contents,
            })
        : null);

    if (!writeFile) {
      throw new Error("Tauri filesystem API not available");
    }

    await writeFile(path, text);
    return path;
  }

  const blob = new Blob([text], { type: "text/yaml;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
  return null;
}

/**
 * Copy snippet to clipboard and optionally save to disk.
 * @param {SimilarityMapExportSettings} settings
 * @param {{ save?: boolean }} [options]
 * @returns {Promise<{ yaml: string, savedPath: string|null }>}
 */
export async function exportSimilarityMapSettingsYaml(settings, options = {}) {
  const yaml = buildSimilarityMapYamlSnippet(settings);
  await copyTextToClipboard(yaml);
  let savedPath = null;
  if (options.save) {
    savedPath = await saveTextToFile(yaml);
  }
  return { yaml, savedPath };
}
