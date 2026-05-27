// Tolerance Mask — per-page 1-bit alpha mask computed entirely on the frontend.
// Updates all pages when the tolerance slider changes with no backend IPC.
// Target: < 16ms for 300 pages (400 pixels each = 120,000 comparisons).

const CELL_SIZE = 20; // 20×20 grid per page
const PIXELS_PER_PAGE = CELL_SIZE * CELL_SIZE; // 400

/**
 * ToleranceMask manages per-page alpha masks based on a tolerance threshold.
 *
 * For each sub-cell (pixel in the 20×20 grid), the mask checks if the highest
 * sim_to_centroid value exceeds the current tolerance. If yes, the pixel remains
 * visible (alpha preserved). If no, the pixel is made transparent (alpha = 0).
 *
 * This is a frontend-only operation — no IPC to the backend.
 */
export class ToleranceMask {
  constructor() {
    /**
     * Per-page similarity data: page (1-based) → Float32Array(400)
     * Each entry holds the highest sim_to_centroid for that sub-cell.
     * A value of 0 means no cluster data for that sub-cell.
     * @type {Map<number, Float32Array>}
     */
    this._simData = new Map();

    /**
     * Per-page original (unmasked) ImageData pixels stored as Uint8ClampedArray.
     * page (1-based) → Uint8ClampedArray(1600) — the raw RGBA from the backend.
     * @type {Map<number, Uint8ClampedArray>}
     */
    this._originalPixels = new Map();

    /** Current tolerance value (0.75–1.00) */
    this._tolerance = 0.88;
  }

  /**
   * Get the current tolerance value.
   * @returns {number}
   */
  get tolerance() {
    return this._tolerance;
  }

  /**
   * Store sub-cell similarity data for a page.
   * This should be called when page data arrives (e.g., from a metadata event
   * or derived from the canvas data).
   *
   * @param {number} page - 1-based page number
   * @param {Float32Array|number[]} simValues - 400 values, one per sub-cell (row-major).
   *   Each value is the highest sim_to_centroid for that sub-cell (0 if empty).
   */
  setPageSimData(page, simValues) {
    if (simValues.length !== PIXELS_PER_PAGE) {
      throw new Error(
        `Expected ${PIXELS_PER_PAGE} similarity values for page ${page}, got ${simValues.length}`
      );
    }
    // Store as Float32Array for fast iteration
    const data =
      simValues instanceof Float32Array
        ? simValues
        : new Float32Array(simValues);
    this._simData.set(page, data);
  }

  /**
   * Store the original (unmasked) RGBA pixel data for a page.
   * This preserves the backend-rendered canvas so we can re-apply the mask
   * without re-fetching.
   *
   * @param {number} page - 1-based page number
   * @param {Uint8ClampedArray} rgbaPixels - 1600 bytes (20×20×4 RGBA, row-major)
   */
  setOriginalPixels(page, rgbaPixels) {
    if (rgbaPixels.length !== PIXELS_PER_PAGE * 4) {
      throw new Error(
        `Expected ${PIXELS_PER_PAGE * 4} bytes for page ${page}, got ${rgbaPixels.length}`
      );
    }
    // Copy so we don't hold a reference to a potentially reused buffer
    this._originalPixels.set(page, new Uint8ClampedArray(rgbaPixels));
  }

  /**
   * Compute the 1-bit alpha mask for a single page at the current tolerance.
   * Returns a Uint8Array of 400 entries: 1 = visible, 0 = transparent.
   *
   * @param {number} page - 1-based page number
   * @returns {Uint8Array|null} The mask, or null if no sim data for this page.
   */
  computeMask(page) {
    const simData = this._simData.get(page);
    if (!simData) return null;

    const mask = new Uint8Array(PIXELS_PER_PAGE);
    const tol = this._tolerance;

    for (let i = 0; i < PIXELS_PER_PAGE; i++) {
      // Pixel is visible if highest sim_to_centroid > tolerance
      mask[i] = simData[i] > tol ? 1 : 0;
    }

    return mask;
  }

  /**
   * Apply the tolerance mask to a canvas element for a given page.
   * Reads the original pixels, applies the mask, and writes to the canvas.
   *
   * @param {number} page - 1-based page number
   * @param {HTMLCanvasElement} canvas - The 20×20 canvas element for this page
   * @returns {boolean} true if mask was applied, false if data was missing
   */
  applyToCanvas(page, canvas) {
    const original = this._originalPixels.get(page);
    if (!original) return false;

    const mask = this.computeMask(page);
    const ctx = canvas.getContext("2d");
    const imageData = ctx.createImageData(CELL_SIZE, CELL_SIZE);
    const pixels = imageData.data;

    if (!mask) {
      // No sim data — just draw the original unmodified
      pixels.set(original);
    } else {
      // Apply mask: copy RGB from original, set alpha based on mask
      for (let i = 0; i < PIXELS_PER_PAGE; i++) {
        const offset = i * 4;
        pixels[offset] = original[offset]; // R
        pixels[offset + 1] = original[offset + 1]; // G
        pixels[offset + 2] = original[offset + 2]; // B
        // Alpha: keep original alpha if mask says visible, else 0
        pixels[offset + 3] = mask[i] ? original[offset + 3] : 0;
      }
    }

    ctx.putImageData(imageData, 0, 0);
    return true;
  }

  /**
   * Update the tolerance value and re-apply the mask to all provided canvases.
   * Designed to be called on slider drag — targets < 16ms for 300 pages.
   *
   * @param {number} newTolerance - New tolerance value (0.75–1.00)
   * @param {Map<number, HTMLCanvasElement>} canvases - page → canvas map
   */
  updateTolerance(newTolerance, canvases) {
    this._tolerance = Math.max(0.75, Math.min(1.0, newTolerance));
    this._applyAll(canvases);
  }

  /**
   * Re-apply the current mask to all pages that have both sim data and
   * original pixels stored. Call this after tolerance changes.
   *
   * @param {Map<number, HTMLCanvasElement>} canvases - page → canvas map
   */
  _applyAll(canvases) {
    for (const [page, canvas] of canvases) {
      this.applyToCanvas(page, canvas);
    }
  }

  /**
   * Derive similarity data from the original RGBA pixels.
   * This is a fallback approach: if the alpha channel of the original render
   * encodes visibility (alpha > 0 means the pixel has a cluster above the
   * backend's threshold), we can use it as a binary signal. However, for
   * proper tolerance filtering we need the actual sim_to_centroid values.
   *
   * This method extracts a binary presence signal (1.0 if alpha > 0, else 0.0)
   * which can serve as placeholder data until real sim values are provided.
   *
   * @param {number} page - 1-based page number
   * @returns {Float32Array} 400 values derived from alpha channel
   */
  deriveSimDataFromAlpha(page) {
    const original = this._originalPixels.get(page);
    if (!original) {
      return new Float32Array(PIXELS_PER_PAGE);
    }

    const simData = new Float32Array(PIXELS_PER_PAGE);
    for (let i = 0; i < PIXELS_PER_PAGE; i++) {
      // If the pixel has alpha > 0, it was rendered by the backend above its threshold.
      // Use 1.0 as a placeholder sim value (always above any tolerance).
      simData[i] = original[i * 4 + 3] > 0 ? 1.0 : 0.0;
    }

    return simData;
  }

  /**
   * Store original pixels and auto-derive sim data from alpha channel.
   * Convenience method for when real sim metadata is not yet available.
   *
   * @param {number} page - 1-based page number
   * @param {Uint8ClampedArray} rgbaPixels - 1600 bytes (20×20×4 RGBA)
   */
  setPageData(page, rgbaPixels) {
    this.setOriginalPixels(page, rgbaPixels);
    const simData = this.deriveSimDataFromAlpha(page);
    this.setPageSimData(page, simData);
  }

  /**
   * Check if a page has stored data.
   * @param {number} page - 1-based page number
   * @returns {boolean}
   */
  hasPage(page) {
    return this._originalPixels.has(page);
  }

  /**
   * Get the number of pages with stored data.
   * @returns {number}
   */
  get pageCount() {
    return this._originalPixels.size;
  }

  /**
   * Remove all stored data for a page.
   * @param {number} page - 1-based page number
   */
  removePage(page) {
    this._simData.delete(page);
    this._originalPixels.delete(page);
  }

  /**
   * Clear all stored data.
   */
  clear() {
    this._simData.clear();
    this._originalPixels.clear();
  }
}
