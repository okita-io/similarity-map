// Import Settings Panel — manages analysis parameter controls and live estimates
// Tauri 2 IPC via window.__TAURI__.core.invoke

import { ProgressView } from "./progress-view.js";

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

    // Track whether user has manually overridden stride
    this._strideManuallySet = false;

    // Debounce timer for estimate updates
    this._estimateTimer = null;

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
      chapterBreak: "^Chapter\\s+\\d+",
    };

    this._buildUI();
    this._attachListeners();
    this._updateEstimate();
  }

  /** Build the panel DOM structure */
  _buildUI() {
    this.container.innerHTML = "";

    this.container.innerHTML = `
      <div class="import-settings-panel">
        <h2 class="import-settings-title">Import Settings</h2>

        <div class="import-settings-controls">
          <div class="setting-group" id="setting-tokens-per-page">
            <label class="setting-label" for="slider-tokens-per-page">
              Tokens per Page: <span class="setting-value" id="value-tokens-per-page">${this._defaults.tokensPerPage}</span>
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
              Phrase Length: <span class="setting-value" id="value-phrase-length">${this._defaults.phraseLength}</span>
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
              Stride: <span class="setting-value" id="value-stride">${this._defaults.stride}</span>
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
              Min Repetitions: <span class="setting-value" id="value-min-repetitions">${this._defaults.minRepetitions}</span>
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
              Min Samples: <span class="setting-value" id="value-min-samples">${this._defaults.minSamples}</span>
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
          <div class="estimate-nudge" id="estimate-nudge" hidden>
            ⏱ Estimated time exceeds 30 minutes. Consider increasing Stride to reduce processing time.
          </div>
        </div>

        <div class="import-settings-actions">
          <button id="btn-analyze" class="btn-analyze" type="button">Analyze</button>
        </div>
      </div>
    `;

    // Cache element references
    this._els = {
      tokensPerPage: this.container.querySelector("#slider-tokens-per-page"),
      phraseLength: this.container.querySelector("#slider-phrase-length"),
      stride: this.container.querySelector("#slider-stride"),
      minRepetitions: this.container.querySelector("#slider-min-repetitions"),
      minSamples: this.container.querySelector("#slider-min-samples"),
      chapterBreak: this.container.querySelector("#input-chapter-break"),
      valueTokensPerPage: this.container.querySelector("#value-tokens-per-page"),
      valuePhraseLength: this.container.querySelector("#value-phrase-length"),
      valueStride: this.container.querySelector("#value-stride"),
      valueMinRepetitions: this.container.querySelector("#value-min-repetitions"),
      valueMinSamples: this.container.querySelector("#value-min-samples"),
      warningTokensPerPage: this.container.querySelector("#warning-tokens-per-page"),
      errorChapterBreak: this.container.querySelector("#error-chapter-break"),
      estimateWindowCount: this.container.querySelector("#estimate-window-count"),
      estimateTime: this.container.querySelector("#estimate-time"),
      estimateNudge: this.container.querySelector("#estimate-nudge"),
      btnAnalyze: this.container.querySelector("#btn-analyze"),
    };
  }

  /** Attach event listeners to all controls */
  _attachListeners() {
    // Tokens per Page slider
    this._els.tokensPerPage.addEventListener("input", () => {
      const val = Number(this._els.tokensPerPage.value);
      this._els.valueTokensPerPage.textContent = val;
      this._checkTokensWarning();
      this._scheduleEstimateUpdate();
    });

    // Phrase Length slider — auto-computes stride
    this._els.phraseLength.addEventListener("input", () => {
      const val = Number(this._els.phraseLength.value);
      this._els.valuePhraseLength.textContent = val;

      // Auto-compute stride unless user manually set it
      if (!this._strideManuallySet) {
        const newStride = Math.max(1, Math.floor(val * 0.25));
        this._els.stride.value = newStride;
        this._els.valueStride.textContent = newStride;
      }

      this._checkTokensWarning();
      this._scheduleEstimateUpdate();
    });

    // Stride slider — mark as manually set
    this._els.stride.addEventListener("input", () => {
      this._strideManuallySet = true;
      const val = Number(this._els.stride.value);
      this._els.valueStride.textContent = val;
      this._scheduleEstimateUpdate();
    });

    // Min Repetitions slider
    this._els.minRepetitions.addEventListener("input", () => {
      const val = Number(this._els.minRepetitions.value);
      this._els.valueMinRepetitions.textContent = val;
      this._scheduleEstimateUpdate();
    });

    // Min Samples slider
    this._els.minSamples.addEventListener("input", () => {
      const val = Number(this._els.minSamples.value);
      this._els.valueMinSamples.textContent = val;
      this._scheduleEstimateUpdate();
    });

    // Chapter Break regex input
    this._els.chapterBreak.addEventListener("input", () => {
      this._validateChapterBreak();
      this._scheduleEstimateUpdate();
    });

    // Analyze button
    this._els.btnAnalyze.addEventListener("click", () => {
      this._startAnalysis();
    });
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
    }, 100);
  }

  /** Call estimate_analysis via Tauri IPC and update display */
  async _updateEstimate() {
    if (!this.filePath) {
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
        window_size: windowSize,
        stride: stride,
        tokens_per_page: tokensPerPage,
      });

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
      return;
    }

    const settings = this.getSettings();

    // Save settings snapshot for restore on cancellation
    this._savedSettings = { ...settings };

    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) {
        console.warn("Tauri runtime not available");
        return;
      }

      // Transition to progress view
      this._showProgressView("");

      const result = await invoke("analyze_document", {
        path: this.filePath,
        window_size: settings.phraseLength,
        stride: settings.stride,
        tokens_per_page: this.isPdf ? null : settings.tokensPerPage,
        chapter_break_regex: settings.chapterBreak || null,
        min_repetitions: settings.minRepetitions,
        min_samples: settings.minSamples,
      });

      // Update progress view with the returned job_id
      if (this._progressView && result && result.job_id) {
        this._progressView.setJobId(result.job_id);
      }
    } catch (err) {
      console.error("analyze_document failed:", err);
      this._restoreSettingsView();
    }
  }

  /**
   * Transition to the progress view, locking all controls
   * @param {string} jobId
   */
  _showProgressView(jobId) {
    this._progressView = new ProgressView(this.container, {
      jobId: jobId,
      onCancel: () => {
        this._restoreSettingsView();
      },
      onResume: (resumeJobId) => {
        this._handleResume(resumeJobId);
      },
    });
  }

  /** Restore the settings panel after cancellation or error */
  _restoreSettingsView() {
    if (this._progressView) {
      this._progressView.destroy();
      this._progressView = null;
    }

    this._buildUI();
    this._attachListeners();

    // Restore saved settings if available
    if (this._savedSettings) {
      this._els.tokensPerPage.value = this._savedSettings.tokensPerPage;
      this._els.valueTokensPerPage.textContent = this._savedSettings.tokensPerPage;
      this._els.phraseLength.value = this._savedSettings.phraseLength;
      this._els.valuePhraseLength.textContent = this._savedSettings.phraseLength;
      this._els.stride.value = this._savedSettings.stride;
      this._els.valueStride.textContent = this._savedSettings.stride;
      this._els.minRepetitions.value = this._savedSettings.minRepetitions;
      this._els.valueMinRepetitions.textContent = this._savedSettings.minRepetitions;
      this._els.minSamples.value = this._savedSettings.minSamples;
      this._els.valueMinSamples.textContent = this._savedSettings.minSamples;
      this._els.chapterBreak.value = this._savedSettings.chapterBreak;
    }

    this._updateEstimate();
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

      await invoke("resume_analysis", { job_id: jobId });
    } catch (err) {
      console.error("resume_analysis failed:", err);
      this._restoreSettingsView();
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
      chapterBreak: this._els.chapterBreak.value.trim(),
    };
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
