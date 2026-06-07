// Pipeline-consumable AnalysisOutput JSON export (THE-328).

/**
 * @param {number} chapter
 * @returns {string}
 */
export function repetitionReportFilename(chapter) {
  const chapterNum = Number(chapter);
  if (!Number.isFinite(chapterNum) || chapterNum < 1) {
    return "repetition_report_ch01.json";
  }
  return `repetition_report_ch${String(Math.floor(chapterNum)).padStart(2, "0")}.json`;
}

/**
 * @param {string|null|undefined} storyPath
 * @param {number} chapter
 * @returns {string|null}
 */
export function defaultRepetitionReportSavePath(storyPath, chapter) {
  if (!storyPath) return null;
  const filename = repetitionReportFilename(chapter);
  const separator = storyPath.includes("\\") ? "\\" : "/";
  const trimmed = storyPath.replace(/[/\\]+$/, "");
  return `${trimmed}${separator}repetition_reports${separator}${filename}`;
}

/**
 * @param {object|null|undefined} output
 * @returns {number}
 */
export function chapterFromAnalysisOutput(output) {
  if (!output || typeof output !== "object") return 1;
  const scopeChapter = output.scope?.chapter;
  if (Number.isFinite(scopeChapter) && scopeChapter >= 1) {
    return Math.floor(scopeChapter);
  }
  const manifestChapter = output.scope_manifest?.chapter;
  if (Number.isFinite(manifestChapter) && manifestChapter >= 1) {
    return Math.floor(manifestChapter);
  }
  return 1;
}

/**
 * @param {string} text
 * @param {string} filename
 * @param {string|null} [defaultPath]
 * @returns {Promise<string|null>} Saved path when using Tauri save dialog; null for browser download.
 */
export async function saveJsonToFile(text, filename, defaultPath = null) {
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
      title: "Save repetition report JSON",
      defaultPath: defaultPath || filename,
      filters: [{ name: "JSON", extensions: ["json"] }],
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

  const blob = new Blob([text], { type: "application/json;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
  return null;
}

/**
 * Validate and export v1 AnalysisOutput for the RF pipeline.
 * @param {object} output
 * @param {{ storyPath?: string|null, chapter?: number|null }} [options]
 * @returns {Promise<{ json: string, savedPath: string|null, filename: string }>}
 */
export async function exportAnalysisOutputJson(output, options = {}) {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) {
    throw new Error("Tauri runtime not available");
  }
  if (!output || typeof output !== "object") {
    throw new Error("No pipeline AnalysisOutput available — run RF chapter analysis first.");
  }

  const chapter =
    options.chapter != null && Number.isFinite(options.chapter)
      ? Math.floor(options.chapter)
      : chapterFromAnalysisOutput(output);
  const filename = repetitionReportFilename(chapter);
  const defaultPath = defaultRepetitionReportSavePath(options.storyPath, chapter);

  const json = await invoke("serialize_analysis_output", { output });
  const savedPath = await saveJsonToFile(json, filename, defaultPath);

  return { json, savedPath, filename };
}
