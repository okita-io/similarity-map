// Session Restore Dialog — modal shown when a complete session is found on document open
// Tauri 2 IPC via window.__TAURI__.core.invoke

/**
 * @typedef {Object} CompleteJobInfo
 * @property {string} job_id
 * @property {string} created_at
 * @property {number} page_count
 * @property {number} window_size
 * @property {number} stride
 * @property {number|null} tokens_per_page
 * @property {string} pagination_mode
 */

export class SessionDialog {
  /**
   * @param {Object} options
   * @param {function} [options.onRestore] - Called after restore_session succeeds, receives job_id
   * @param {function} [options.onGenerateNew] - Called after user chooses Generate New Map
   */
  constructor(options = {}) {
    this._onRestore = options.onRestore || (() => {});
    this._onGenerateNew = options.onGenerateNew || (() => {});
    this._overlay = null;
    this._dialog = null;
    this._boundKeyHandler = this._handleKeyDown.bind(this);
  }

  /**
   * Show the session restore dialog for a complete job.
   * @param {CompleteJobInfo} job - The complete job info from check_document_session
   * @returns {void}
   */
  show(job) {
    if (this._overlay) {
      this.dismiss();
    }

    this._job = job;
    this._buildDOM(job);
    this._attachListeners();

    // Add to document
    document.body.appendChild(this._overlay);

    // Focus the restore button for keyboard accessibility
    const restoreBtn = this._dialog.querySelector(".session-dialog-btn-restore");
    if (restoreBtn) {
      restoreBtn.focus();
    }
  }

  /**
   * Build the dialog DOM structure.
   * @param {CompleteJobInfo} job
   */
  _buildDOM(job) {
    // Format the creation date
    const createdDate = this._formatDate(job.created_at);

    // Create overlay (backdrop)
    this._overlay = document.createElement("div");
    this._overlay.className = "session-dialog-overlay";
    this._overlay.setAttribute("role", "dialog");
    this._overlay.setAttribute("aria-modal", "true");
    this._overlay.setAttribute("aria-labelledby", "session-dialog-title");

    // Create dialog container
    this._dialog = document.createElement("div");
    this._dialog.className = "session-dialog";

    this._dialog.innerHTML = `
      <h2 class="session-dialog-title" id="session-dialog-title">Previous Session Found</h2>
      <p class="session-dialog-description">
        A completed analysis session exists for this document.
      </p>
      <div class="session-dialog-details">
        <div class="session-dialog-detail-row">
          <span class="session-dialog-detail-label">Created:</span>
          <span class="session-dialog-detail-value">${createdDate}</span>
        </div>
        <div class="session-dialog-detail-row">
          <span class="session-dialog-detail-label">Pages:</span>
          <span class="session-dialog-detail-value">${job.page_count}</span>
        </div>
        <div class="session-dialog-detail-row">
          <span class="session-dialog-detail-label">Phrase Length:</span>
          <span class="session-dialog-detail-value">${job.window_size} tokens</span>
        </div>
        <div class="session-dialog-detail-row">
          <span class="session-dialog-detail-label">Stride:</span>
          <span class="session-dialog-detail-value">${job.stride} tokens</span>
        </div>
      </div>
      <div class="session-dialog-actions">
        <button class="session-dialog-btn session-dialog-btn-restore" type="button">
          Restore Session
        </button>
        <button class="session-dialog-btn session-dialog-btn-new" type="button">
          Generate New Map
        </button>
      </div>
    `;

    this._overlay.appendChild(this._dialog);
  }

  /** Attach event listeners for dialog interactions */
  _attachListeners() {
    // Restore Session button
    const restoreBtn = this._dialog.querySelector(".session-dialog-btn-restore");
    restoreBtn.addEventListener("click", () => {
      this._handleRestore();
    });

    // Generate New Map button
    const newBtn = this._dialog.querySelector(".session-dialog-btn-new");
    newBtn.addEventListener("click", () => {
      this._handleGenerateNew();
    });

    // Click outside dialog (on overlay backdrop) = Generate New Map
    this._overlay.addEventListener("click", (e) => {
      if (e.target === this._overlay) {
        this._handleGenerateNew();
      }
    });

    // Escape key = Generate New Map
    document.addEventListener("keydown", this._boundKeyHandler);
  }

  /**
   * Handle keydown events (Escape dismisses as Generate New Map)
   * @param {KeyboardEvent} e
   */
  _handleKeyDown(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      this._handleGenerateNew();
    }
  }

  /** Handle Restore Session action */
  async _handleRestore() {
    const job = this._job;
    this.dismiss();

    try {
      await this._onRestore(job.job_id, job.page_count);
    } catch (err) {
      console.error("restore session failed:", err);
      // Fall back to generate new on failure
      this._onGenerateNew();
    }
  }

  /** Handle Generate New Map action */
  async _handleGenerateNew() {
    const job = this._job;
    this.dismiss();

    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (invoke && job && job.job_id) {
        await invoke("discard_job", { jobId: job.job_id });
      }
    } catch (err) {
      console.warn("discard_job failed:", err);
    }

    this._onGenerateNew();
  }

  /** Remove the dialog from the DOM and clean up listeners */
  dismiss() {
    document.removeEventListener("keydown", this._boundKeyHandler);

    if (this._overlay && this._overlay.parentNode) {
      this._overlay.parentNode.removeChild(this._overlay);
    }

    this._overlay = null;
    this._dialog = null;
    this._job = null;
  }

  /**
   * Format an ISO date string into a human-readable format.
   * @param {string} isoString
   * @returns {string}
   */
  _formatDate(isoString) {
    try {
      const date = new Date(isoString);
      if (isNaN(date.getTime())) {
        return isoString;
      }
      return date.toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return isoString;
    }
  }
}
