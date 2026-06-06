// Tooltip Manager — contextual tooltips for macro-cells and sub-cells.
// Macro-cell tooltips at base zoom: page number, top 3 clusters, max similarity.
// Sub-cell tooltips at ≥ 5×5 px per sub-cell: position %, cluster name, sim score, excerpt.

const CELL_SIZE = 20; // native canvas resolution (20×20 sub-cells per page)
const SUB_CELL_TOOLTIP_THRESHOLD = 5; // minimum px per sub-cell to show sub-cell tooltips

/**
 * TooltipManager provides hover tooltips for the similarity map grid.
 *
 * At base zoom (sub-cells < 5px), hovering a page cell shows a macro-cell tooltip
 * with page number, top 3 clusters, and max similarity.
 *
 * At higher zoom (sub-cells ≥ 5px), hovering shows a sub-cell tooltip with
 * position %, cluster name, similarity score, and a 120-char text excerpt.
 *
 * No tooltip is shown for cells/sub-cells with no clusters above Tolerance.
 */
export class TooltipManager {
  /**
   * @param {HTMLElement} gridContainer - The #grid-container element
   * @param {object} options
   * @param {function} options.getZoom - Returns the current zoom level
   * @param {function} options.getTolerance - Returns the current tolerance value (0.75–1.00)
   * @param {function} [options.getPageSubCellData] - Returns sub-cell cluster data for a page:
   *   (page: number) => Array<Array<{cluster_id, sim_to_centroid, window_id}>> | null
   *   The array is 400 entries (row-major 20×20), each entry is an array of cluster objects.
   * @param {function} [options.getClusterInfo] - Returns cluster info by ID:
   *   (cluster_id: number) => { cluster_id, member_count, most_central_window_text, pages } | null
   * @param {function} [options.getWindowExcerpt] - Returns window text by window_id:
   *   (window_id: string) => string | null
   */
  constructor(gridContainer, options = {}) {
    this._container = gridContainer;
    this._getZoom = options.getZoom || (() => 1);
    this._getTolerance = options.getTolerance || (() => 0.88);
    this._getPageSubCellData = options.getPageSubCellData || (() => null);
    this._getClusterInfo = options.getClusterInfo || (() => null);
    this._getWindowExcerpt = options.getWindowExcerpt || (() => null);

    this._tooltipEl = null;
    this._visible = false;

    this._handleMouseMove = this._handleMouseMove.bind(this);
    this._handleMouseLeave = this._handleMouseLeave.bind(this);

    this._createTooltipElement();
    this._attachListeners();
  }

  /**
   * Create the floating tooltip DOM element.
   * @private
   */
  _createTooltipElement() {
    this._tooltipEl = document.createElement("div");
    this._tooltipEl.className = "sim-tooltip";
    this._tooltipEl.setAttribute("role", "tooltip");
    this._tooltipEl.setAttribute("aria-hidden", "true");
    document.body.appendChild(this._tooltipEl);
  }

  /**
   * Attach mouse event listeners to the grid container.
   * @private
   */
  _attachListeners() {
    this._container.addEventListener("mousemove", this._handleMouseMove);
    this._container.addEventListener("mouseleave", this._handleMouseLeave);
  }

  /**
   * Handle mouse movement over the grid.
   * Determines whether to show macro-cell or sub-cell tooltip based on zoom.
   * @private
   * @param {MouseEvent} e
   */
  _handleMouseMove(e) {
    const canvas = e.target.closest("canvas[data-page]");
    if (!canvas) {
      this._hide();
      return;
    }

    const page = parseInt(canvas.dataset.page, 10);
    if (isNaN(page)) {
      this._hide();
      return;
    }

    const tolerance = this._getTolerance();
    const cellRect = canvas.getBoundingClientRect();
    const subCellPx = cellRect.width / CELL_SIZE;

    let content;

    if (subCellPx >= SUB_CELL_TOOLTIP_THRESHOLD) {
      // Sub-cell tooltip mode
      const subCell = this._getSubCellFromEvent(e, canvas);
      if (!subCell) {
        this._hide();
        return;
      }
      content = this._buildSubCellTooltip(page, subCell.row, subCell.col, tolerance);
    } else {
      // Macro-cell tooltip mode
      content = this._buildMacroCellTooltip(page, tolerance);
    }

    if (!content) {
      this._hide();
      return;
    }

    this._show(content, e.clientX, e.clientY);
  }

  /**
   * Handle mouse leaving the grid container.
   * @private
   */
  _handleMouseLeave() {
    this._hide();
  }

  /**
   * Determine which sub-cell the mouse is over based on position within the canvas.
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
   * Build macro-cell tooltip content.
   * Shows: page number, top 3 clusters (by sim), max similarity.
   * @private
   * @param {number} page - 1-based page number
   * @param {number} tolerance - Current tolerance threshold
   * @returns {string|null} HTML content or null if nothing to show
   */
  _buildMacroCellTooltip(page, tolerance) {
    const subCellData = this._getPageSubCellData(page);
    if (!subCellData) return null;

    // Collect all clusters above tolerance across all sub-cells on this page
    const clusterMap = new Map(); // cluster_id → max sim_to_centroid

    for (let i = 0; i < subCellData.length; i++) {
      const clusters = subCellData[i];
      if (!clusters) continue;
      for (const entry of clusters) {
        if (entry.sim_to_centroid > tolerance) {
          const existing = clusterMap.get(entry.cluster_id);
          if (existing === undefined || entry.sim_to_centroid > existing) {
            clusterMap.set(entry.cluster_id, entry.sim_to_centroid);
          }
        }
      }
    }

    if (clusterMap.size === 0) return null;

    // Sort by sim descending, take top 3
    const sorted = [...clusterMap.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 3);

    const maxSim = sorted[0][1];

    // Build cluster labels
    const clusterLines = sorted.map(([clusterId, sim]) => {
      const info = this._getClusterInfo(clusterId);
      const label = info ? `Cluster ${clusterId}` : `Cluster ${clusterId}`;
      return `<span class="sim-tooltip-cluster">${label} (${sim.toFixed(3)})</span>`;
    });

    return (
      `<div class="sim-tooltip-header">Page ${page}</div>` +
      `<div class="sim-tooltip-body">` +
      clusterLines.join("") +
      `<span class="sim-tooltip-max">Max similarity: ${maxSim.toFixed(3)}</span>` +
      `</div>`
    );
  }

  /**
   * Build sub-cell tooltip content.
   * Shows: position %, cluster name, sim score, excerpt (120 chars).
   * @private
   * @param {number} page - 1-based page number
   * @param {number} row - Sub-cell row (0–19)
   * @param {number} col - Sub-cell col (0–19)
   * @param {number} tolerance - Current tolerance threshold
   * @returns {string|null} HTML content or null if nothing to show
   */
  _buildSubCellTooltip(page, row, col, tolerance) {
    const subCellData = this._getPageSubCellData(page);
    if (!subCellData) return null;

    const index = row * CELL_SIZE + col;
    const clusters = subCellData[index];
    if (!clusters || clusters.length === 0) return null;

    // Find the top cluster above tolerance
    const aboveTolerance = clusters.filter((c) => c.sim_to_centroid > tolerance);
    if (aboveTolerance.length === 0) return null;

    const top = aboveTolerance[0]; // Already sorted by sim_to_centroid desc

    // Position as percentage of page (linear index / 400 * 100)
    const positionPct = ((index / 400) * 100).toFixed(1);

    // Get excerpt text
    let excerpt = "";
    if (this._getWindowExcerpt && top.window_id) {
      const text = this._getWindowExcerpt(top.window_id);
      if (text) {
        excerpt = text.length > 120 ? text.slice(0, 120) + "…" : text;
      }
    }

    const clusterLabel = `Cluster ${top.cluster_id}`;

    return (
      `<div class="sim-tooltip-header">${positionPct}% through page ${page}</div>` +
      `<div class="sim-tooltip-body">` +
      `<span class="sim-tooltip-cluster">${clusterLabel}</span>` +
      `<span class="sim-tooltip-sim">Similarity: ${top.sim_to_centroid.toFixed(3)}</span>` +
      (excerpt
        ? `<span class="sim-tooltip-excerpt">"${this._escapeHtml(excerpt)}"</span>`
        : "") +
      `</div>`
    );
  }

  /**
   * Show the tooltip at the given screen position.
   * @private
   * @param {string} html - Tooltip inner HTML
   * @param {number} clientX - Mouse X position
   * @param {number} clientY - Mouse Y position
   */
  _show(html, clientX, clientY) {
    this._tooltipEl.innerHTML = html;
    this._tooltipEl.classList.add("sim-tooltip-visible");
    this._tooltipEl.setAttribute("aria-hidden", "false");
    this._visible = true;

    // Position near cursor, avoiding off-screen overflow
    this._position(clientX, clientY);
  }

  /**
   * Hide the tooltip.
   * @private
   */
  _hide() {
    if (!this._visible) return;
    this._tooltipEl.classList.remove("sim-tooltip-visible");
    this._tooltipEl.setAttribute("aria-hidden", "true");
    this._visible = false;
  }

  /**
   * Position the tooltip near the cursor, keeping it within the viewport.
   * @private
   * @param {number} clientX
   * @param {number} clientY
   */
  _position(clientX, clientY) {
    const offset = 12; // px offset from cursor
    const el = this._tooltipEl;

    // Place to the right and below cursor initially
    let left = clientX + offset;
    let top = clientY + offset;

    // Measure tooltip dimensions
    const rect = el.getBoundingClientRect();
    const viewportW = window.innerWidth;
    const viewportH = window.innerHeight;

    // Adjust if overflowing right edge
    if (left + rect.width > viewportW) {
      left = clientX - rect.width - offset;
    }

    // Adjust if overflowing bottom edge
    if (top + rect.height > viewportH) {
      top = clientY - rect.height - offset;
    }

    // Clamp to viewport
    left = Math.max(0, left);
    top = Math.max(0, top);

    el.style.left = `${left}px`;
    el.style.top = `${top}px`;
  }

  /**
   * Escape HTML special characters to prevent XSS in tooltip content.
   * @private
   * @param {string} str
   * @returns {string}
   */
  _escapeHtml(str) {
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  /**
   * Remove event listeners and the tooltip element.
   */
  destroy() {
    this._container.removeEventListener("mousemove", this._handleMouseMove);
    this._container.removeEventListener("mouseleave", this._handleMouseLeave);
    if (this._tooltipEl && this._tooltipEl.parentNode) {
      this._tooltipEl.parentNode.removeChild(this._tooltipEl);
    }
    this._tooltipEl = null;
  }
}
