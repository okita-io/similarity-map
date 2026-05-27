// Model Download UI — manages embedding model availability and download progress
// Tauri 2 IPC via window.__TAURI__.core.invoke
// Events via window.__TAURI__.event.listen

/**
 * ModelDownloadUI handles:
 * - Calling ensure_embedding_model on startup
 * - Listening for model-download-progress events
 * - Showing a progress bar with percentage and bytes
 * - Displaying error with Retry button on failure
 * - Blocking the Analyze button until the model is available
 *
 * Validates: Requirements 27.2, 27.4, 5.5
 */
export class ModelDownloadUI {
  /**
   * @param {HTMLElement} container - Element to render the download banner into
   * @param {Object} [options]
   * @param {function} [options.onModelReady] - Callback when model becomes available
   * @param {function} [options.onModelUnavailable] - Callback when model is not yet available
   */
  constructor(container, options = {}) {
    this.container = container;
    this._onModelReady = options.onModelReady || null;
    this._onModelUnavailable = options.onModelUnavailable || null;

    /** @type {boolean} */
    this._modelReady = false;

    /** @type {function|null} - Unlisten handle for progress events */
    this._unlistenProgress = null;

    /** @type {function|null} - Unlisten handle for model-ready events */
    this._unlistenReady = null;

    /** @type {HTMLElement|null} */
    this._bannerEl = null;

    this._init();
  }

  /** Whether the model is available for use */
  get isModelReady() {
    return this._modelReady;
  }

  /** Initialize: check model status and set up event listeners */
  async _init() {
    await this._listenForEvents();
    await this._checkModel();
  }

  /** Set up Tauri event listeners for download progress and model-ready */
  async _listenForEvents() {
    const listen = window.__TAURI__?.event?.listen;
    if (!listen) return;

    this._unlistenProgress = await listen(
      "similarity-map:model-download-progress",
      (event) => {
        this._handleProgress(event.payload);
      }
    );

    this._unlistenReady = await listen(
      "similarity-map:model-ready",
      (event) => {
        this._handleModelReady(event.payload);
      }
    );
  }

  /** Call ensure_embedding_model to check/trigger download */
  async _checkModel() {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      // No Tauri runtime (dev/testing) — assume model is ready
      this._setModelReady();
      return;
    }

    try {
      const status = await invoke("ensure_embedding_model");

      if (status && status.present) {
        this._setModelReady();
      } else {
        // Model not present — download should be in progress
        // Show the download banner
        this._showDownloadBanner(0, 0, 0);
        if (this._onModelUnavailable) {
          this._onModelUnavailable();
        }
      }
    } catch (err) {
      console.error("ensure_embedding_model failed:", err);
      this._showErrorBanner(String(err));
      if (this._onModelUnavailable) {
        this._onModelUnavailable();
      }
    }
  }

  /**
   * Handle download progress event
   * @param {{ pct: number, bytes_received: number, total_bytes: number }} payload
   */
  _handleProgress(payload) {
    const { pct, bytes_received, total_bytes } = payload;
    this._showDownloadBanner(pct, bytes_received, total_bytes);
  }

  /**
   * Handle model-ready event
   * @param {{ path: string }} payload
   */
  _handleModelReady(payload) {
    this._setModelReady();
  }

  /** Mark model as ready and hide the banner */
  _setModelReady() {
    this._modelReady = true;
    this._hideBanner();
    if (this._onModelReady) {
      this._onModelReady();
    }
  }

  /**
   * Show the download progress banner
   * @param {number} pct - Percentage (0-100)
   * @param {number} bytesReceived
   * @param {number} totalBytes
   */
  _showDownloadBanner(pct, bytesReceived, totalBytes) {
    if (!this._bannerEl) {
      this._bannerEl = document.createElement("div");
      this._bannerEl.className = "model-download-banner";
      this._bannerEl.setAttribute("role", "status");
      this._bannerEl.setAttribute("aria-live", "polite");
      this.container.prepend(this._bannerEl);
    }

    const pctDisplay = Math.round(pct);
    const bytesDisplay = this._formatBytes(bytesReceived);
    const totalDisplay = totalBytes > 0 ? this._formatBytes(totalBytes) : "~22 MB";

    this._bannerEl.innerHTML = `
      <div class="model-download-content">
        <div class="model-download-header">
          <span class="model-download-icon" aria-hidden="true">⬇</span>
          <span class="model-download-title">Downloading embedding model…</span>
        </div>
        <div class="model-download-progress-row">
          <div class="model-download-bar-track" role="progressbar" aria-valuenow="${pctDisplay}" aria-valuemin="0" aria-valuemax="100" aria-label="Model download progress">
            <div class="model-download-bar-fill" style="width: ${pctDisplay}%"></div>
          </div>
          <span class="model-download-pct">${pctDisplay}%</span>
        </div>
        <div class="model-download-bytes">${bytesDisplay} / ${totalDisplay}</div>
      </div>
    `;

    this._bannerEl.classList.remove("model-download-error");
  }

  /**
   * Show an error banner with a Retry button
   * @param {string} errorMessage
   */
  _showErrorBanner(errorMessage) {
    if (!this._bannerEl) {
      this._bannerEl = document.createElement("div");
      this._bannerEl.className = "model-download-banner";
      this._bannerEl.setAttribute("role", "alert");
      this.container.prepend(this._bannerEl);
    }

    this._bannerEl.classList.add("model-download-error");

    this._bannerEl.innerHTML = `
      <div class="model-download-content">
        <div class="model-download-header">
          <span class="model-download-icon model-download-icon-error" aria-hidden="true">⚠</span>
          <span class="model-download-title">Model download failed</span>
        </div>
        <div class="model-download-error-msg">${this._escapeHtml(errorMessage)}</div>
        <button class="btn-model-retry" type="button">Retry</button>
      </div>
    `;

    const retryBtn = this._bannerEl.querySelector(".btn-model-retry");
    retryBtn.addEventListener("click", () => {
      this._retry();
    });
  }

  /** Hide and remove the banner */
  _hideBanner() {
    if (this._bannerEl) {
      this._bannerEl.remove();
      this._bannerEl = null;
    }
  }

  /** Retry model download */
  async _retry() {
    // Show a loading state
    this._showDownloadBanner(0, 0, 0);
    await this._checkModel();
  }

  /**
   * Format bytes into a human-readable string
   * @param {number} bytes
   * @returns {string}
   */
  _formatBytes(bytes) {
    if (bytes === 0) return "0 B";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  /**
   * Escape HTML to prevent XSS in error messages
   * @param {string} str
   * @returns {string}
   */
  _escapeHtml(str) {
    const div = document.createElement("div");
    div.textContent = str;
    return div.innerHTML;
  }

  /** Clean up event listeners */
  destroy() {
    if (this._unlistenProgress) {
      this._unlistenProgress();
      this._unlistenProgress = null;
    }
    if (this._unlistenReady) {
      this._unlistenReady();
      this._unlistenReady = null;
    }
    this._hideBanner();
  }
}
