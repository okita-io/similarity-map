// Similarity Map - Main entry point
// Tauri 2 IPC will be accessed via window.__TAURI__

import { GridRenderer } from "./grid.js";
import { ZoomController } from "./zoom.js";
import { ImportSettingsPanel } from "./import-settings.js";
import { DisplaySettingsPanel } from "./display-settings.js";
import { DetailPanel } from "./detail-panel.js";
import { NavigationController } from "./navigation.js";
import { ModelDownloadUI } from "./model-download.js";

document.addEventListener("DOMContentLoaded", () => {
  const container = document.getElementById("grid-container");
  const gridRenderer = new GridRenderer(container);

  // Zoom controller — CSS-only scaling, no bitmap allocation on zoom/scroll
  const zoomController = new ZoomController(container, {
    onZoomChange: () => {
      gridRenderer._updateRenderingMode();
    }
  });

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
  const displaySettingsPanel = new DisplaySettingsPanel(displaySettingsContainer);

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

  // Expose globally for other modules and debugging
  window.gridRenderer = gridRenderer;
  window.zoomController = zoomController;
  window.importSettingsPanel = importSettingsPanel;
  window.displaySettingsPanel = displaySettingsPanel;
  window.navigationController = navigationController;
  window.detailPanel = detailPanel;
  window.modelDownloadUI = modelDownloadUI;

  // Start listening for page-ready events from the backend
  gridRenderer.startListening();

  // Handle window resize to update image-rendering mode (CSS scaling only, req 29.4)
  const resizeObserver = new ResizeObserver(() => {
    gridRenderer._updateRenderingMode();
  });
  resizeObserver.observe(container);

  console.log("Similarity Map initialized");
});
