// Saved Results Panel — manage named analysis results for the open document.

import { activateJob } from "./job-activation.js";
import { mountSettingsYamlExportControls } from "./export-settings-yaml.js";

/**
 * @typedef {Object} SavedResultEntry
 * @property {string} result_id
 * @property {string} name
 * @property {string} job_id
 * @property {number} window_size
 * @property {number} stride
 * @property {number} page_count
 * @property {string} created_at
 * @property {string} updated_at
 */

/**
 * @typedef {Object} DocumentResultsList
 * @property {string} document_path
 * @property {string|null} active_result_id
 * @property {string|null} active_job_id
 * @property {SavedResultEntry[]} results
 */

export class ResultsPanel {
  /**
   * @param {HTMLElement} container
   * @param {Object} [options]
   * @param {function} [options.getDocumentPath] - Returns the current document path
   */
  constructor(container, options = {}) {
    this.container = container;
    this.getDocumentPath = options.getDocumentPath || (() => null);

    /** @type {DocumentResultsList|null} */
    this._catalog = null;
    /** @type {boolean} */
    this._loading = false;

    this._buildUI();
    this._attachListeners();
  }

  _buildUI() {
    this.container.innerHTML = `
      <div class="results-panel">
        <h2 class="results-panel-title">Saved Results</h2>
        <p class="results-panel-note">
          Save each phrase-length run, then switch between them without re-analyzing.
        </p>

        <div class="setting-group">
          <label class="setting-label" for="results-select">Result</label>
          <select id="results-select" class="results-select" aria-label="Saved results">
            <option value="">No saved results</option>
          </select>
        </div>

        <div class="setting-group">
          <label class="setting-label" for="results-name">Name</label>
          <input
            type="text"
            id="results-name"
            class="results-name-input"
            placeholder="e.g. 20 tokens"
            aria-label="Result name"
          />
        </div>

        <div id="results-meta" class="results-meta" hidden></div>

        <div class="results-actions">
          <button type="button" id="btn-result-load" class="btn-result-action" disabled>Load</button>
          <button type="button" id="btn-result-save" class="btn-result-action" disabled>Save</button>
          <button type="button" id="btn-result-save-as" class="btn-result-action" disabled>Save As…</button>
          <button type="button" id="btn-result-delete" class="btn-result-action btn-result-delete" disabled>Delete</button>
        </div>

        <div id="results-status" class="results-status" role="status" aria-live="polite"></div>

        <div class="results-export-section">
          <div class="results-export-title">Pipeline export</div>
          <div id="results-settings-yaml-export"></div>
        </div>
      </div>
    `;

    this._els = {
      select: this.container.querySelector("#results-select"),
      name: this.container.querySelector("#results-name"),
      meta: this.container.querySelector("#results-meta"),
      status: this.container.querySelector("#results-status"),
      btnLoad: this.container.querySelector("#btn-result-load"),
      btnSave: this.container.querySelector("#btn-result-save"),
      btnSaveAs: this.container.querySelector("#btn-result-save-as"),
      btnDelete: this.container.querySelector("#btn-result-delete"),
    };

    mountSettingsYamlExportControls(
      this.container.querySelector("#results-settings-yaml-export"),
      {
        getExportSettings: () => {
          const panel = window.importSettingsPanel;
          return panel?.getExportSettings?.() ?? null;
        },
      },
    );
  }

  _attachListeners() {
    this._els.select.addEventListener("change", () => {
      this._syncSelectionToForm();
    });

    this._els.btnLoad.addEventListener("click", () => {
      void this._handleLoad();
    });

    this._els.btnSave.addEventListener("click", () => {
      void this._handleSave();
    });

    this._els.btnSaveAs.addEventListener("click", () => {
      void this._handleSaveAs();
    });

    this._els.btnDelete.addEventListener("click", () => {
      void this._handleDelete();
    });
  }

  /** @returns {string|null} */
  _documentPath() {
    const path = this.getDocumentPath();
    return path || null;
  }

  /** @returns {SavedResultEntry|null} */
  _selectedEntry() {
    const resultId = this._els.select.value;
    if (!resultId || !this._catalog) return null;
    return this._catalog.results.find((entry) => entry.result_id === resultId) || null;
  }

  _syncSelectionToForm() {
    const entry = this._selectedEntry();
    if (!entry) {
      this._els.name.value = "";
      this._els.meta.hidden = true;
      this._els.meta.textContent = "";
      this._updateButtons(false);
      return;
    }

    this._els.name.value = entry.name;
    this._els.meta.hidden = false;
    this._els.meta.textContent =
      `${entry.window_size} tokens · stride ${entry.stride} · ${entry.page_count} pages`;
    this._updateButtons(true);
  }

  /** @param {boolean} hasSelection */
  _updateButtons(hasSelection) {
    const hasDocument = Boolean(this._documentPath());
    const hasCurrentJob = Boolean(window.currentJobId);

    this._els.btnLoad.disabled = !hasSelection || this._loading;
    this._els.btnSave.disabled = !hasSelection || !hasDocument || this._loading;
    this._els.btnSaveAs.disabled = !hasCurrentJob || !hasDocument || this._loading;
    this._els.btnDelete.disabled = !hasSelection || !hasDocument || this._loading;
  }

  /** @param {string} message @param {"info"|"error"} [kind] */
  _setStatus(message, kind = "info") {
    this._els.status.textContent = message;
    this._els.status.dataset.kind = kind;
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
    return "Request failed";
  }

  /**
   * Refresh the results list from the backend.
   * @param {string} [selectResultId]
   */
  async refresh(selectResultId) {
    const path = this._documentPath();
    const invoke = window.__TAURI__?.core?.invoke;
    if (!path || !invoke) {
      this._renderCatalog({ document_path: "", active_result_id: null, active_job_id: null, results: [] });
      return;
    }

    try {
      const catalog = await invoke("list_document_results", { path });
      this._renderCatalog(catalog, selectResultId);
      this._setStatus("");
    } catch (err) {
      console.warn("list_document_results failed:", err);
      this._setStatus(this._formatError(err), "error");
    }
  }

  /**
   * @param {DocumentResultsList} catalog
   * @param {string} [selectResultId]
   */
  _renderCatalog(catalog, selectResultId) {
    this._catalog = catalog;
    const select = this._els.select;
    select.innerHTML = "";

    if (!catalog.results.length) {
      const option = document.createElement("option");
      option.value = "";
      option.textContent = "No saved results";
      select.appendChild(option);
      this._syncSelectionToForm();
      this._updateButtons(false);
      return;
    }

    for (const entry of catalog.results) {
      const option = document.createElement("option");
      option.value = entry.result_id;
      option.textContent = entry.name;
      select.appendChild(option);
    }

    const preferred =
      selectResultId ||
      catalog.active_result_id ||
      catalog.results.find((entry) => entry.job_id === window.currentJobId)?.result_id ||
      catalog.results[0]?.result_id;

    if (preferred) {
      select.value = preferred;
    }

    this._syncSelectionToForm();
    this._updateButtons(Boolean(preferred));
  }

  async _handleLoad() {
    const entry = this._selectedEntry();
    const path = this._documentPath();
    const invoke = window.__TAURI__?.core?.invoke;
    if (!entry || !path || !invoke || this._loading) return;

    this._loading = true;
    this._updateButtons(true);
    this._setStatus(`Loading "${entry.name}"…`);

    try {
      await invoke("set_active_document_result", {
        path,
        resultId: entry.result_id,
      });
      await activateJob(entry.job_id, entry.page_count);
      await this.refresh(entry.result_id);
      this._setStatus(`Loaded "${entry.name}"`);
    } catch (err) {
      console.error("load result failed:", err);
      this._setStatus(this._formatError(err), "error");
    } finally {
      this._loading = false;
      this._updateButtons(Boolean(this._selectedEntry()));
    }
  }

  async _handleSave() {
    const entry = this._selectedEntry();
    const path = this._documentPath();
    const invoke = window.__TAURI__?.core?.invoke;
    const name = this._els.name.value.trim();
    if (!entry || !path || !invoke || !name || this._loading) return;

    this._loading = true;
    this._updateButtons(true);

    try {
      const catalog = await invoke("save_document_result", {
        path,
        resultId: entry.result_id,
        name,
      });
      this._renderCatalog(catalog, entry.result_id);
      this._setStatus(`Saved "${name}"`);
    } catch (err) {
      console.error("save result failed:", err);
      this._setStatus(this._formatError(err), "error");
    } finally {
      this._loading = false;
      this._updateButtons(Boolean(this._selectedEntry()));
    }
  }

  async _handleSaveAs() {
    const path = this._documentPath();
    const jobId = window.currentJobId;
    const invoke = window.__TAURI__?.core?.invoke;
    const name = this._els.name.value.trim();
    if (!path || !jobId || !invoke || !name || this._loading) return;

    this._loading = true;
    this._updateButtons(true);

    try {
      const catalog = await invoke("save_document_result_as", {
        path,
        jobId,
        name,
      });
      const created = catalog.results.find((entry) => entry.name === name);
      this._renderCatalog(catalog, created?.result_id);
      this._setStatus(`Saved as "${name}"`);
    } catch (err) {
      console.error("save result as failed:", err);
      this._setStatus(this._formatError(err), "error");
    } finally {
      this._loading = false;
      this._updateButtons(Boolean(this._selectedEntry()));
    }
  }

  async _handleDelete() {
    const entry = this._selectedEntry();
    const path = this._documentPath();
    const invoke = window.__TAURI__?.core?.invoke;
    if (!entry || !path || !invoke || this._loading) return;

    const confirmed = window.confirm(
      `Delete "${entry.name}"? This removes the saved result${entry.job_id === window.currentJobId ? " and its analysis data" : ""}.`,
    );
    if (!confirmed) return;

    this._loading = true;
    this._updateButtons(true);

    try {
      const catalog = await invoke("delete_document_result", {
        path,
        resultId: entry.result_id,
      });
      this._renderCatalog(catalog);
      if (entry.job_id === window.currentJobId) {
        window.currentJobId = null;
      }
      this._setStatus(`Deleted "${entry.name}"`);
    } catch (err) {
      console.error("delete result failed:", err);
      this._setStatus(this._formatError(err), "error");
    } finally {
      this._loading = false;
      this._updateButtons(Boolean(this._selectedEntry()));
    }
  }
}
