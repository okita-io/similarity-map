// Similarity Map - Main entry point
// Tauri 2 IPC will be accessed via window.__TAURI__

import { GridRenderer } from "./grid.js";
import { ZoomController } from "./zoom.js";
import { ImportSettingsPanel } from "./import-settings.js";
import { DisplaySettingsPanel } from "./display-settings.js";
import { ResultsPanel } from "./results-panel.js";
import { DetailPanel } from "./detail-panel.js";
import { NavigationController } from "./navigation.js";
import { ModelDownloadUI } from "./model-download.js";
import { LogPanel } from "./log-panel.js";
import { TextPreviewPanel } from "./text-preview.js";

document.addEventListener("DOMContentLoaded", () => {
  // Init log panel first so subsequent module logs are captured.
  const logContainer = document.getElementById("log-panel-container");
  const logPanel = new LogPanel(logContainer);
  window.logPanel = logPanel;

  const tauri = window.__TAURI__;
  logPanel.log(
    "info",
    "ui",
    `Tauri globals: ${tauri ? Object.keys(tauri).join(", ") : "none"}`,
  );

  const container = document.getElementById("grid-container");
  const gridRenderer = new GridRenderer(container);

  // Zoom controller — cell size scales via --zoom; #main scrolls the layout
  const zoomController = new ZoomController(container);

  // Import Settings Panel
  const importSettingsContainer = document.getElementById("import-settings-container");
  const importSettingsPanel = new ImportSettingsPanel(importSettingsContainer);

  // Model Download UI — blocks Analyze until model is available
  const modelDownloadUI = new ModelDownloadUI(importSettingsContainer, {
    onModelReady: () => {
      // Enable the Analyze button once model is available
      const btnAnalyze = importSettingsContainer.querySelector("#btn-analyze");
      if (btnAnalyze && btnAnalyze.dataset.modelBlocked === "true") {
        btnAnalyze.disabled = false;
        delete btnAnalyze.dataset.modelBlocked;
      }
    },
    onModelUnavailable: () => {
      // Disable the Analyze button until model is ready
      const btnAnalyze = importSettingsContainer.querySelector("#btn-analyze");
      if (btnAnalyze) {
        btnAnalyze.disabled = true;
        btnAnalyze.dataset.modelBlocked = "true";
      }
    },
  });

  // Display Settings Panel
  const displaySettingsContainer = document.getElementById("display-settings-container");
  const displaySettingsPanel = new DisplaySettingsPanel(displaySettingsContainer, {
    onPagesUpdated: async (pages) => {
      for (const pageCanvas of pages) {
        await gridRenderer.updatePage(pageCanvas.page, pageCanvas.canvas_rgba_b64);
      }
    },
  });

  // Saved Results Panel
  const resultsPanelContainer = document.getElementById("results-panel-container");
  const resultsPanel = new ResultsPanel(resultsPanelContainer, {
    getDocumentPath: () =>
      importSettingsPanel.filePath || null,
  });

  // Navigation Controller — counterpart link navigation
  const navigationController = new NavigationController(container);

  // Detail Panel — side panel for cell click detail data
  const detailPanelContainer = document.getElementById("detail-panel-container");
  const detailPanel = new DetailPanel(detailPanelContainer, {
    getZoom: () => zoomController ? zoomController.getZoom() : 1,
    getTolerance: () => displaySettingsPanel ? displaySettingsPanel.getTolerance() : 0.88,
    getJobId: () => window.currentJobId || null,
    onCounterpartClick: (page, subCellRow, subCellCol) => {
      navigationController.navigateTo(page, subCellRow, subCellCol);
    }
  });
  detailPanel.attachToGrid(container);

  const textPreviewContainer = document.getElementById("text-preview-container");
  const textPreviewPanel = new TextPreviewPanel(textPreviewContainer, {
    onHighlightClick: (page, clusterId) => {
      window.textPreviewPanel?.setActiveCluster(clusterId);
      navigationController.navigateToPage(page);
    },
  });

  // Expose globally for other modules and debugging
  window.gridRenderer = gridRenderer;
  window.zoomController = zoomController;
  window.importSettingsPanel = importSettingsPanel;
  window.displaySettingsPanel = displaySettingsPanel;
  window.resultsPanel = resultsPanel;
  window.navigationController = navigationController;
  window.detailPanel = detailPanel;
  window.textPreviewPanel = textPreviewPanel;
  window.modelDownloadUI = modelDownloadUI;

  // Start listening for page-ready events from the backend
  gridRenderer.startListening();

  console.log("Similarity Map initialized");
});
