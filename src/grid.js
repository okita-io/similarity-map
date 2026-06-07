// Grid Renderer — manages a flowing flex-wrap layout of 20×20 page canvases.
// Listens for similarity-map:page-ready events and progressively populates cells.

const CELL_SIZE = 20; // pixels per page cell (native resolution)
const MAX_PAGES = 300;
const DEFAULT_BG = "rgb(34, 34, 42)"; // visible baseline for processed/empty pages

/**
 * GridRenderer manages the page grid container, creating canvas elements
 * and drawing ImageBitmaps as page-ready events arrive.
 */
export class GridRenderer {
  /**
   * @param {HTMLElement} container - The #grid-container element
   */
  constructor(container) {
    this._container = container;
    /** @type {Map<number, HTMLCanvasElement>} page (1-based) → canvas element */
    this._canvases = new Map();
    this._pageCount = 0;
    this._unlisten = null;
  }

  /**
   * Initialise the grid with empty (transparent) canvas cells.
   * @param {number} pageCount - Total number of pages in the document
   * @param {object|null} [scopeManifest] - RF scope manifest (one page per act)
   */
  initGrid(pageCount, scopeManifest = null) {
    this._pageCount = pageCount;
    this._scopeManifest = scopeManifest;
    // Clear any existing content
    this._container.innerHTML = "";
    this._canvases.clear();

    // Cap at MAX_PAGES (300) per requirement 14.6
    const displayCount = Math.min(pageCount, MAX_PAGES);
    console.info(`[grid] initGrid pageCount=${pageCount} displayCount=${displayCount}`);

    for (let i = 1; i <= displayCount; i++) {
      const canvas = document.createElement("canvas");
      canvas.width = CELL_SIZE;
      canvas.height = CELL_SIZE;
      canvas.dataset.page = String(i);
      canvas.setAttribute("aria-label", `Page ${i}`);
      if (scopeManifest?.acts?.[i - 1]) {
        const act = scopeManifest.acts[i - 1];
        canvas.classList.add("grid-act-page");
        canvas.dataset.act = String(act.act);
        canvas.title = `Act ${act.act} (page ${i})`;
      }
      // Fill with a baseline background so "processed but empty" is visible even
      // if no page-ready events are received (or pixels are fully masked out).
      const ctx = canvas.getContext("2d");
      ctx.fillStyle = DEFAULT_BG;
      ctx.fillRect(0, 0, CELL_SIZE, CELL_SIZE);
      this._container.appendChild(canvas);
      this._canvases.set(i, canvas);
    }

    // Show overflow indicator if pages exceed the maximum
    if (pageCount > MAX_PAGES) {
      const overflow = document.createElement("div");
      overflow.className = "grid-overflow";
      overflow.textContent = `+${pageCount - MAX_PAGES} pages not shown`;
      this._container.parentElement.appendChild(overflow);
    }

  }

  /**
   * Decode a base64-encoded RGBA buffer and draw it to the page's canvas.
   * @param {number} page - 1-based page number
   * @param {string} canvasRgbaB64 - Base64-encoded 1600-byte RGBA array
   */
  async updatePage(page, canvasRgbaB64) {
    const canvas = this._canvases.get(page);
    if (!canvas) return;

    try {
      // Decode base64 → Uint8Array
      const binary = atob(canvasRgbaB64);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) {
        bytes[i] = binary.charCodeAt(i);
      }

      // Create ImageData from the raw RGBA bytes
      const imageData = new ImageData(
        new Uint8ClampedArray(bytes.buffer),
        CELL_SIZE,
        CELL_SIZE
      );

      const ctx = canvas.getContext("2d");
      ctx.clearRect(0, 0, CELL_SIZE, CELL_SIZE);

      // Prefer ImageBitmap when supported, otherwise putImageData.
      try {
        if (typeof createImageBitmap === "function") {
          const bitmap = await createImageBitmap(imageData);
          ctx.drawImage(bitmap, 0, 0);
          bitmap.close?.();
          return;
        }
      } catch (e) {
        console.warn("createImageBitmap failed; falling back to putImageData", e);
      }

      ctx.putImageData(imageData, 0, 0);
    } catch (err) {
      console.error("[grid] updatePage failed", { page, err });
    }
  }

  /**
   * Set the grid gap in pixels (0–4, default 1).
   * @param {number} gap
   */
  setGap(gap) {
    const clamped = Math.max(0, Math.min(4, Math.round(gap)));
    document.documentElement.style.setProperty("--grid-gap", `${clamped}px`);
  }

  /**
   * Start listening for page-ready events from the Tauri backend.
   */
  async startListening() {
    if (this._unlisten) return;

    // Use the global Tauri API (no bundler)
    const tauriEvent = window.__TAURI__ && window.__TAURI__.event;
    if (!tauriEvent) {
      console.warn("Tauri event API not available — page-ready events will not be received.");
      return;
    }

    this._unlisten = await tauriEvent.listen("similarity-map:page-ready", (event) => {
      const { page, canvas_rgba_b64, job_id } = event.payload;
      const activeJob = window.currentJobId;
      if (activeJob && job_id && job_id !== activeJob) return;

      console.info(`[grid] page-ready job_id=${job_id} page=${page}`);

      // Draw within the next animation frame for smooth progressive fill
      requestAnimationFrame(() => {
        void this.updatePage(page, canvas_rgba_b64);
      });
    });
  }

  /**
   * Stop listening for page-ready events.
   */
  stopListening() {
    if (this._unlisten) {
      this._unlisten();
      this._unlisten = null;
    }
  }

  /**
   * Clean up resources.
   */
  destroy() {
    this.stopListening();
    this._container.innerHTML = "";
    this._canvases.clear();
  }
}
