// Import Settings Panel — manages analysis parameter controls and live estimates
// Tauri 2 IPC via window.__TAURI__.core.invoke

import { ProgressView } from "./progress-view.js";
import { bindSliderNumberInput } from "./slider-input.js";
import { activateJob } from "./job-activation.js";
import { applyVisualizationPayload } from "./text-preview.js";
import {
  buildExportSettingsFromPanel,
  inferAnalysisPassMode,
  mountSettingsYamlExportControls,
} from "./export-settings-yaml.js";

import {
  DEFAULT_RF_CHAPTER_PRESET,
  RF_CHAPTER_PRESETS,
} from "./rf-chapter-presets.js";

/**
 * @typedef {Object} RfPassEstimate
 * @property {string} name
 * @property {string} scope
 * @property {number} window_size
 * @property {number} stride
 * @property {number} window_count
 */

/**
 * @typedef {Object} RfChapterPassEstimate
 * @property {string} preset
 * @property {RfPassEstimate[]} passes
 * @property {number} total_windows
 */

/**
 * @typedef {Object} AnalysisEstimate
 * @property {number} page_count
 * @property {number} window_count
 * @property {number} eta_seconds
 * @property {number} benchmark_windows_per_sec
 */

export class ImportSettingsPanel {
  /**
   * @param {HTMLElement} container - The panel container element
   * @param {Object} [options]
   * @param {string} [options.filePath] - Path to the loaded document
   * @param {boolean} [options.isPdf] - Whether the loaded file is a PDF
   */
  constructor(container, options = {}) {
    this.container = container;
    this.filePath = options.filePath || "";
    this.isPdf = options.isPdf || false;

    /** @type {string|null} Romance Factory story directory path */
    this.rfStoryPath = null;
    /** @type {number|null} Selected RF chapter number */
    this.rfChapter = null;
    /** @type {number[]} */
    this._rfChapters = [];
    /** @type {string} */
    this.rfChapterPreset = DEFAULT_RF_CHAPTER_PRESET;
    /** @type {RfChapterPassEstimate|null} */
    this._lastRfPassEstimate = null;

    /** @type {"rf_preset"|"single"|null} Last analysis pass mode for YAML export */
    this._lastAnalysisPassMode = null;

    /** @type {ReturnType<typeof mountSettingsYamlExportControls>|null} */
    this._settingsYamlExport = null;

    // Track whether user has manually overridden stride
    this._strideManuallySet = false;

    // Debounce timer for estimate updates
    this._estimateTimer = null;

    /** @type {AnalysisEstimate|null} */
    this._lastEstimate = null;

    // Progress view reference (active during analysis)
    this._progressView = null;

    // Saved settings snapshot for restore on cancellation
    this._savedSettings = null;

    // Default values
    this._defaults = {
      tokensPerPage: 400,
      phraseLength: 20,
      stride: 5, // max(1, floor(20 * 0.25))
      minRepetitions: 3,
      minSamples: 3,
      enableHdbscan: true,
      linkSubphrases: false,
      chapterBreak: "^Chapter\\s+\\d+",
    };

    this._buildUI();
    this._attachListeners();
    void this._loadAppSettings();
    this._updateEstimate();
  }

  /** Build the panel DOM structure */
  _buildUI() {
    this.container.innerHTML = "";

    this.container.innerHTML = `
      <div class="import-settings-panel">
        <h2 class="import-settings-title">Import Settings</h2>

        <div class="import-file-section">
          <button id="btn-open-file" class="btn-open-file" type="button">
            <span class="btn-open-file-icon" aria-hidden="true">📄</span>
            <span class="btn-open-file-text">Open Document</span>
          </button>
          <div class="import-file-name" id="import-file-name"></div>
        </div>

        <div class="import-rf-section">
          <button id="btn-load-rf-story" class="btn-open-file btn-rf-story" type="button">
            <span class="btn-open-file-icon" aria-hidden="true">📁</span>
            <span class="btn-open-file-text">Load RF Story</span>
          </button>
          <div class="import-rf-meta" id="import-rf-meta" hidden>
            <div class="import-rf-story-name" id="import-rf-story-name"></div>
            <label class="setting-label" for="select-rf-preset">RF chapter preset</label>
            <select id="select-rf-preset" class="setting-select" aria-label="RF chapter preset">
              ${Object.entries(RF_CHAPTER_PRESETS)
                .map(
                  ([id, preset]) =>
                    `<option value="${id}"${id === this.rfChapterPreset ? " selected" : ""}>${preset.label}</option>`,
                )
                .join("")}
            </select>
            <label class="setting-label" for="select-rf-chapter">Chapter</label>
            <select id="select-rf-chapter" class="setting-select" aria-label="RF chapter">
              <option value="">Select chapter…</option>
            </select>
            <div class="import-rf-scope-note setting-note" id="import-rf-scope-note"></div>
            <button id="btn-analyze-rf-chapter" class="btn-secondary" type="button" disabled>
              Analyze RF Chapter
            </button>
          </div>
        </div>

        <div class="import-settings-controls">
          <div class="setting-group" id="setting-tokens-per-page">
            <label class="setting-label" for="slider-tokens-per-page">
              <span>Tokens per Page:</span>
              <input
                type="number"
                id="input-tokens-per-page"
                class="setting-number-input"
                min="200"
                max="2000"
                step="10"
                value="${this._defaults.tokensPerPage}"
                aria-label="Tokens per Page value"
                ${this.isPdf ? "disabled" : ""}
              />
            </label>
            <input
              type="range"
              id="slider-tokens-per-page"
              class="setting-slider"
              min="200"
              max="2000"
              step="10"
              value="${this._defaults.tokensPerPage}"
              ${this.isPdf ? "disabled" : ""}
              aria-label="Tokens per Page"
            />
            ${this.isPdf ? '<span class="setting-note">Disabled for PDF imports</span>' : ""}
            <div class="setting-warning" id="warning-tokens-per-page" hidden></div>
          </div>

          <div class="setting-group">
            <label class="setting-label" for="slider-phrase-length">
              <span>Phrase Length:</span>
              <input
                type="number"
                id="input-phrase-length"
                class="setting-number-input"
                min="5"
                max="1500"
                step="1"
                value="${this._defaults.phraseLength}"
                aria-label="Phrase Length value"
              />
            </label>
            <input
              type="range"
              id="slider-phrase-length"
              class="setting-slider"
              min="5"
              max="1500"
              step="1"
              value="${this._defaults.phraseLength}"
              aria-label="Phrase Length"
            />
          </div>

          <div class="setting-group">
            <label class="setting-label" for="slider-stride">
              <span>Stride:</span>
              <input
                type="number"
                id="input-stride"
                class="setting-number-input"
                min="1"
                max="200"
                step="1"
                value="${this._defaults.stride}"
                aria-label="Stride value"
              />
            </label>
            <input
              type="range"
              id="slider-stride"
              class="setting-slider"
              min="1"
              max="200"
              step="1"
              value="${this._defaults.stride}"
              aria-label="Stride"
            />
          </div>

          <div class="setting-group">
            <label class="setting-label" for="slider-min-repetitions">
              <span>Min Repetitions:</span>
              <input
                type="number"
                id="input-min-repetitions"
                class="setting-number-input"
                min="2"
                max="20"
                step="1"
                value="${this._defaults.minRepetitions}"
                aria-label="Min Repetitions value"
              />
            </label>
            <input
              type="range"
              id="slider-min-repetitions"
              class="setting-slider"
              min="2"
              max="20"
              step="1"
              value="${this._defaults.minRepetitions}"
              aria-label="Min Repetitions"
            />
          </div>

          <div class="setting-group">
            <label class="setting-label" for="slider-min-samples">
              <span>Min Samples:</span>
              <input
                type="number"
                id="input-min-samples"
                class="setting-number-input"
                min="1"
                max="10"
                step="1"
                value="${this._defaults.minSamples}"
                aria-label="Min Samples value"
              />
            </label>
            <input
              type="range"
              id="slider-min-samples"
              class="setting-slider"
              min="1"
              max="10"
              step="1"
              value="${this._defaults.minSamples}"
              aria-label="Min Samples"
            />
            <span class="setting-note" id="note-min-samples">HDBSCAN noise sensitivity — higher is stricter</span>
          </div>

          <div class="setting-group setting-checkbox-group">
            <label class="setting-checkbox-label" for="checkbox-enable-hdbscan">
              <input
                type="checkbox"
                id="checkbox-enable-hdbscan"
                class="setting-checkbox"
                ${this._defaults.enableHdbscan ? "checked" : ""}
              />
              Enable HDBSCAN density scan
            </label>
            <span class="setting-note" id="note-enable-hdbscan">
              When off, grouping uses KMeans similarity only (Min Samples is ignored).
            </span>
          </div>

          <div class="setting-group setting-checkbox-group">
            <label class="setting-checkbox-label" for="checkbox-link-subphrases">
              <input
                type="checkbox"
                id="checkbox-link-subphrases"
                class="setting-checkbox"
                ${this._defaults.linkSubphrases ? "checked" : ""}
              />
              Link subphrases to parent blocks
            </label>
            <span class="setting-note" id="note-link-subphrases">
              Merge short repeated phrases into larger repeated paragraphs so they share one color.
            </span>
          </div>

          <div class="setting-group">
            <label class="setting-label" for="input-chapter-break">
              Chapter Break Regex:
            </label>
            <input
              type="text"
              id="input-chapter-break"
              class="setting-text-input"
              value="${this._defaults.chapterBreak}"
              placeholder="e.g. ^Chapter\\s+\\d+"
              aria-label="Chapter Break Regex"
            />
            <div class="setting-error" id="error-chapter-break" hidden></div>
          </div>
        </div>

        <div class="import-settings-estimate" id="estimate-display">
          <div class="estimate-row">
            <span class="estimate-label">Estimated windows:</span>
            <span class="estimate-value" id="estimate-window-count">—</span>
          </div>
          <div class="estimate-row">
            <span class="estimate-label">Estimated time:</span>
            <span class="estimate-value" id="estimate-time">—</span>
          </div>
          <div class="rf-pass-estimates" id="rf-pass-estimates" hidden>
            <div class="rf-pass-estimates-title">Multi-pass window estimate</div>
            <div class="rf-pass-estimates-list" id="rf-pass-estimates-list"></div>
            <div class="estimate-row rf-pass-estimates-total">
              <span class="estimate-label">Total windows (all passes):</span>
              <span class="estimate-value" id="rf-pass-estimates-total">—</span>
            </div>
          </div>
          <div class="estimate-nudge" id="estimate-nudge" hidden>
            ⏱ Estimated time exceeds 30 minutes. Consider increasing Stride to reduce processing time.
          </div>
        </div>

        <div class="import-paste-section">
          <label class="setting-label" for="paste-text-area">
            Paste text (Romance Factory output)
          </label>
          <textarea
            id="paste-text-area"
            class="paste-text-area"
            rows="5"
            placeholder="Paste a chapter or excerpt to tune phrase length and tolerance against LLM output…"
            aria-label="Pasted manuscript text"
          ></textarea>
          <button id="btn-analyze-text" class="btn-secondary" type="button">
            Analyze Pasted Text
          </button>
        </div>

        <div class="import-settings-actions">
          <button id="btn-analyze" class="btn-analyze" type="button">Analyze</button>
        </div>

        <div class="import-settings-export" id="import-settings-export"></div>
      </div>
    `;

    // Cache element references
    this._els = {
      btnOpenFile: this.container.querySelector("#btn-open-file"),
      fileName: this.container.querySelector("#import-file-name"),
      tokensPerPage: this.container.querySelector("#slider-tokens-per-page"),
      inputTokensPerPage: this.container.querySelector("#input-tokens-per-page"),
      phraseLength: this.container.querySelector("#slider-phrase-length"),
      inputPhraseLength: this.container.querySelector("#input-phrase-length"),
      stride: this.container.querySelector("#slider-stride"),
      inputStride: this.container.querySelector("#input-stride"),
      minRepetitions: this.container.querySelector("#slider-min-repetitions"),
      inputMinRepetitions: this.container.querySelector("#input-min-repetitions"),
      minSamples: this.container.querySelector("#slider-min-samples"),
      inputMinSamples: this.container.querySelector("#input-min-samples"),
      enableHdbscan: this.container.querySelector("#checkbox-enable-hdbscan"),
      linkSubphrases: this.container.querySelector("#checkbox-link-subphrases"),
      chapterBreak: this.container.querySelector("#input-chapter-break"),
      warningTokensPerPage: this.container.querySelector("#warning-tokens-per-page"),
      errorChapterBreak: this.container.querySelector("#error-chapter-break"),
      estimateWindowCount: this.container.querySelector("#estimate-window-count"),
      estimateTime: this.container.querySelector("#estimate-time"),
      estimateNudge: this.container.querySelector("#estimate-nudge"),
      btnAnalyze: this.container.querySelector("#btn-analyze"),
      btnLoadRfStory: this.container.querySelector("#btn-load-rf-story"),
      rfMeta: this.container.querySelector("#import-rf-meta"),
      rfStoryName: this.container.querySelector("#import-rf-story-name"),
      rfPresetSelect: this.container.querySelector("#select-rf-preset"),
      rfChapterSelect: this.container.querySelector("#select-rf-chapter"),
      rfScopeNote: this.container.querySelector("#import-rf-scope-note"),
      rfPassEstimates: this.container.querySelector("#rf-pass-estimates"),
      rfPassEstimatesList: this.container.querySelector("#rf-pass-estimates-list"),
      rfPassEstimatesTotal: this.container.querySelector("#rf-pass-estimates-total"),
      btnAnalyzeRfChapter: this.container.querySelector("#btn-analyze-rf-chapter"),
      pasteTextArea: this.container.querySelector("#paste-text-area"),
      btnAnalyzeText: this.container.querySelector("#btn-analyze-text"),
      exportContainer: this.container.querySelector("#import-settings-export"),
    };

    this._mountSettingsYamlExport();
  }

  _mountSettingsYamlExport() {
    if (!this._els.exportContainer) return;
    this._settingsYamlExport = mountSettingsYamlExportControls(this._els.exportContainer, {
      getExportSettings: () => this.getExportSettings(),
    });
  }

  /** Attach event listeners to all controls */
  _attachListeners() {
    // Open Document button
    this._els.btnOpenFile.addEventListener("click", () => {
      this._openFileDialog();
    });

    const onPhraseLengthChange = () => {
      const val = Number(this._els.phraseLength.value);
      if (!this._strideManuallySet) {
        const newStride = Math.max(1, Math.floor(val * 0.25));
        this._els.stride.value = newStride;
        this._els.inputStride.value = newStride;
      }
      this._checkTokensWarning();
      this._scheduleEstimateUpdate();
    };

    bindSliderNumberInput(this._els.tokensPerPage, this._els.inputTokensPerPage, {
      onInput: () => {
        this._checkTokensWarning();
        this._scheduleEstimateUpdate();
      },
    });

    bindSliderNumberInput(this._els.phraseLength, this._els.inputPhraseLength, {
      onInput: onPhraseLengthChange,
      onChange: onPhraseLengthChange,
    });

    bindSliderNumberInput(this._els.stride, this._els.inputStride, {
      onInput: () => {
        this._strideManuallySet = true;
        this._scheduleEstimateUpdate();
      },
      onChange: () => {
        this._strideManuallySet = true;
        this._scheduleEstimateUpdate();
      },
    });

    bindSliderNumberInput(this._els.minRepetitions, this._els.inputMinRepetitions, {
      onInput: () => this._scheduleEstimateUpdate(),
    });

    bindSliderNumberInput(this._els.minSamples, this._els.inputMinSamples, {
      onInput: () => this._scheduleEstimateUpdate(),
    });

    // HDBSCAN enable checkbox — disable Min Samples when bypassed
    this._els.enableHdbscan.addEventListener("change", () => {
      this._updateHdbscanDependentControls();
      this._scheduleEstimateUpdate();
    });

    this._updateHdbscanDependentControls();

    // Chapter Break regex input
    this._els.chapterBreak.addEventListener("input", () => {
      this._validateChapterBreak();
      this._scheduleEstimateUpdate();
    });

    // Analyze button
    this._els.btnAnalyze.addEventListener("click", () => {
      this._startAnalysis();
    });

    this._els.btnLoadRfStory.addEventListener("click", () => {
      void this._openRfStoryDialog();
    });

    this._els.rfPresetSelect.addEventListener("change", () => {
      void this._onRfPresetSelected();
    });

    this._els.rfChapterSelect.addEventListener("change", () => {
      void this._onRfChapterSelected();
    });

    this._els.btnAnalyzeRfChapter.addEventListener("click", () => {
      void this._startRfChapterAnalysis();
    });

    this._els.btnAnalyzeText.addEventListener("click", () => {
      this._startTextAnalysis();
    });
  }

  /** Load persisted app settings (last-used RF preset). */
  async _loadAppSettings() {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return;

    try {
      const settings = await invoke("get_app_settings");
      const preset = settings?.rf_chapter_preset;
      if (preset && RF_CHAPTER_PRESETS[preset]) {
        this.rfChapterPreset = preset;
        if (this._els?.rfPresetSelect) {
          this._els.rfPresetSelect.value = preset;
        }
        this._applyRfPresetSliders(preset, { persist: false });
      }
    } catch (err) {
      console.warn("get_app_settings failed:", err);
    }
  }

  /**
   * @param {string} presetId
   * @param {{ persist?: boolean }} [options]
   */
  _applyRfPresetSliders(presetId, options = {}) {
    const preset = RF_CHAPTER_PRESETS[presetId];
    if (!preset) return;

    this.rfChapterPreset = presetId;
    if (this._els?.rfPresetSelect && this._els.rfPresetSelect.value !== presetId) {
      this._els.rfPresetSelect.value = presetId;
    }

    const { windowSize, stride } = preset.displayPass;
    this._strideManuallySet = true;
    this._els.phraseLength.value = windowSize;
    this._els.inputPhraseLength.value = windowSize;
    this._els.stride.value = stride;
    this._els.inputStride.value = stride;
    this._checkTokensWarning();
    this._scheduleEstimateUpdate();

    if (options.persist !== false) {
      void this._persistRfPreset(presetId);
    }
  }

  async _onRfPresetSelected() {
    const presetId = this._els.rfPresetSelect.value;
    if (!RF_CHAPTER_PRESETS[presetId]) return;
    this._applyRfPresetSliders(presetId);
  }

  async _persistRfPreset(presetId) {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return;

    try {
      await invoke("save_app_settings", { rfChapterPreset: presetId });
    } catch (err) {
      console.warn("save_app_settings failed:", err);
    }
  }

  /** Render per-pass window counts for the selected RF chapter + preset. */
  _renderRfPassEstimates(estimate) {
    if (!this._els.rfPassEstimates || !this._els.rfPassEstimatesList) return;

    if (!estimate?.passes?.length) {
      this._els.rfPassEstimates.hidden = true;
      this._els.rfPassEstimatesList.innerHTML = "";
      this._els.rfPassEstimatesTotal.textContent = "—";
      return;
    }

    this._els.rfPassEstimates.hidden = false;
    this._els.rfPassEstimatesList.innerHTML = estimate.passes
      .map((pass) => {
        const scopeLabel = pass.scope === "act" ? "act" : "chapter";
        return `
          <div class="estimate-row rf-pass-estimate-row">
            <span class="estimate-label">${pass.name} (${scopeLabel} ${pass.window_size}/${pass.stride})</span>
            <span class="estimate-value">${pass.window_count.toLocaleString()}</span>
          </div>
        `;
      })
      .join("");
    this._els.rfPassEstimatesTotal.textContent = estimate.total_windows.toLocaleString();
  }

  async _updateRfPassEstimates() {
    if (!this.rfStoryPath || !this.rfChapter) {
      this._lastRfPassEstimate = null;
      this._renderRfPassEstimates(null);
      return;
    }

    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      this._renderRfPassEstimates(null);
      return;
    }

    try {
      /** @type {RfChapterPassEstimate} */
      const estimate = await invoke("estimate_rf_chapter", {
        storyPath: this.rfStoryPath,
        chapter: this.rfChapter,
        preset: this.rfChapterPreset,
      });
      this._lastRfPassEstimate = estimate;
      this._renderRfPassEstimates(estimate);
    } catch (err) {
      console.warn("estimate_rf_chapter failed:", err);
      this._lastRfPassEstimate = null;
      this._renderRfPassEstimates(null);
    }
  }

  /** Open a native folder picker for a Romance Factory story directory. */
  async _openRfStoryDialog() {
    const dialogOptions = {
      multiple: false,
      directory: true,
      title: "Load Romance Factory Story",
    };

    try {
      const openDialog =
        window.__TAURI__?.dialog?.open ??
        (window.__TAURI_INTERNALS__?.invoke
          ? (opts) =>
              window.__TAURI_INTERNALS__.invoke("plugin:dialog|open", {
                options: opts,
              })
          : null);

      if (!openDialog) {
        console.warn("Tauri dialog API not available");
        return;
      }

      const selected = await openDialog(dialogOptions);
      if (!selected) return;

      const storyPath = typeof selected === "string" ? selected : selected.path;
      if (!storyPath) return;

      await this._setRfStory(storyPath);
    } catch (err) {
      console.error("RF story dialog failed:", err);
      this._showAnalysisError(err);
    }
  }

  /**
   * @param {string} storyPath
   */
  async _setRfStory(storyPath) {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      this._showAnalysisError("Tauri runtime not available (running in a browser?)");
      return;
    }

    this.rfStoryPath = storyPath;
    this.rfChapter = null;
    this._rfChapters = [];

    const storyName = storyPath.split(/[/\\]/).pop() || storyPath;
    this._els.rfStoryName.textContent = storyName;
    this._els.rfStoryName.title = storyPath;
    this._els.rfMeta.hidden = false;
    this._els.rfScopeNote.textContent = "";
    this._els.btnAnalyzeRfChapter.disabled = true;

    const select = this._els.rfChapterSelect;
    select.innerHTML = '<option value="">Select chapter…</option>';

    try {
      const list = await invoke("list_rf_chapters", { storyPath });
      this._rfChapters = list?.chapters || [];
      for (const ch of this._rfChapters) {
        const opt = document.createElement("option");
        opt.value = String(ch);
        opt.textContent = `Chapter ${ch}`;
        select.appendChild(opt);
      }
      if (this._rfChapters.length === 0) {
        this._els.rfScopeNote.textContent =
          "No chapters found — add drafts/chapter_XX_act_YY.json files.";
      }
    } catch (err) {
      console.error("list_rf_chapters failed:", err);
      this._showAnalysisError(err);
    }
  }

  async _onRfChapterSelected() {
    const raw = this._els.rfChapterSelect.value;
    if (!raw || !this.rfStoryPath) {
      this.rfChapter = null;
      this._els.rfScopeNote.textContent = "";
      this._els.btnAnalyzeRfChapter.disabled = true;
      return;
    }

    const chapter = Number(raw);
    this.rfChapter = chapter;
    this._els.btnAnalyzeRfChapter.disabled = false;

    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return;

    try {
      const scope = await invoke("build_rf_chapter_scope", {
        storyPath: this.rfStoryPath,
        chapter,
      });
      this._els.rfScopeNote.textContent =
        `${scope.act_count} act(s), ${scope.paragraph_count} paragraph(s) — ` +
        `${scope.text.length.toLocaleString()} chars`;
      await this._updateRfPassEstimates();
    } catch (err) {
      console.warn("build_rf_chapter_scope failed:", err);
      this._els.rfScopeNote.textContent = this._formatInvokeError(err);
      this._els.btnAnalyzeRfChapter.disabled = true;
    }
  }

  /** Analyze the selected RF chapter with pipeline multi-pass + act-scoped grid pages. */
  async _startRfChapterAnalysis() {
    if (!this.rfStoryPath || !this.rfChapter) {
      this._showAnalysisError("Select an RF story and chapter first.");
      return;
    }

    this._lastAnalysisPassMode = "rf_preset";
    const settings = this.getSettings();
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      this._showAnalysisError("Tauri runtime not available (running in a browser?)");
      return;
    }

    this._savedSettings = { ...settings };
    this._els.btnAnalyzeRfChapter.disabled = true;

    try {
      this._showProgressView("");
    } catch (renderErr) {
      console.error("Failed to render progress view:", renderErr);
      this._els.btnAnalyzeRfChapter.disabled = false;
      return;
    }

    try {
      window.currentJobId = "";
      const payload = await invoke("analyze_rf_chapter", {
        storyPath: this.rfStoryPath,
        chapter: this.rfChapter,
        preset: settings.rfChapterPreset,
        windowSize: settings.phraseLength,
        stride: settings.stride,
        tokensPerPage: settings.tokensPerPage,
        minRepetitions: settings.minRepetitions,
        minSamples: settings.minSamples,
        enableHdbscan: settings.enableHdbscan,
        linkSubphrases: settings.linkSubphrases,
      });

      if (payload?.job_id) {
        if (this._progressView) {
          this._progressView.setJobId(payload.job_id);
        }
        const label = `${this.rfStoryPath.split(/[/\\]/).pop() || "story"} ch${this.rfChapter}`;
        this.filePath = this.rfStoryPath;
        this.isPdf = false;
        this._els.fileName.textContent = label;
        this._els.fileName.title = this.rfStoryPath;
        await applyVisualizationPayload(payload);
      }
    } catch (err) {
      console.error("analyze_rf_chapter failed:", err);
      this._showAnalysisError(err);
    } finally {
      this._els.btnAnalyzeRfChapter.disabled = false;
      await this._restoreSettingsView();
    }
  }

  /** Open a native file dialog to select a document */
  async _openFileDialog() {
    const dialogOptions = {
      multiple: false,
      directory: false,
      title: "Open Document",
      filters: [
        { name: "Documents", extensions: ["txt", "pdf", "md", "text"] },
        { name: "Plain Text", extensions: ["txt", "text", "md"] },
        { name: "PDF", extensions: ["pdf"] },
        { name: "All Files", extensions: ["*"] },
      ],
    };

    try {
      const openDialog =
        window.__TAURI__?.dialog?.open ??
        (window.__TAURI_INTERNALS__?.invoke
          ? (opts) =>
              window.__TAURI_INTERNALS__.invoke("plugin:dialog|open", {
                options: opts,
              })
          : null);

      if (!openDialog) {
        console.warn("Tauri dialog API not available");
        return;
      }

      const selected = await openDialog(dialogOptions);

      if (!selected) return; // User cancelled

      const filePath = typeof selected === "string" ? selected : selected.path;
      if (!filePath) return;

      const isPdf = filePath.toLowerCase().endsWith(".pdf");
      const fileName = filePath.split(/[/\\]/).pop() || filePath;

      // Update the file name display
      this._els.fileName.textContent = fileName;
      this._els.fileName.title = filePath;

      // Set the file and update estimates
      this.resetStrideOverride();
      this.setFile(filePath, isPdf);

      if (window.resultsPanel) {
        void window.resultsPanel.refresh();
      }

      // Check for existing sessions
      this._checkExistingSession(filePath);
    } catch (err) {
      console.error("File dialog failed:", err);
    }
  }

  /** Check if there's an existing session for the opened document */
  async _checkExistingSession(filePath) {
    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) return;

      const session = await invoke("check_document_session", { path: filePath });

      if (session && session.complete_job) {
        // Import SessionDialog dynamically to avoid circular deps
        const { SessionDialog } = await import("./session-dialog.js");
        const dialog = new SessionDialog({
          onRestore: (jobId, pageCount) => {
            this._activateJob(jobId, pageCount);
          },
          onGenerateNew: () => {
            // User chose to generate new — settings panel is already showing
          },
        });
        dialog.show(session.complete_job);
      } else if (session && session.partial_job) {
        this.showResumeBanner(session.partial_job);
      }
    } catch (err) {
      console.warn("check_document_session failed:", err);
    }
  }

  /** Check if tokens_per_page < 4× phrase_length and show warning */
  _checkTokensWarning() {
    const tokensPerPage = Number(this._els.tokensPerPage.value);
    const phraseLength = Number(this._els.phraseLength.value);

    if (tokensPerPage < 4 * phraseLength) {
      this._els.warningTokensPerPage.textContent =
        "⚠ Tokens per Page is less than 4× Phrase Length. Sub-cells may be sparsely populated.";
      this._els.warningTokensPerPage.hidden = false;
    } else {
      this._els.warningTokensPerPage.hidden = true;
    }
  }

  /** Validate chapter break regex syntax */
  _validateChapterBreak() {
    const pattern = this._els.chapterBreak.value.trim();
    if (pattern === "") {
      this._els.errorChapterBreak.hidden = true;
      this._els.btnAnalyze.disabled = false;
      return true;
    }

    try {
      new RegExp(pattern);
      this._els.errorChapterBreak.hidden = true;
      this._els.btnAnalyze.disabled = false;
      return true;
    } catch (e) {
      this._els.errorChapterBreak.textContent = `Invalid regex: ${e.message}`;
      this._els.errorChapterBreak.hidden = false;
      this._els.btnAnalyze.disabled = true;
      return false;
    }
  }

  /** Schedule a debounced estimate update (100ms) */
  _scheduleEstimateUpdate() {
    if (this._estimateTimer !== null) {
      clearTimeout(this._estimateTimer);
    }
    this._estimateTimer = setTimeout(() => {
      this._estimateTimer = null;
      this._updateEstimate();
      void this._updateRfPassEstimates();
    }, 100);
  }

  /** Call estimate_analysis via Tauri IPC and update display */
  async _updateEstimate() {
    if (!this.filePath) {
      this._lastEstimate = null;
      this._els.estimateWindowCount.textContent = "—";
      this._els.estimateTime.textContent = "—";
      this._els.estimateNudge.hidden = true;
      return;
    }

    const windowSize = Number(this._els.phraseLength.value);
    const stride = Number(this._els.stride.value);
    const tokensPerPage = this.isPdf ? null : Number(this._els.tokensPerPage.value);

    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) {
        // Fallback: no Tauri runtime available (dev/testing)
        this._els.estimateWindowCount.textContent = "estimate unavailable";
        this._els.estimateTime.textContent = "estimate unavailable";
        return;
      }

      /** @type {AnalysisEstimate} */
      const estimate = await invoke("estimate_analysis", {
        path: this.filePath,
        windowSize: windowSize,
        stride: stride,
        tokensPerPage: tokensPerPage,
      });

      this._lastEstimate = estimate;
      this._els.estimateWindowCount.textContent = estimate.window_count.toLocaleString();

      if (estimate.benchmark_windows_per_sec > 0) {
        const etaSeconds = estimate.eta_seconds;
        this._els.estimateTime.textContent = this._formatTime(etaSeconds);

        // Show nudge if > 30 minutes
        if (etaSeconds > 1800) {
          this._els.estimateNudge.hidden = false;
        } else {
          this._els.estimateNudge.hidden = true;
        }
      } else {
        this._els.estimateTime.textContent = "estimate unavailable";
        this._els.estimateNudge.hidden = true;
      }
    } catch (err) {
      console.warn("estimate_analysis failed:", err);
      this._els.estimateWindowCount.textContent = "estimate unavailable";
      this._els.estimateTime.textContent = "estimate unavailable";
      this._els.estimateNudge.hidden = true;
    }
  }

  /**
   * Format seconds into a human-readable time string
   * @param {number} seconds
   * @returns {string}
   */
  _formatTime(seconds) {
    if (seconds < 60) {
      return `${Math.round(seconds)}s`;
    } else if (seconds < 3600) {
      const mins = Math.floor(seconds / 60);
      const secs = Math.round(seconds % 60);
      return `${mins}m ${secs}s`;
    } else {
      const hours = Math.floor(seconds / 3600);
      const mins = Math.round((seconds % 3600) / 60);
      return `${hours}h ${mins}m`;
    }
  }

  /** Start analysis by calling analyze_document via Tauri IPC */
  async _startAnalysis() {
    if (!this._validateChapterBreak()) return;
    if (!this.filePath) {
      console.warn("No file path set for analysis");
      this._showAnalysisError("No document selected. Click Open Document first.");
      return;
    }

    this._lastAnalysisPassMode = "single";
    const settings = this.getSettings();
    this._savedSettings = { ...settings };

    console.info(
      `Analyze clicked: path=${this.filePath} settings=${JSON.stringify(settings)}`,
    );

    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      console.warn("Tauri runtime not available");
      this._showAnalysisError("Tauri runtime not available (running in a browser?)");
      return;
    }

    // Transition to progress view. Wrap in try/catch so a render error doesn't blank the panel.
    let pageCount = this._lastEstimate?.page_count ?? 0;
    if (!pageCount) {
      try {
        const estimate = await invoke("estimate_analysis", {
          path: this.filePath,
          windowSize: settings.phraseLength,
          stride: settings.stride,
          tokensPerPage: this.isPdf ? null : settings.tokensPerPage,
        });
        this._lastEstimate = estimate;
        pageCount = estimate.page_count;
      } catch (estErr) {
        console.warn("estimate_analysis before analyze failed:", estErr);
      }
    }

    if (pageCount > 0 && window.gridRenderer) {
      window.gridRenderer.initGrid(pageCount);
    }

    try {
      this._showProgressView("");
    } catch (renderErr) {
      console.error("Failed to render progress view:", renderErr);
      this._restoreSettingsView().catch(() => {});
      this._showAnalysisError(renderErr);
      return;
    }

    try {
      // Clear the active job filter while the backend is streaming `page-ready` events
      // for a new analysis run (we don't know the new job_id yet).
      // Otherwise, a previous job_id can cause the grid to ignore the new run's events.
      window.currentJobId = "";

      const result = await invoke("analyze_document", {
        path: this.filePath,
        windowSize: settings.phraseLength,
        stride: settings.stride,
        tokensPerPage: this.isPdf ? null : settings.tokensPerPage,
        chapterBreakRegex: settings.chapterBreak || null,
        minRepetitions: settings.minRepetitions,
        minSamples: settings.minSamples,
        enableHdbscan: settings.enableHdbscan,
        linkSubphrases: settings.linkSubphrases,
      });

      console.info(`analyze_document returned: ${JSON.stringify(result)}`);

      if (result?.job_id) {
        if (this._progressView) {
          this._progressView.setJobId(result.job_id);
        }
        await this._onAnalysisComplete(result);
      }
    } catch (err) {
      console.error("analyze_document failed:", err);
      await this._restoreSettingsView();
      this._showAnalysisError(err);
    }
  }

  /** Analyze pasted plain text (Romance Factory output) and apply visualization JSON. */
  async _startTextAnalysis() {
    if (!this._validateChapterBreak()) return;

    const text = this._els.pasteTextArea?.value?.trim();
    if (!text) {
      this._showAnalysisError("Paste manuscript text before analyzing.");
      return;
    }

    this._lastAnalysisPassMode = "single";
    const settings = this.getSettings();
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      this._showAnalysisError("Tauri runtime not available (running in a browser?)");
      return;
    }

    this._savedSettings = { ...settings };
    this._els.btnAnalyzeText.disabled = true;

    try {
      this._showProgressView("");
    } catch (renderErr) {
      console.error("Failed to render progress view:", renderErr);
      this._els.btnAnalyzeText.disabled = false;
      return;
    }

    try {
      window.currentJobId = "";
      const payload = await invoke("analyze_text", {
        text,
        label: "rf_paste",
        windowSize: settings.phraseLength,
        stride: settings.stride,
        tokensPerPage: settings.tokensPerPage,
        chapterBreakRegex: settings.chapterBreak || null,
        minRepetitions: settings.minRepetitions,
        minSamples: settings.minSamples,
        enableHdbscan: settings.enableHdbscan,
        linkSubphrases: settings.linkSubphrases,
      });

      if (payload?.job_id) {
        if (this._progressView) {
          this._progressView.setJobId(payload.job_id);
        }
        this.filePath = payload.job_id;
        this.isPdf = false;
        this._els.fileName.textContent = "Pasted text";
        await applyVisualizationPayload(payload);
      }
    } catch (err) {
      console.error("analyze_text failed:", err);
      this._showAnalysisError(err);
    } finally {
      this._els.btnAnalyzeText.disabled = false;
      await this._restoreSettingsView();
    }
  }

  /**
   * Wire up the grid and display panels after a job finishes or is restored.
   * Re-streams page-ready events from storage so the grid fills even if events
   * were missed during the blocking analyze_document call.
   * @param {string} jobId
   * @param {number} pageCount
   */
  async _activateJob(jobId, pageCount) {
    await activateJob(jobId, pageCount);
  }

  /**
   * @param {{ job_id: string, page_count: number, window_count: number }} result
   */
  async _onAnalysisComplete(result) {
    await this._activateJob(result.job_id, result.page_count);
    if (window.resultsPanel && this.filePath) {
      const catalog = await window.__TAURI__?.core?.invoke?.("list_document_results", {
        path: this.filePath,
      });
      const entry = catalog?.results?.find((item) => item.job_id === result.job_id);
      if (entry) {
        await window.__TAURI__.core.invoke("set_active_document_result", {
          path: this.filePath,
          resultId: entry.result_id,
        });
      }
      await window.resultsPanel.refresh(entry?.result_id);
    }
    await this._restoreSettingsView();
  }

  /**
   * Transition to the progress view, locking all controls
   * @param {string} jobId
   */
  _showProgressView(jobId) {
    this._progressView = new ProgressView(this.container, {
      jobId: jobId,
      onCancel: async () => {
        await this._restoreSettingsView();
      },
      onResume: (resumeJobId) => {
        this._handleResume(resumeJobId);
      },
    });
  }

  /** Restore the settings panel after cancellation or error */
  async _restoreSettingsView() {
    if (this._progressView) {
      await this._progressView.destroy(false);
      this._progressView = null;
    }

    this._buildUI();
    this._attachListeners();

    if (this.filePath) {
      const fileName = this.filePath.split(/[/\\]/).pop() || this.filePath;
      this._els.fileName.textContent = fileName;
      this._els.fileName.title = this.filePath;
    }

    if (this.rfStoryPath) {
      void this._restoreRfSection();
    }

    if (this.rfChapterPreset && this._els.rfPresetSelect) {
      this._els.rfPresetSelect.value = this.rfChapterPreset;
    }

    // Restore saved settings if available
    if (this._savedSettings) {
      this._els.tokensPerPage.value = this._savedSettings.tokensPerPage;
      this._els.inputTokensPerPage.value = this._savedSettings.tokensPerPage;
      this._els.phraseLength.value = this._savedSettings.phraseLength;
      this._els.inputPhraseLength.value = this._savedSettings.phraseLength;
      this._els.stride.value = this._savedSettings.stride;
      this._els.inputStride.value = this._savedSettings.stride;
      this._els.minRepetitions.value = this._savedSettings.minRepetitions;
      this._els.inputMinRepetitions.value = this._savedSettings.minRepetitions;
      this._els.minSamples.value = this._savedSettings.minSamples;
      this._els.inputMinSamples.value = this._savedSettings.minSamples;
      this._els.enableHdbscan.checked = this._savedSettings.enableHdbscan;
      this._els.linkSubphrases.checked = this._savedSettings.linkSubphrases;
      this._els.chapterBreak.value = this._savedSettings.chapterBreak;
      this._updateHdbscanDependentControls();
    }

    this._mountSettingsYamlExport();
    this._updateEstimate();
  }

  /** Show an analysis error above the settings controls */
  _showAnalysisError(err) {
    const message = this._formatInvokeError(err);
    let banner = this.container.querySelector(".import-analysis-error");
    if (!banner) {
      banner = document.createElement("div");
      banner.className = "import-analysis-error";
      banner.setAttribute("role", "alert");
      const controls = this.container.querySelector(".import-settings-controls");
      if (controls) {
        controls.parentNode.insertBefore(banner, controls);
      } else {
        this.container.prepend(banner);
      }
    }
    banner.textContent = `Analysis failed: ${message}`;
  }

  /**
   * @param {unknown} err
   * @returns {string}
   */
  _formatInvokeError(err) {
    if (typeof err === "string") return err;
    if (err && typeof err === "object") {
      const o = /** @type {Record<string, unknown>} */ (err);
      if (typeof o.message === "string") return o.message;
      const detail = o.detail;
      if (detail && typeof detail === "object" && typeof detail.message === "string") {
        return detail.message;
      }
      try {
        return JSON.stringify(err);
      } catch {
        return String(err);
      }
    }
    return String(err);
  }

  /**
   * Handle resume of a partial job
   * @param {string} jobId
   */
  async _handleResume(jobId) {
    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) return;

      // Hide resume banner and show progress
      if (this._progressView) {
        this._progressView.hideResumeBanner();
      }

      await invoke("resume_analysis", { jobId: jobId });
    } catch (err) {
      console.error("resume_analysis failed:", err);
      await this._restoreSettingsView();
      this._showAnalysisError(err);
    }
  }

  /**
   * Show the resume banner for a partial job
   * @param {Object} partialJob - PartialJobInfo from check_document_session
   */
  showResumeBanner(partialJob) {
    // Transition to progress view in paused state
    this._showProgressView(partialJob.job_id);
    this._progressView.showResumeBanner(partialJob);
  }

  /** Restore RF story picker state after progress view teardown. */
  async _restoreRfSection() {
    if (!this.rfStoryPath || !this._els.rfMeta) return;
    this._els.rfMeta.hidden = false;
    const storyName = this.rfStoryPath.split(/[/\\]/).pop() || this.rfStoryPath;
    this._els.rfStoryName.textContent = storyName;
    this._els.rfStoryName.title = this.rfStoryPath;

    const select = this._els.rfChapterSelect;
    select.innerHTML = '<option value="">Select chapter…</option>';
    for (const ch of this._rfChapters) {
      const opt = document.createElement("option");
      opt.value = String(ch);
      opt.textContent = `Chapter ${ch}`;
      select.appendChild(opt);
    }
    if (this.rfChapter) {
      select.value = String(this.rfChapter);
      this._els.btnAnalyzeRfChapter.disabled = false;
      await this._onRfChapterSelected();
    } else {
      this._renderRfPassEstimates(null);
    }
  }

  /**
   * Get current settings values
   * @returns {Object}
   */
  getSettings() {
    return {
      tokensPerPage: Number(this._els.tokensPerPage.value),
      phraseLength: Number(this._els.phraseLength.value),
      stride: Number(this._els.stride.value),
      minRepetitions: Number(this._els.minRepetitions.value),
      minSamples: Number(this._els.minSamples.value),
      enableHdbscan: this._els.enableHdbscan.checked,
      linkSubphrases: this._els.linkSubphrases.checked,
      chapterBreak: this._els.chapterBreak.value.trim(),
      rfChapterPreset: this.rfChapterPreset,
    };
  }

  /** Settings payload for `generate:similarity_map:` YAML export (THE-326). */
  getExportSettings() {
    if (!this._els?.phraseLength) return null;
    return buildExportSettingsFromPanel(
      this.getSettings(),
      inferAnalysisPassMode(this),
    );
  }

  /** Gray out Min Samples when HDBSCAN is bypassed */
  _updateHdbscanDependentControls() {
    const enabled = this._els.enableHdbscan.checked;
    this._els.minSamples.disabled = !enabled;
    this._els.inputMinSamples.disabled = !enabled;
    const note = this.container.querySelector("#note-min-samples");
    if (note) {
      note.hidden = enabled;
    }
  }

  /**
   * Set the file path and update estimates
   * @param {string} path
   * @param {boolean} isPdf
   */
  setFile(path, isPdf = false) {
    this.filePath = path;
    this.isPdf = isPdf;

    // Disable/enable tokens per page for PDF
    this._els.tokensPerPage.disabled = isPdf;
    this._els.inputTokensPerPage.disabled = isPdf;
    const settingGroup = this.container.querySelector("#setting-tokens-per-page");
    const existingNote = settingGroup.querySelector(".setting-note");
    if (isPdf && !existingNote) {
      const note = document.createElement("span");
      note.className = "setting-note";
      note.textContent = "Disabled for PDF imports";
      settingGroup.appendChild(note);
    } else if (!isPdf && existingNote) {
      existingNote.remove();
    }

    this._updateEstimate();
  }

  /** Reset stride manual override flag (e.g., on new file load) */
  resetStrideOverride() {
    this._strideManuallySet = false;
  }

  /** Destroy the panel and clean up timers */
  destroy() {
    if (this._estimateTimer !== null) {
      clearTimeout(this._estimateTimer);
      this._estimateTimer = null;
    }
    if (this._progressView) {
      this._progressView.destroy();
      this._progressView = null;
    }
    this.container.innerHTML = "";
  }
}
