// Detail Panel — side panel showing window excerpts, cluster info, counterpart links.
// Triggered by macro-cell or sub-cell clicks, or text-preview highlight inspection.
// Calls get_page_detail via Tauri IPC and displays results grouped by cluster.

const CELL_SIZE = 20;

/**
 * @typedef {Object} SpanLocation
 * @property {number} chapter
 * @property {number} act
 * @property {number} paragraph_index
 * @property {string} segment_id
 * @property {number} sentence_index
 */

/**
 * @typedef {Object} EditSpan
 * @property {number} cluster_id
 * @property {number} instance_id
 * @property {number} doc_char_start
 * @property {number} doc_char_end
 * @property {string} text
 * @property {number} similarity_to_centroid
 * @property {SpanLocation|null} [location]
 */

/**
 * DetailPanel renders a side panel listing windows above Tolerance for a clicked cell,
 * grouped by cluster. Each entry shows window text excerpt, cluster hue indicator,
 * similarity score, and links to counterpart pages.
 *
 * Cluster inspection mode lists all spans in a cluster with SpanLocation fields,
 * canonical/duplicate role badges, and scroll-to-span links in the text preview.
 *
 * The panel updates in-place on new cell clicks without close/reopen.
 * No action is taken for clicks on empty/below-threshold sub-cells.
 */
export class DetailPanel {
  /**
   * @param {HTMLElement} container - The #detail-panel-container element
   * @param {object} options
   * @param {function} options.getZoom - Returns the current zoom level
   * @param {function} options.getTolerance - Returns the current tolerance value (0.75–1.00)
   * @param {function} options.getJobId - Returns the current job_id string or null
   * @param {function} [options.onCounterpartClick] - Called when a counterpart link is clicked:
   *   (page: number, subCellRow: number, subCellCol: number) => void
   * @param {function} [options.onSpanNavigate] - Called when a cluster span link is clicked:
   *   (span: EditSpan, role: "canonical"|"duplicate") => void
   */
  constructor(container, options = {}) {
    this._container = container;
    this._getZoom = options.getZoom || (() => 1);
    this._getTolerance = options.getTolerance || (() => 0.88);
    this._getJobId = options.getJobId || (() => null);
    this._onCounterpartClick = options.onCounterpartClick || null;
    this._onSpanNavigate = options.onSpanNavigate || null;

    this._visible = false;
    this._currentPage = null;
    this._currentRow = null;
    this._currentCol = null;
    /** @type {object|null} */
    this._visualizationPayload = null;
    /** @type {number|null} */
    this._activeClusterId = null;

    this._panelEl = null;
    this._headerEl = null;
    this._titleEl = null;
    this._contentEl = null;

    this._handleGridClick = this._handleGridClick.bind(this);

    this._buildPanel();
  }

  /**
   * Build the panel DOM structure inside the container.
   * @private
   */
  _buildPanel() {
    this._container.innerHTML = "";

    this._panelEl = document.createElement("div");
    this._panelEl.className = "detail-panel";
    this._panelEl.setAttribute("role", "complementary");
    this._panelEl.setAttribute("aria-label", "Detail panel");

    this._headerEl = document.createElement("div");
    this._headerEl.className = "detail-panel-header";

    const title = document.createElement("span");
    title.className = "detail-panel-title";
    title.textContent = "Detail";
    this._titleEl = title;
    this._headerEl.appendChild(title);

    const closeBtn = document.createElement("button");
    closeBtn.className = "detail-panel-close";
    closeBtn.textContent = "\u00D7";
    closeBtn.setAttribute("aria-label", "Close detail panel");
    closeBtn.addEventListener("click", () => this.hide());
    this._headerEl.appendChild(closeBtn);

    this._contentEl = document.createElement("div");
    this._contentEl.className = "detail-panel-content";

    this._panelEl.appendChild(this._headerEl);
    this._panelEl.appendChild(this._contentEl);
    this._container.appendChild(this._panelEl);
  }

  /**
   * Attach click listener to the grid container.
   * @param {HTMLElement} gridContainer - The #grid-container element
   */
  attachToGrid(gridContainer) {
    this._gridContainer = gridContainer;
    gridContainer.addEventListener("click", this._handleGridClick);
  }

  /**
   * Handle click events on the grid.
   * Determines the clicked page and sub-cell, then fetches detail data.
   * @private
   * @param {MouseEvent} e
   */
  _handleGridClick(e) {
    const canvas = e.target.closest("canvas[data-page]");
    if (!canvas) return;

    const page = parseInt(canvas.dataset.page, 10);
    if (isNaN(page)) return;

    const subCell = this._getSubCellFromEvent(e, canvas);

    if (subCell) {
      this._fetchAndDisplay(page, subCell.row, subCell.col);
    } else {
      // Macro-cell click — use row=0, col=0 as representative (full page detail)
      this._fetchAndDisplay(page, 0, 0);
    }
  }

  /**
   * Determine which sub-cell was clicked based on position within the canvas.
   * @private
   * @param {MouseEvent} e
   * @param {HTMLCanvasElement} canvas
   * @returns {{row: number, col: number} | null}
   */
  _getSubCellFromEvent(e, canvas) {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    const col = Math.floor((x / rect.width) * CELL_SIZE);
    const row = Math.floor((y / rect.height) * CELL_SIZE);

    if (row < 0 || row >= CELL_SIZE || col < 0 || col >= CELL_SIZE) {
      return null;
    }

    return { row, col };
  }

  /**
   * Store the latest visualization payload for cluster inspection.
   * @param {object|null} payload
   */
  setVisualizationPayload(payload) {
    this._visualizationPayload = payload || null;
  }

  /**
   * Show cluster inspection with SpanLocation fields for each span.
   * @param {number} clusterId
   * @param {EditSpan} [focusSpan] - Span to emphasize (e.g. from highlight click)
   */
  showCluster(clusterId, focusSpan) {
    const payload = this._visualizationPayload || window.currentVisualizationPayload;
    if (!payload?.repetition_report?.clusters) {
      return;
    }

    const cluster = payload.repetition_report.clusters.find(
      (entry) => entry.cluster_id === clusterId,
    );
    if (!cluster) {
      return;
    }

    this._activeClusterId = clusterId;
    this._currentPage = null;
    this._currentRow = null;
    this._currentCol = null;

    this._renderClusterInspection(cluster, focusSpan);
    this.show();
  }

  /**
   * @private
   * @param {object} cluster
   * @param {EditSpan} [focusSpan]
   */
  _renderClusterInspection(cluster, focusSpan) {
    this._contentEl.innerHTML = "";
    if (this._titleEl) {
      this._titleEl.textContent = `Cluster ${cluster.cluster_id}`;
    }

    const sectionHeader = document.createElement("div");
    sectionHeader.className = "detail-section-header";

    const pageLabel = document.createElement("div");
    pageLabel.className = "detail-page-label";
    pageLabel.textContent = `${cluster.instance_count} instances · ${cluster.spans?.length ?? 0} spans`;
    sectionHeader.appendChild(pageLabel);

    if (cluster.representative_text) {
      const rep = document.createElement("div");
      rep.className = "detail-meta detail-cluster-representative";
      rep.textContent = this._truncateText(cluster.representative_text, 120);
      sectionHeader.appendChild(rep);
    }

    this._contentEl.appendChild(sectionHeader);

    const spansSection = document.createElement("div");
    spansSection.className = "detail-matches-section";

    const spansTitle = document.createElement("div");
    spansTitle.className = "detail-matches-title";
    spansTitle.textContent = "Spans";
    spansSection.appendChild(spansTitle);

    const spansList = document.createElement("div");
    spansList.className = "detail-matches-list detail-spans-list";

    const spans = cluster.spans?.length
      ? cluster.spans
      : [cluster.canonical, ...(cluster.duplicates || [])];

    for (const span of spans) {
      const role =
        span.instance_id === cluster.canonical?.instance_id
          ? "canonical"
          : "duplicate";
      const isFocused =
        focusSpan &&
        span.doc_char_start === focusSpan.doc_char_start &&
        span.instance_id === focusSpan.instance_id;
      spansList.appendChild(
        this._createClusterSpanItem(span, role, cluster.cluster_id, isFocused),
      );
    }

    spansSection.appendChild(spansList);
    this._contentEl.appendChild(spansSection);
    this._contentEl.scrollTop = 0;
  }

  /**
   * @private
   * @param {EditSpan} span
   * @param {"canonical"|"duplicate"} role
   * @param {number} clusterId
   * @param {boolean} [focused]
   * @returns {HTMLElement}
   */
  _createClusterSpanItem(span, role, clusterId, focused = false) {
    const item = document.createElement("div");
    item.className = "detail-match-item detail-span-item";
    if (focused) {
      item.classList.add("detail-span-item-focused");
    }

    const header = document.createElement("div");
    header.className = "detail-span-header";

    const badge = document.createElement("span");
    badge.className = `detail-role-badge detail-role-${role}`;
    badge.textContent = role === "canonical" ? "Keep" : "Duplicate";
    header.appendChild(badge);

    const simBadge = document.createElement("span");
    simBadge.className = "detail-match-sim";
    simBadge.textContent = span.similarity_to_centroid.toFixed(3);
    header.appendChild(simBadge);

    item.appendChild(header);

    if (span.location) {
      const breadcrumb = document.createElement("div");
      breadcrumb.className = "detail-location-breadcrumb";
      breadcrumb.textContent = this._formatLocationBreadcrumb(span.location);
      item.appendChild(breadcrumb);

      const fields = document.createElement("dl");
      fields.className = "detail-location-fields";
      this._appendLocationField(fields, "segment_id", span.location.segment_id);
      this._appendLocationField(fields, "act", String(span.location.act));
      this._appendLocationField(
        fields,
        "paragraph_index",
        String(span.location.paragraph_index),
      );
      this._appendLocationField(
        fields,
        "sentence_index",
        String(span.location.sentence_index),
      );
      item.appendChild(fields);
    }

    const link = document.createElement("a");
    link.className = "detail-match-link detail-span-link";
    link.href = "#";
    link.textContent = span.location?.segment_id
      ? `Go to ${span.location.segment_id}`
      : "Go to span";
    link.addEventListener("click", (e) => {
      e.preventDefault();
      if (this._onSpanNavigate) {
        this._onSpanNavigate(span, role);
      }
    });
    item.appendChild(link);

    const excerpt = document.createElement("div");
    excerpt.className = "detail-match-excerpt";
    excerpt.textContent = this._truncateText(span.text, 160);
    item.appendChild(excerpt);

    return item;
  }

  /**
   * @private
   * @param {SpanLocation} location
   * @returns {string}
   */
  _formatLocationBreadcrumb(location) {
    const parts = [];
    if (location.chapter) {
      parts.push(`Ch ${location.chapter}`);
    }
    if (location.act) {
      parts.push(`Act ${location.act}`);
    }
    if (location.paragraph_index) {
      parts.push(`¶${location.paragraph_index}`);
    }
    if (location.sentence_index) {
      parts.push(`Sent ${location.sentence_index}`);
    }
    return parts.join(" · ");
  }

  /**
   * @private
   * @param {HTMLDListElement} dl
   * @param {string} label
   * @param {string} value
   */
  _appendLocationField(dl, label, value) {
    const dt = document.createElement("dt");
    dt.textContent = label;
    const dd = document.createElement("dd");
    dd.textContent = value;
    if (label === "segment_id") {
      dd.className = "detail-segment-id";
    }
    dl.appendChild(dt);
    dl.appendChild(dd);
  }

  /**
   * Fetch detail data from the backend and display it.
   * Does nothing if the sub-cell is empty or below threshold.
   * @private
   * @param {number} page - 1-based page number
   * @param {number} row - Sub-cell row (0–19)
   * @param {number} col - Sub-cell col (0–19)
   */
  async _fetchAndDisplay(page, row, col) {
    const jobId = this._getJobId();
    if (!jobId) return;

    const tolerance = this._getTolerance();

    try {
      const detail = await this._invokeGetPageDetail(jobId, page, row, col, tolerance);

      // No action for empty/below-threshold sub-cells
      if (!detail || !detail.window_text) {
        return;
      }

      this._currentPage = page;
      this._currentRow = row;
      this._currentCol = col;

      this._renderDetail(detail, page, row, col);
      this.show();
    } catch (err) {
      // Silently ignore errors (e.g., no data for this cell)
      console.warn("Detail panel: failed to fetch page detail", err);
    }
  }

  /**
   * Invoke the get_page_detail Tauri command.
   * @private
   * @param {string} jobId
   * @param {number} page
   * @param {number} row
   * @param {number} col
   * @param {number} threshold
   * @returns {Promise<{window_text: string, cluster_id: number, similarity: number, matches: Array}>}
   */
  async _invokeGetPageDetail(jobId, page, row, col, threshold) {
    const tauri = window.__TAURI__;
    if (!tauri || !tauri.core || !tauri.core.invoke) {
      console.warn("Tauri invoke API not available");
      return null;
    }

    return await tauri.core.invoke("get_page_detail", {
      jobId,
      page,
      row,
      col,
      threshold
    });
  }

  /**
   * Render the detail data into the panel content area.
   * Groups matches by cluster and shows window excerpts with counterpart links.
   * @private
   * @param {object} detail - SubCellDetail response
   * @param {number} page - Current page
   * @param {number} row - Current sub-cell row
   * @param {number} col - Current sub-cell col
   */
  _renderDetail(detail, page, row, col) {
    this._contentEl.innerHTML = "";
    this._activeClusterId = null;
    if (this._titleEl) {
      this._titleEl.textContent = "Detail";
    }

    // Header section with current window info
    const sectionHeader = document.createElement("div");
    sectionHeader.className = "detail-section-header";

    const pageLabel = document.createElement("div");
    pageLabel.className = "detail-page-label";
    pageLabel.textContent = `Page ${page} \u2014 Position (${row}, ${col})`;
    sectionHeader.appendChild(pageLabel);

    this._contentEl.appendChild(sectionHeader);

    // Current window excerpt
    const currentWindow = document.createElement("div");
    currentWindow.className = "detail-current-window";

    const hueIndicator = this._createHueIndicator(detail.cluster_id);
    currentWindow.appendChild(hueIndicator);

    const windowInfo = document.createElement("div");
    windowInfo.className = "detail-window-info";

    const excerpt = document.createElement("div");
    excerpt.className = "detail-excerpt";
    excerpt.textContent = this._truncateText(detail.window_text, 200);
    windowInfo.appendChild(excerpt);

    const meta = document.createElement("div");
    meta.className = "detail-meta";
    meta.textContent = `Cluster ${detail.cluster_id} \u2022 Similarity: ${detail.similarity.toFixed(3)}`;
    windowInfo.appendChild(meta);

    currentWindow.appendChild(windowInfo);
    this._contentEl.appendChild(currentWindow);

    // Counterpart matches grouped by cluster
    if (detail.matches && detail.matches.length > 0) {
      const matchesSection = document.createElement("div");
      matchesSection.className = "detail-matches-section";

      const matchesTitle = document.createElement("div");
      matchesTitle.className = "detail-matches-title";
      matchesTitle.textContent = `Counterparts (${detail.matches.length})`;
      matchesSection.appendChild(matchesTitle);

      // Group matches by cluster (in this case they share the same cluster_id from the detail)
      const matchesList = document.createElement("div");
      matchesList.className = "detail-matches-list";

      for (const match of detail.matches) {
        const matchItem = this._createMatchItem(match);
        matchesList.appendChild(matchItem);
      }

      matchesSection.appendChild(matchesList);
      this._contentEl.appendChild(matchesSection);
    }

    // Scroll to top of content
    this._contentEl.scrollTop = 0;
  }

  /**
   * Create a match item element for a counterpart window.
   * @private
   * @param {object} match - WindowMatch object
   * @returns {HTMLElement}
   */
  _createMatchItem(match) {
    const item = document.createElement("div");
    item.className = "detail-match-item";

    const link = document.createElement("a");
    link.className = "detail-match-link";
    link.href = "#";
    link.dataset.navigatePage = String(match.page);
    link.textContent = `Page ${match.page}`;
    link.addEventListener("click", (e) => {
      e.preventDefault();
      if (this._onCounterpartClick) {
        this._onCounterpartClick(match.page, match.sub_cell_row, match.sub_cell_col);
      }
    });
    item.appendChild(link);

    const simBadge = document.createElement("span");
    simBadge.className = "detail-match-sim";
    simBadge.textContent = match.similarity.toFixed(3);
    item.appendChild(simBadge);

    const matchExcerpt = document.createElement("div");
    matchExcerpt.className = "detail-match-excerpt";
    matchExcerpt.textContent = this._truncateText(match.window_text, 120);
    item.appendChild(matchExcerpt);

    return item;
  }

  /**
   * Create a cluster hue indicator element.
   * Uses the golden-ratio hue assignment: (cluster_id × 0.6180339887) mod 1.0
   * @private
   * @param {number} clusterId
   * @returns {HTMLElement}
   */
  _createHueIndicator(clusterId) {
    const indicator = document.createElement("div");
    indicator.className = "detail-hue-indicator";

    const hue = ((clusterId * 0.6180339887) % 1.0) * 360;
    indicator.style.backgroundColor = `hsl(${hue}, 100%, 50%)`;

    return indicator;
  }

  /**
   * Truncate text to a maximum length, appending ellipsis if needed.
   * @private
   * @param {string} text
   * @param {number} maxLen
   * @returns {string}
   */
  _truncateText(text, maxLen) {
    if (!text) return "";
    if (text.length <= maxLen) return text;
    return text.slice(0, maxLen) + "\u2026";
  }

  /**
   * Show the detail panel.
   */
  show() {
    if (this._visible) return;
    this._container.classList.add("detail-panel-visible");
    this._visible = true;
  }

  /**
   * Hide the detail panel.
   */
  hide() {
    if (!this._visible) return;
    this._container.classList.remove("detail-panel-visible");
    this._visible = false;
    this._currentPage = null;
    this._currentRow = null;
    this._currentCol = null;
    this._activeClusterId = null;
  }

  /**
   * Returns whether the panel is currently visible.
   * @returns {boolean}
   */
  isVisible() {
    return this._visible;
  }

  /**
   * Detach event listeners and clean up.
   */
  destroy() {
    if (this._gridContainer) {
      this._gridContainer.removeEventListener("click", this._handleGridClick);
      this._gridContainer = null;
    }
    this._container.innerHTML = "";
    this._panelEl = null;
    this._headerEl = null;
    this._contentEl = null;
  }
}
