// Display Settings Panel — Tolerance, Gamma, and Cluster Filter controls
// Tolerance: frontend-only mask update (no IPC)
// Gamma: full re-raster via raster_pages IPC (all pages)
// Cluster Filter: targeted re-raster via raster_pages IPC (affected pages only)
// Display state persistence: debounced 2 seconds after any change

/**
 * @typedef {Object} DisplayState
 * @property {string} job_id
 * @property {number} tolerance
 * @property {number} gamma
 * @property {number[]} hidden_clusters
 * @property {number} zoom
 * @property {number} scroll_x
 * @property {number} scroll_y
 * @property {string} saved_at
 */

/**
 * @typedef {Object} ClusterInfo
 * @property {number} cluster_id
 * @property {number} hue
 * @property {number} member_count
 * @property {string} most_central_window_text
 * @property {number[]} pages
 */

export class DisplaySettingsPanel {
  /**
   * @param {HTMLElement} container - The panel container element
   * @param {Object} [options]
   * @param {string} [options.jobId] - Current job ID for IPC calls
   * @param {import('./tolerance.js').ToleranceMask} [options.toleranceMask] - ToleranceMask instance
   * @param {Map<number, HTMLCanvasElement>} [options.canvases] - page → canvas map
   * @param {function} [options.onPagesUpdated] - Callback when pages are re-rastered
   */
  constructor(container, options = {}) {
    this.container = container;
    this.jobId = options.jobId || "";
    this.toleranceMask = options.toleranceMask || null;
    this.canvases = options.canvases || new Map();
    this.onPagesUpdated = options.onPagesUpdated || null;

    // Current display state
    // Match backend default raster threshold for first-run visibility.
    this._tolerance = 0.75;
    this._gamma = 1.5;
    /** @type {Set<number>} */
    this._hiddenClusters = new Set();

    // Cluster registry data
    /** @type {ClusterInfo[]} */
    this._clusters = [];

    // Debounce timer for display state persistence (2 seconds)
    this._persistTimer = null;

    // All pages in the current job (for gamma full re-raster)
    /** @type {number[]} */
    this._allPages = [];

    this._buildUI();
    this._attachListeners();
  }

  /** Build the panel DOM structure */
  _buildUI() {
    this.container.innerHTML = "";

    this.container.innerHTML = `
      <div class="display-settings-panel">
        <h2 class="display-settings-title">Display Settings</h2>

        <div class="display-settings-controls">
          <div class="setting-group">
            <label class="setting-label" for="slider-tolerance">
              Tolerance: <span class="setting-value" id="value-tolerance">${this._tolerance.toFixed(2)}</span>
            </label>
            <input
              type="range"
              id="slider-tolerance"
              class="setting-slider"
              min="0.75"
              max="1.00"
              step="0.01"
              value="${this._tolerance}"
              aria-label="Tolerance"
            />
          </div>

          <div class="setting-group">
            <label class="setting-label" for="slider-gamma">
              Gamma: <span class="setting-value" id="value-gamma">${this._gamma.toFixed(1)}</span>
            </label>
            <input
              type="range"
              id="slider-gamma"
              class="setting-slider"
              min="0.5"
              max="3.0"
              step="0.1"
              value="${this._gamma}"
              aria-label="Gamma"
            />
          </div>

          <div class="setting-group">
            <label class="setting-label">Cluster Filter:</label>
            <div class="cluster-filter-list" id="cluster-filter-list">
              <span class="setting-note">No clusters loaded</span>
            </div>
          </div>
        </div>
      </div>
    `;

    // Cache element references
    this._els = {
      tolerance: this.container.querySelector("#slider-tolerance"),
      gamma: this.container.querySelector("#slider-gamma"),
      valueTolerance: this.container.querySelector("#value-tolerance"),
      valueGamma: this.container.querySelector("#value-gamma"),
      clusterFilterList: this.container.querySelector("#cluster-filter-list"),
    };
  }

  /** Attach event listeners to slider controls */
  _attachListeners() {
    // Tolerance slider — frontend-only mask update, no IPC
    this._els.tolerance.addEventListener("input", () => {
      const val = Number(this._els.tolerance.value);
      this._tolerance = val;
      this._els.valueTolerance.textContent = val.toFixed(2);

      // Update tolerance mask (frontend-only, no IPC)
      if (this.toleranceMask && this.canvases) {
        this.toleranceMask.updateTolerance(val, this.canvases);
      }

      this._schedulePersist();
    });

    // Gamma slider — full re-raster via IPC (all pages)
    this._els.gamma.addEventListener("input", () => {
      const val = Number(this._els.gamma.value);
      this._gamma = val;
      this._els.valueGamma.textContent = val.toFixed(1);
    });

    this._els.gamma.addEventListener("change", () => {
      this._onGammaChange();
      this._schedulePersist();
    });
  }

  /**
   * Populate cluster filter toggles from the cluster registry.
   * @param {ClusterInfo[]} clusters - Array of cluster info objects
   */
  setClusters(clusters) {
    this._clusters = clusters;
    this._renderClusterToggles();
  }

  /**
   * Short label for the cluster list using the centroid (most central) window text.
   * @param {ClusterInfo} cluster
   * @returns {string}
   */
  _clusterListLabel(cluster) {
    const text = (cluster.most_central_window_text || "").trim();
    if (!text) {
      return `Cluster ${cluster.cluster_id}`;
    }
    const maxLen = 72;
    if (text.length <= maxLen) {
      return text;
    }
    return `${text.slice(0, maxLen - 1)}…`;
  }

  /**
   * Full tooltip text for a cluster row.
   * @param {ClusterInfo} cluster
   * @returns {string}
   */
  _clusterListTitle(cluster) {
    const text = (cluster.most_central_window_text || "").trim();
    if (!text) {
      return `Cluster ${cluster.cluster_id} (${cluster.member_count} windows)`;
    }
    return `Cluster ${cluster.cluster_id} (${cluster.member_count} windows)\n${text}`;
  }

  /** Render cluster toggle checkboxes */
  _renderClusterToggles() {
    const list = this._els.clusterFilterList;
    list.innerHTML = "";

    if (this._clusters.length === 0) {
      list.innerHTML = '<span class="setting-note">No clusters loaded</span>';
      return;
    }

    const sorted = [...this._clusters].sort(
      (a, b) => a.cluster_id - b.cluster_id,
    );

    for (const cluster of sorted) {
      const isVisible = !this._hiddenClusters.has(cluster.cluster_id);
      const hueColor = this._hueToCSS(cluster.hue);
      const title = this._clusterListTitle(cluster);

      const toggle = document.createElement("label");
      toggle.className = "cluster-toggle";
      toggle.title = title;

      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.className = "cluster-checkbox";
      checkbox.dataset.clusterId = String(cluster.cluster_id);
      checkbox.checked = isVisible;
      checkbox.setAttribute(
        "aria-label",
        `Toggle cluster ${cluster.cluster_id}`,
      );

      const swatch = document.createElement("span");
      swatch.className = "cluster-swatch";
      swatch.style.backgroundColor = hueColor;
      swatch.setAttribute("aria-hidden", "true");

      const label = document.createElement("span");
      label.className = "cluster-toggle-label";
      label.textContent = this._clusterListLabel(cluster);

      const count = document.createElement("span");
      count.className = "cluster-toggle-count";
      count.textContent = `(${cluster.member_count})`;

      checkbox.addEventListener("change", () => {
        this._onClusterToggle(cluster.cluster_id, checkbox.checked);
      });

      toggle.append(checkbox, swatch, label, count);
      list.appendChild(toggle);
    }
  }

  /**
   * Convert a hue value (0–1) to a CSS hsl color string.
   * @param {number} hue - Hue in range 0–1
   * @returns {string}
   */
  _hueToCSS(hue) {
    return `hsl(${Math.round(hue * 360)}, 100%, 50%)`;
  }

  /**
   * Handle cluster toggle change — targeted re-raster via IPC.
   * @param {number} clusterId
   * @param {boolean} visible
   */
  async _onClusterToggle(clusterId, visible) {
    if (visible) {
      this._hiddenClusters.delete(clusterId);
    } else {
      this._hiddenClusters.add(clusterId);
    }

    // Find affected pages from the cluster-to-pages index
    const cluster = this._clusters.find((c) => c.cluster_id === clusterId);
    if (!cluster || !cluster.pages || cluster.pages.length === 0) {
      this._schedulePersist();
      return;
    }

    // Targeted re-raster: only pages containing this cluster
    await this._rasterPages(cluster.pages);
    this._schedulePersist();
  }

  /** Handle gamma change — full re-raster via IPC (all pages) */
  async _onGammaChange() {
    if (this._allPages.length === 0) return;
    await this._rasterPages(this._allPages);
  }

  /**
   * Call raster_pages IPC command.
   * @param {number[]} pages - Pages to re-raster
   */
  async _rasterPages(pages) {
    if (!this.jobId || pages.length === 0) return;

    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) {
        console.warn("Tauri runtime not available for raster_pages");
        return;
      }

      const result = await invoke("raster_pages", {
        jobId: this.jobId,
        pages: pages,
        threshold: this._tolerance,
        gamma: this._gamma,
        hiddenClusters: Array.from(this._hiddenClusters),
      });

      // Notify caller that pages were updated
      if (this.onPagesUpdated) {
        this.onPagesUpdated(result);
      }
    } catch (err) {
      console.error("raster_pages failed:", err);
    }
  }

  /** Schedule debounced display state persistence (2 seconds) */
  _schedulePersist() {
    if (this._persistTimer !== null) {
      clearTimeout(this._persistTimer);
    }
    this._persistTimer = setTimeout(() => {
      this._persistTimer = null;
      this._persistDisplayState();
    }, 2000);
  }

  /** Persist display state via save_display_state IPC */
  async _persistDisplayState() {
    if (!this.jobId) return;

    const state = this.getDisplayState();

    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) {
        console.warn("Tauri runtime not available for save_display_state");
        return;
      }

      await invoke("save_display_state", { state });
    } catch (err) {
      console.error("save_display_state failed:", err);
    }
  }

  /**
   * Get the current display state object.
   * @returns {DisplayState}
   */
  getDisplayState() {
    return {
      job_id: this.jobId,
      tolerance: this._tolerance,
      gamma: this._gamma,
      hidden_clusters: Array.from(this._hiddenClusters),
      zoom: 1.0,
      scroll_x: 0,
      scroll_y: 0,
      saved_at: new Date().toISOString(),
    };
  }

  /**
   * Restore display state from a saved state object.
   * @param {DisplayState} state
   */
  restoreState(state) {
    if (state.tolerance !== undefined) {
      this._tolerance = state.tolerance;
      this._els.tolerance.value = state.tolerance;
      this._els.valueTolerance.textContent = state.tolerance.toFixed(2);
    }

    if (state.gamma !== undefined) {
      this._gamma = state.gamma;
      this._els.gamma.value = state.gamma;
      this._els.valueGamma.textContent = state.gamma.toFixed(1);
    }

    if (state.hidden_clusters && Array.isArray(state.hidden_clusters)) {
      this._hiddenClusters = new Set(state.hidden_clusters);
      this._renderClusterToggles();
    }
  }

  /**
   * Set the job ID for IPC calls.
   * @param {string} jobId
   */
  setJobId(jobId) {
    this.jobId = jobId;
  }

  /**
   * Set the list of all pages in the current job (for gamma full re-raster).
   * @param {number[]} pages - All page numbers (1-based)
   */
  setAllPages(pages) {
    this._allPages = pages;
  }

  /**
   * Set the tolerance mask instance.
   * @param {import('./tolerance.js').ToleranceMask} mask
   */
  setToleranceMask(mask) {
    this.toleranceMask = mask;
  }

  /**
   * Set the canvases map for tolerance mask updates.
   * @param {Map<number, HTMLCanvasElement>} canvases
   */
  setCanvases(canvases) {
    this.canvases = canvases;
  }

  /** Get current tolerance value */
  get tolerance() {
    return this._tolerance;
  }

  /** Get current gamma value */
  get gamma() {
    return this._gamma;
  }

  /** Get current hidden clusters set */
  get hiddenClusters() {
    return new Set(this._hiddenClusters);
  }

  /** Destroy the panel and clean up timers */
  destroy() {
    if (this._persistTimer !== null) {
      clearTimeout(this._persistTimer);
      this._persistTimer = null;
    }
    this.container.innerHTML = "";
  }
}
