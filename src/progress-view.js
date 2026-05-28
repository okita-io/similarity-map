// Progress View — displays multi-stage analysis progress with cancellation support
// Tauri 2 IPC via window.__TAURI__.core.invoke
// Listens for similarity-map:progress events

/**
 * @typedef {Object} ProgressPayload
 * @property {string} job_id
 * @property {string} stage
 * @property {number} pct - 0.0–1.0
 * @property {number} windows_done
 * @property {number} windows_total
 * @property {number} eta_seconds
 */

/**
 * @typedef {Object} PartialJobInfo
 * @property {string} job_id
 * @property {number} windows_committed
 * @property {number} windows_total
 * @property {number} pct
 * @property {string} cancelled_at
 * @property {number} window_size
 * @property {number} stride
 * @property {number|null} tokens_per_page
 */

// Stage ids must match pipeline.rs Stage::as_str() values.
const STAGES = [
  { id: "import", label: "Importing" },
  { id: "windowing", label: "Windowing" },
  { id: "embedding", label: "Embedding" },
  { id: "clustering", label: "Clustering" },
  { id: "stabilization", label: "Stabilizing" },
  { id: "centroid", label: "Centroids" },
  { id: "subcell", label: "Sub-cells" },
  { id: "rasterization", label: "Rasterizing" },
];

export class ProgressView {
  /**
   * @param {HTMLElement} container - The panel container element
   * @param {Object} options
   * @param {string} options.jobId - The active job ID
   * @param {Function} options.onCancel - Callback after cancellation completes
   * @param {Function} [options.onResume] - Callback when user clicks Resume
   */
  constructor(container, options = {}) {
    this.container = container;
    this.jobId = options.jobId || "";
    this._onCancel = options.onCancel || (() => {});
    this._onResume = options.onResume || (() => {});

    this._currentStage = "";
    this._pct = 0;
    this._etaSeconds = 0;
    this._windowsDone = 0;
    this._windowsTotal = 0;
    this._isCancelling = false;
    this._unlisten = null;

    this._buildUI();
    this._attachListeners();
    this._updateStageDisplay();
    this._startListening();
  }

  /** Build the progress view DOM */
  _buildUI() {
    this.container.innerHTML = "";

    const stageItems = STAGES.map(
      (stage) => `
      <li class="progress-stage" id="stage-${stage.id}" data-stage="${stage.id}">
        <span class="progress-stage-icon" aria-hidden="true">○</span>
        <span class="progress-stage-label">${stage.label}</span>
        <div class="progress-stage-detail" hidden>
          <div class="progress-bar-track">
            <div class="progress-bar-fill" style="width: 0%"></div>
          </div>
          <span class="progress-stage-pct">0%</span>
          <span class="progress-stage-eta"></span>
        </div>
      </li>
    `
    ).join("");

    this.container.innerHTML = `
      <div class="progress-view">
        <h2 class="progress-view-title">Analyzing…</h2>
        <ul class="progress-stage-list" aria-label="Analysis stages">
          ${stageItems}
        </ul>
        <div class="progress-view-actions">
          <button class="btn-cancel" id="btn-cancel-analysis" type="button">Cancel import</button>
        </div>
        <div class="progress-resume-banner" id="resume-banner" hidden>
          <span class="resume-banner-text"></span>
          <button class="btn-resume" id="btn-resume" type="button">Resume</button>
        </div>
      </div>
    `;

    this._els = {
      title: this.container.querySelector(".progress-view-title"),
      stageList: this.container.querySelector(".progress-stage-list"),
      btnCancel: this.container.querySelector("#btn-cancel-analysis"),
      resumeBanner: this.container.querySelector("#resume-banner"),
      resumeBannerText: this.container.querySelector(".resume-banner-text"),
      btnResume: this.container.querySelector("#btn-resume"),
    };
  }

  /** Attach button event listeners */
  _attachListeners() {
    this._els.btnCancel.addEventListener("click", () => {
      this._handleCancel();
    });

    this._els.btnResume.addEventListener("click", () => {
      this._onResume(this.jobId);
    });
  }

  /** Start listening for progress events from the backend */
  async _startListening() {
    try {
      const listen = window.__TAURI__?.event?.listen;
      if (!listen) return;

      this._unlisten = await listen("similarity-map:progress", (event) => {
        this._handleProgress(event.payload);
      });
    } catch (err) {
      console.warn("Failed to listen for progress events:", err);
    }
  }

  /**
   * Handle a progress event payload
   * @param {ProgressPayload} payload
   */
  _handleProgress(payload) {
    // Ignore only when we already know our job and the event is for a different one.
    if (this.jobId && payload.job_id && payload.job_id !== this.jobId) return;

    this._currentStage = payload.stage;
    this._pct = payload.pct;
    this._etaSeconds = payload.eta_seconds;
    this._windowsDone = payload.windows_done;
    this._windowsTotal = payload.windows_total;

    this._updateStageDisplay();
  }

  /** Update the stage checklist UI based on current progress */
  _updateStageDisplay() {
    let currentIndex = STAGES.findIndex((s) => s.id === this._currentStage);
    // Before the first progress event, show the checklist with Importing as active.
    if (currentIndex < 0) {
      currentIndex = 0;
    }

    for (let i = 0; i < STAGES.length; i++) {
      const stage = STAGES[i];
      const el = this.container.querySelector(`#stage-${stage.id}`);
      if (!el) continue;

      const icon = el.querySelector(".progress-stage-icon");
      const detail = el.querySelector(".progress-stage-detail");

      // Remove all state classes
      el.classList.remove("stage-done", "stage-active", "stage-pending");

      if (i < currentIndex) {
        // Completed stage
        el.classList.add("stage-done");
        icon.textContent = "✓";
        detail.hidden = true;
      } else if (i === currentIndex) {
        // Active stage
        el.classList.add("stage-active");
        icon.textContent = "●";
        detail.hidden = false;

        const fill = detail.querySelector(".progress-bar-fill");
        const pctLabel = detail.querySelector(".progress-stage-pct");
        const etaLabel = detail.querySelector(".progress-stage-eta");

        const pctValue = Math.round(this._pct * 100);
        fill.style.width = `${pctValue}%`;
        pctLabel.textContent = `${pctValue}%`;

        if (this._etaSeconds > 0) {
          etaLabel.textContent = `ETA: ${this._formatEta(this._etaSeconds)}`;
        } else {
          etaLabel.textContent = "";
        }
      } else {
        // Pending stage
        el.classList.add("stage-pending");
        icon.textContent = "○";
        detail.hidden = true;
      }
    }
  }

  /**
   * Format ETA seconds into a readable string
   * @param {number} seconds
   * @returns {string}
   */
  _formatEta(seconds) {
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

  /** Handle cancel button click */
  async _handleCancel() {
    if (this._isCancelling) return;
    this._isCancelling = true;

    this._els.btnCancel.disabled = true;
    this._els.btnCancel.textContent = "Cancelling…";

    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (invoke) {
        await invoke("cancel_analysis", { jobId: this.jobId });
      }
    } catch (err) {
      console.error("cancel_analysis failed:", err);
    }

    this._isCancelling = false;
    this._onCancel();
  }

  /**
   * Show the resume banner for a partial job
   * @param {PartialJobInfo} partialJob
   */
  showResumeBanner(partialJob) {
    const pct = Math.round((partialJob.windows_committed / partialJob.windows_total) * 100);
    const storageMB = ((partialJob.windows_committed * 384 * 4) / (1024 * 1024)).toFixed(1);

    this._els.resumeBannerText.textContent =
      `Partial analysis: ${pct}% complete (${storageMB} MB stored)`;
    this._els.resumeBanner.hidden = false;

    // Hide the cancel button and stage list when showing resume banner in idle state
    this._els.btnCancel.hidden = true;
    this._els.title.textContent = "Analysis paused";
  }

  /** Hide the resume banner */
  hideResumeBanner() {
    this._els.resumeBanner.hidden = true;
    this._els.btnCancel.hidden = false;
    this._els.title.textContent = "Analyzing…";
  }

  /**
   * Update the job ID (e.g., after analyze_document returns)
   * @param {string} jobId
   */
  setJobId(jobId) {
    this.jobId = jobId;
  }

  /** Clean up event listeners. Pass clearDom=false when the container will be rebuilt immediately. */
  async destroy(clearDom = true) {
    if (this._unlisten) {
      await this._unlisten();
      this._unlisten = null;
    }
    if (clearDom) {
      this.container.innerHTML = "";
    }
  }
}
