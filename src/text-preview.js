// Text preview with cluster span highlighting for tuning against RF output.

import {
  exportAnalysisOutputJson,
} from "./export-analysis-output.js";

/**
 * @typedef {Object} SpanLocation
 * @property {number} chapter
 * @property {number} act
 * @property {number} paragraph_index
 * @property {string} segment_id
 * @property {number} sentence_index
 */

/**
 * @typedef {Object} TextHighlight
 * @property {number} cluster_id
 * @property {number} instance_id
 * @property {"canonical"|"duplicate"} role
 * @property {number} doc_char_start
 * @property {number} doc_char_end
 * @property {number} page
 * @property {number} hue
 * @property {number} similarity_to_centroid
 * @property {string} text
 * @property {SpanLocation|null} [location]
 */

export class TextPreviewPanel {
  /**
   * @param {HTMLElement} container
   * @param {Object} [options]
   * @param {(highlight: TextHighlight) => void} [options.onHighlightClick]
   */
  constructor(container, options = {}) {
    this.container = container;
    this.onHighlightClick = options.onHighlightClick || (() => {});
    /** @type {TextHighlight[]} */
    this._highlights = [];
    this._documentText = "";
    this._activeClusterId = null;
    this._payload = null;
    this._scopeManifest = null;
    this._buildUI();
  }

  _buildUI() {
    this.container.innerHTML = `
      <div class="text-preview-panel">
        <div class="text-preview-header">
          <h2 class="text-preview-title">Text Preview</h2>
          <div class="text-preview-actions">
            <button type="button" id="btn-export-json" class="btn-secondary" title="Export visualization JSON">
              Export JSON
            </button>
            <button
              type="button"
              id="btn-export-report-json"
              class="btn-secondary"
              title="Export pipeline AnalysisOutput JSON for Romance Factory"
            >
              Export report JSON
            </button>
          </div>
        </div>
        <p class="text-preview-export-status" id="text-preview-export-status" role="status" aria-live="polite"></p>
        <p class="text-preview-hint setting-note">
          Highlighted spans from the core analysis. Canonical = first occurrence; duplicates = edit targets.
        </p>
        <div class="text-preview-legend">
          <span class="legend-item legend-canonical">Keep (1st)</span>
          <span class="legend-item legend-duplicate">Duplicate</span>
        </div>
        <div id="text-preview-body" class="text-preview-body" tabindex="0" aria-label="Document text with repetition highlights"></div>
      </div>
    `;

    this._body = this.container.querySelector("#text-preview-body");
    this._exportBtn = this.container.querySelector("#btn-export-json");
    this._exportReportBtn = this.container.querySelector("#btn-export-report-json");
    this._exportStatus = this.container.querySelector("#text-preview-export-status");
    this._exportBtn.addEventListener("click", () => this._exportJson());
    this._exportReportBtn.addEventListener("click", () => this._exportReportJson());
    this.clear();
  }

  /**
   * @param {object} payload
   */
  applyPayload(payload) {
    this._payload = payload;
    this._documentText = payload.document_text || "";
    this._highlights = payload.highlights || [];
    this._scopeManifest = payload.scope_manifest || null;
    this._render();
  }

  /** @param {number|null} clusterId */
  setActiveCluster(clusterId) {
    this._activeClusterId = clusterId;
    this._render();
  }

  /**
   * Scroll the preview to a span and briefly emphasize it.
   * @param {number} docCharStart
   * @param {number|null} [clusterId]
   */
  scrollToSpan(docCharStart, clusterId) {
    if (clusterId != null) {
      this._activeClusterId = clusterId;
    }
    this._render();

    const mark = this._body.querySelector(
      `mark[data-char-start="${docCharStart}"]`,
    );
    if (!mark) return;

    mark.classList.add("text-highlight-focused");
    mark.scrollIntoView({ behavior: "smooth", block: "center" });

    window.setTimeout(() => {
      mark.classList.remove("text-highlight-focused");
    }, 2000);
  }

  clear() {
    this._payload = null;
    this._documentText = "";
    this._highlights = [];
    this._activeClusterId = null;
    this._scopeManifest = null;
    this._setExportStatus("");
    this._body.textContent = "Run analysis to preview highlighted spans.";
  }

  /** @param {string} message @param {"info"|"error"} [kind] */
  _setExportStatus(message, kind = "info") {
    if (!this._exportStatus) return;
    this._exportStatus.textContent = message;
    this._exportStatus.dataset.kind = kind;
  }

  /** @param {unknown} err @returns {string} */
  _formatError(err) {
    if (typeof err === "string") return err;
    if (err && typeof err === "object") {
      const o = /** @type {Record<string, unknown>} */ (err);
      if (typeof o.message === "string") return o.message;
      const detail = o.detail;
      if (detail && typeof detail === "object" && typeof detail.message === "string") {
        return detail.message;
      }
    }
    return "Export failed";
  }

  _hueToCSS(hue) {
    const h = ((hue % 1) + 1) % 1;
    return `hsl(${Math.round(h * 360)}, 70%, 45%)`;
  }

  _render() {
    if (!this._documentText) {
      this._body.textContent = "Run analysis to preview highlighted spans.";
      return;
    }

    this._body.innerHTML = "";
    const frag = document.createDocumentFragment();
    let cursor = 0;

    const actStarts = new Set(
      (this._scopeManifest?.acts || [])
        .map((a) => a.scope_char_start)
        .filter((start) => start > 0),
    );

    const emitActBoundary = (offset) => {
      if (!actStarts.has(offset)) return;
      const act = (this._scopeManifest?.acts || []).find(
        (a) => a.scope_char_start === offset,
      );
      const div = document.createElement("div");
      div.className = "text-act-boundary";
      div.setAttribute("role", "separator");
      div.textContent = act ? `— Act ${act.act} —` : "— Act boundary —";
      frag.appendChild(div);
    };

    const appendProseWithActBoundaries = (from, to) => {
      let pos = from;
      const boundaries = [...actStarts]
        .filter((b) => b > from && b < to)
        .sort((a, b) => a - b);
      for (const boundary of boundaries) {
        if (boundary > pos) {
          frag.appendChild(
            document.createTextNode(this._documentText.slice(pos, boundary)),
          );
        }
        emitActBoundary(boundary);
        pos = boundary;
      }
      if (pos < to) {
        frag.appendChild(document.createTextNode(this._documentText.slice(pos, to)));
      }
    };

    if (this._highlights.length === 0) {
      if (actStarts.size > 0) {
        appendProseWithActBoundaries(0, this._documentText.length);
      } else {
        frag.appendChild(document.createTextNode(this._documentText));
      }
      this._body.appendChild(frag);
      return;
    }

    for (const highlight of this._highlights) {
      const start = highlight.doc_char_start;
      const end = highlight.doc_char_end;

      if (start > cursor) {
        appendProseWithActBoundaries(cursor, start);
      }

      const mark = document.createElement("mark");
      mark.className = `text-highlight text-highlight-${highlight.role}`;
      if (this._activeClusterId === highlight.cluster_id) {
        mark.classList.add("text-highlight-active");
      }
      mark.dataset.clusterId = String(highlight.cluster_id);
      mark.dataset.page = String(highlight.page);
      mark.dataset.role = highlight.role;
      mark.dataset.charStart = String(highlight.doc_char_start);
      mark.style.setProperty("--highlight-hue", this._hueToCSS(highlight.hue));
      const locationHint = highlight.location?.segment_id
        ? ` · ${highlight.location.segment_id}`
        : "";
      mark.title = `Cluster ${highlight.cluster_id} · ${highlight.role} · page ${highlight.page} · sim ${highlight.similarity_to_centroid.toFixed(2)}${locationHint}`;
      mark.textContent = this._documentText.slice(start, end);
      mark.addEventListener("click", (e) => {
        e.stopPropagation();
        this.onHighlightClick(highlight);
      });
      frag.appendChild(mark);
      cursor = Math.max(cursor, end);
    }

    if (cursor < this._documentText.length) {
      appendProseWithActBoundaries(cursor, this._documentText.length);
    }

    this._body.appendChild(frag);
  }

  async _exportJson() {
    const invoke = window.__TAURI__?.core?.invoke;
    const jobId = window.currentJobId;
    if (!invoke || !jobId) {
      console.warn("Cannot export: no active job");
      return;
    }

    try {
      const payload = await invoke("get_visualization_payload", {
        jobId,
        tolerance: window.displaySettingsPanel?.getTolerance?.() ?? 0.75,
        gamma: window.displaySettingsPanel?.gamma ?? 1.5,
        expandToSentences: true,
      });

      const blob = new Blob([JSON.stringify(payload, null, 2)], {
        type: "application/json",
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `similarity-map-${jobId.slice(0, 8)}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Export JSON failed:", err);
      this._setExportStatus(this._formatError(err), "error");
    }
  }

  async _exportReportJson() {
    const output =
      window.currentAnalysisOutput ||
      window.currentVisualizationPayload?.analysis_output ||
      this._payload?.analysis_output;

    if (!output) {
      this._setExportStatus(
        "No pipeline report available — run analysis first.",
        "error",
      );
      return;
    }

    const panel = window.importSettingsPanel;
    const storyPath = panel?.rfStoryPath || null;
    const chapter = panel?.rfChapter ?? null;

    try {
      this._exportReportBtn.disabled = true;
      this._setExportStatus("Validating report…");

      const { savedPath, filename } = await exportAnalysisOutputJson(output, {
        storyPath,
        chapter,
      });

      if (savedPath) {
        this._setExportStatus(`Saved ${savedPath}`);
      } else {
        this._setExportStatus(`Downloaded ${filename}`);
      }
    } catch (err) {
      console.error("Export report JSON failed:", err);
      this._setExportStatus(this._formatError(err), "error");
    } finally {
      this._exportReportBtn.disabled = false;
    }
  }
}

/**
 * @param {object} payload
 */
export async function applyVisualizationPayload(payload) {
  window.currentJobId = payload.job_id;
  window.currentVisualizationPayload = payload;
  window.currentScopeManifest = payload.scope_manifest || null;
  window.currentAnalysisOutput = payload.analysis_output || null;

  const grid = window.gridRenderer;
  const pageCount = payload.pages?.length || payload.page_rasters?.length || 0;

  if (grid && pageCount > 0) {
    grid.initGrid(pageCount, payload.scope_manifest || null);
    for (const raster of payload.page_rasters || []) {
      await grid.updatePage(raster.page, raster.canvas_rgba_b64);
    }
  }

  const display = window.displaySettingsPanel;
  if (display) {
    display.setJobId(payload.job_id);
    display.setAllPages(Array.from({ length: pageCount }, (_, i) => i + 1));
    if (grid) {
      display.setCanvases(grid._canvases);
    }
    if (payload.cluster_registry?.clusters) {
      display.setClusters(Object.values(payload.cluster_registry.clusters));
    }
  }

  const textPreview = window.textPreviewPanel;
  if (textPreview) {
    textPreview.applyPayload(payload);
  }

  const detailPanel = window.detailPanel;
  if (detailPanel) {
    detailPanel.setVisualizationPayload(payload);
  }

  const resultsPanel = window.resultsPanel;
  if (resultsPanel) {
    await resultsPanel.refresh();
  }
}
