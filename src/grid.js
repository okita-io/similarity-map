// Grid Renderer — manages the 10-column CSS grid of 20×20 page canvases.
// Listens for similarity-map:page-ready events and progressively populates cells.

const CELL_SIZE = 20; // pixels per page cell (native resolution)
const COLUMNS = 10;
const MAX_PAGES = 300;
const SMOOTH_THRESHOLD = 100; // px on longest dimension to switch rendering mode

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
    /** @type {Map<number, ImageBitmap>} page (1-based) → current ImageBitmap */
    this._bitmaps = new Map();
    /** @type {Map<number, HTMLCanvasElement>} page (1-based) → canvas element */
    this._canvases = new Map();
    this._pageCount = 0;
    this._unlisten = null;
  }

  /**
   * Initialise the grid with empty (transparent) canvas cells.
   * @param {number} pageCount - Total number of pages in the document
   */
  initGrid(pageCount) {
    this._pageCount = pageCount;
    // Clear any existing content
    this._releaseAllBitmaps();
    this._container.innerHTML = "";
    this._canvases.clear();

    // Cap at MAX_PAGES (300) per requirement 14.6
    const displayCount = Math.min(pageCount, MAX_PAGES);

    for (let i = 1; i <= displayCount; i++) {
      const canvas = document.createElement("canvas");
      canvas.width = CELL_SIZE;
      canvas.height = CELL_SIZE;
      canvas.dataset.page = String(i);
      canvas.setAttribute("aria-label", `Page ${i}`);
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

    this._updateRenderingMode();
  }

  /**
   * Decode a base64-encoded RGBA buffer and draw it to the page's canvas.
   * @param {number} page - 1-based page number
   * @param {string} canvasRgbaB64 - Base64-encoded 1600-byte RGBA array
   */
  async updatePage(page, canvasRgbaB64) {
    const canvas = this._canvases.get(page);
    if (!canvas) return;

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

    // Create ImageBitmap (release previous one first)
    const oldBitmap = this._bitmaps.get(page);
    if (oldBitmap) {
      oldBitmap.close();
    }

    const bitmap = await createImageBitmap(imageData);
    this._bitmaps.set(page, bitmap);

    // Draw to canvas
    const ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, CELL_SIZE, CELL_SIZE);
    ctx.drawImage(bitmap, 0, 0);
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
   * Check the rendered cell size and toggle image-rendering mode.
   * Call this after zoom or resize changes.
   */
  _updateRenderingMode() {
    const firstCanvas = this._canvases.get(1);
    if (!firstCanvas) return;

    const rect = firstCanvas.getBoundingClientRect();
    const longest = Math.max(rect.width, rect.height);

    if (longest >= SMOOTH_THRESHOLD) {
      this._container.classList.add("smooth-rendering");
    } else {
      this._container.classList.remove("smooth-rendering");
    }
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
      const { page, canvas_rgba_b64 } = event.payload;
      // Draw within the next animation frame for smooth progressive fill
      requestAnimationFrame(() => {
        this.updatePage(page, canvas_rgba_b64);
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
   * Release all stored ImageBitmaps to free memory.
   */
  _releaseAllBitmaps() {
    for (const bitmap of this._bitmaps.values()) {
      bitmap.close();
    }
    this._bitmaps.clear();
  }

  /**
   * Clean up resources.
   */
  destroy() {
    this.stopListening();
    this._releaseAllBitmaps();
    this._container.innerHTML = "";
    this._canvases.clear();
  }
}
