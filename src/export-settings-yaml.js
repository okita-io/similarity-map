// Shared UI wiring for settings.yaml export buttons (THE-326).

import {
  buildSimilarityMapYamlSnippet,
  copyTextToClipboard,
  exportSimilarityMapSettingsYaml,
  saveTextToFile,
} from "./settings-yaml-export.js";

/**
 * @typedef {import("./settings-yaml-export.js").SimilarityMapExportSettings} SimilarityMapExportSettings
 */

/**
 * @param {HTMLElement} container
 * @param {{ getExportSettings: () => SimilarityMapExportSettings|null }} options
 */
export function mountSettingsYamlExportControls(container, options) {
  container.innerHTML = `
    <div class="settings-yaml-export">
      <div class="settings-yaml-export-actions">
        <button type="button" class="btn-secondary btn-export-settings-yaml">
          Export to settings.yaml
        </button>
        <button type="button" class="btn-secondary btn-save-settings-yaml" title="Save snippet to file">
          Save…
        </button>
      </div>
      <div class="settings-yaml-export-status" role="status" aria-live="polite"></div>
    </div>
  `;

  const statusEl = container.querySelector(".settings-yaml-export-status");
  const btnExport = container.querySelector(".btn-export-settings-yaml");
  const btnSave = container.querySelector(".btn-save-settings-yaml");

  /** @param {string} message @param {"info"|"error"} [kind] */
  const setStatus = (message, kind = "info") => {
    statusEl.textContent = message;
    statusEl.dataset.kind = kind;
  };

  /** @param {unknown} err @returns {string} */
  const formatError = (err) => {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && typeof err.message === "string") {
      return err.message;
    }
    return "Export failed";
  };

  btnExport.addEventListener("click", () => {
    void (async () => {
      const settings = options.getExportSettings();
      if (!settings) {
        setStatus("Tune analysis settings first.", "error");
        return;
      }

      btnExport.disabled = true;
      btnSave.disabled = true;
      try {
        await copyTextToClipboard(buildSimilarityMapYamlSnippet(settings));
        setStatus("Copied generate:similarity_map snippet to clipboard.");
      } catch (err) {
        console.error("settings.yaml export failed:", err);
        setStatus(formatError(err), "error");
      } finally {
        btnExport.disabled = false;
        btnSave.disabled = false;
      }
    })();
  });

  btnSave.addEventListener("click", () => {
    void (async () => {
      const settings = options.getExportSettings();
      if (!settings) {
        setStatus("Tune analysis settings first.", "error");
        return;
      }

      btnExport.disabled = true;
      btnSave.disabled = true;
      try {
        const { savedPath } = await exportSimilarityMapSettingsYaml(settings, { save: true });
        if (savedPath) {
          setStatus(`Saved snippet to ${savedPath} (also copied to clipboard).`);
        } else {
          setStatus("Downloaded snippet (also copied to clipboard).");
        }
      } catch (err) {
        console.error("settings.yaml save failed:", err);
        setStatus(formatError(err), "error");
      } finally {
        btnExport.disabled = false;
        btnSave.disabled = false;
      }
    })();
  });

  return {
    setStatus,
    /** @param {SimilarityMapExportSettings} settings */
    async copy(settings) {
      await copyTextToClipboard(buildSimilarityMapYamlSnippet(settings));
    },
    /** @param {SimilarityMapExportSettings} settings */
    async save(settings) {
      const yaml = buildSimilarityMapYamlSnippet(settings);
      await copyTextToClipboard(yaml);
      return saveTextToFile(yaml);
    },
  };
}

/**
 * Build export payload from ImportSettingsPanel.getSettings().
 * @param {ReturnType<import("./import-settings.js").ImportSettingsPanel["getSettings"]>} panelSettings
 * @param {import("./settings-yaml-export.js").PassExportMode|null} analysisPassMode
 * @returns {SimilarityMapExportSettings}
 */
export function buildExportSettingsFromPanel(panelSettings, analysisPassMode) {
  return {
    phraseLength: panelSettings.phraseLength,
    stride: panelSettings.stride,
    minRepetitions: panelSettings.minRepetitions,
    minSamples: panelSettings.minSamples,
    enableHdbscan: panelSettings.enableHdbscan,
    linkSubphrases: panelSettings.linkSubphrases,
    rfChapterPreset: panelSettings.rfChapterPreset,
    analysisPassMode,
  };
}

/**
 * Infer pass export mode from ImportSettingsPanel state.
 * @param {import("./import-settings.js").ImportSettingsPanel} panel
 * @returns {import("./settings-yaml-export.js").PassExportMode}
 */
export function inferAnalysisPassMode(panel) {
  if (panel._lastAnalysisPassMode === "rf_preset") {
    return "rf_preset";
  }
  if (panel._lastAnalysisPassMode === "single") {
    return "single";
  }
  if (panel.rfStoryPath && panel.rfChapter) {
    return "rf_preset";
  }
  return "single";
}
