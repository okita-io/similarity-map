// Shared job activation — wires grid, display settings, and backend restore.

/**
 * Activate a completed analysis job: re-init grid, restore session, load clusters.
 * @param {string} jobId
 * @param {number} pageCount
 * @returns {Promise<void>}
 */
export async function activateJob(jobId, pageCount) {
  window.currentJobId = jobId;

  const grid = window.gridRenderer;
  if (grid && pageCount > 0) {
    grid.initGrid(pageCount);
  }

  const display = window.displaySettingsPanel;
  if (display) {
    display.setJobId(jobId);
    display.setAllPages(Array.from({ length: pageCount }, (_, i) => i + 1));
    if (grid) {
      display.setCanvases(grid._canvases);
    }
  }

  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;

  try {
    const handle = await invoke("restore_session", { jobId });
    if (display && handle?.display_state) {
      display.restoreState(handle.display_state);
    }
  } catch (err) {
    console.error("restore_session failed:", err);
    throw err;
  }

  try {
    const registry = await invoke("get_cluster_registry", { jobId });
    if (display && registry?.clusters) {
      display.setClusters(Object.values(registry.clusters));
    }
  } catch (err) {
    console.warn("get_cluster_registry failed:", err);
  }

  const resultsPanel = window.resultsPanel;
  if (resultsPanel) {
    await resultsPanel.refresh();
  }
}
