// Dither Engine — applies spatial dithering patterns to sub-cells at high zoom.
// When a cell is rendered at ≥ 100×200px, multi-cluster sub-cells display
// individual cluster colors via dithering instead of a weighted blend.

/**
 * Minimum rendered cell dimensions (in CSS pixels) to activate dithering.
 * Below this threshold, the weighted color blend is used instead.
 */
const DITHER_WIDTH_THRESHOLD = 100;
const DITHER_HEIGHT_THRESHOLD = 200;

/**
 * DitherEngine determines when spatial dithering should be active and
 * selects the appropriate dithering pattern for a given sub-cell based
 * on the number of visible clusters it contains.
 *
 * Dithering replaces the blended color with individual cluster colors
 * at the pixel level when zoomed in enough. Each pixel within a rendered
 * sub-cell is assigned to one cluster via a deterministic pattern.
 */
export class DitherEngine {
  constructor() {
    /**
     * Cache of pattern functions keyed by cluster count.
     * @type {Map<number, function(number, number): number>}
     */
    this._patterns = new Map();
    this._patterns.set(2, DitherEngine.checkerboard);
    this._patterns.set(3, DitherEngine.thirdsScatter);
    this._patterns.set(4, DitherEngine.quadrantTile);
  }

  /**
   * Determine whether dithering should be active based on the rendered
   * cell dimensions.
   * @param {number} cellWidth - Rendered width of a cell in CSS pixels
   * @param {number} cellHeight - Rendered height of a cell in CSS pixels
   * @returns {boolean} True if dithering should be applied
   */
  shouldDither(cellWidth, cellHeight) {
    return cellWidth >= DITHER_WIDTH_THRESHOLD && cellHeight >= DITHER_HEIGHT_THRESHOLD;
  }

  /**
   * Get the cluster index for a pixel position within a sub-cell.
   * The returned index selects which cluster color to display at that pixel.
   *
   * @param {number} pxRow - Pixel row within the rendered sub-cell (0-based)
   * @param {number} pxCol - Pixel column within the rendered sub-cell (0-based)
   * @param {number} clusterCount - Number of visible clusters in the sub-cell (2–8)
   * @returns {number} Index into the cluster array (0 to clusterCount-1)
   */
  getClusterIndex(pxRow, pxCol, clusterCount) {
    if (clusterCount <= 1) {
      return 0;
    }

    const patternFn = this._patterns.get(clusterCount);
    if (patternFn) {
      return patternFn(pxRow, pxCol);
    }

    // 5–8 clusters: modulo scatter
    return DitherEngine.moduloScatter(pxRow, pxCol, clusterCount);
  }

  /**
   * Render a sub-cell's pixels using dithering. Returns an array of RGBA
   * values for each pixel in the sub-cell area.
   *
   * @param {Array<{color: [number,number,number,number]}>} clusters
   *   Visible clusters sorted by sim_to_centroid desc, each with an RGBA color.
   *   Maximum 8 entries.
   * @param {number} width - Width of the sub-cell area in pixels
   * @param {number} height - Height of the sub-cell area in pixels
   * @returns {Uint8ClampedArray} RGBA pixel data (width × height × 4 bytes)
   */
  renderDithered(clusters, width, height) {
    const pixelCount = width * height;
    const data = new Uint8ClampedArray(pixelCount * 4);
    const clusterCount = clusters.length;

    if (clusterCount === 0) {
      // All transparent
      return data;
    }

    if (clusterCount === 1) {
      // Single cluster — fill with its color
      const [r, g, b, a] = clusters[0].color;
      for (let i = 0; i < pixelCount; i++) {
        const offset = i * 4;
        data[offset] = r;
        data[offset + 1] = g;
        data[offset + 2] = b;
        data[offset + 3] = a;
      }
      return data;
    }

    // Multiple clusters — apply dithering pattern
    for (let row = 0; row < height; row++) {
      for (let col = 0; col < width; col++) {
        const idx = this.getClusterIndex(row, col, clusterCount);
        const [r, g, b, a] = clusters[idx].color;
        const offset = (row * width + col) * 4;
        data[offset] = r;
        data[offset + 1] = g;
        data[offset + 2] = b;
        data[offset + 3] = a;
      }
    }

    return data;
  }

  /**
   * Compute the blended color for a sub-cell when below the dithering
   * threshold. Uses similarity-weighted blending in linear RGB space.
   *
   * @param {Array<{color: [number,number,number,number], weight: number}>} clusters
   *   Visible clusters with pre-computed RGBA colors and weights (sim^gamma).
   * @returns {[number, number, number, number]} Blended RGBA color
   */
  blendWeighted(clusters) {
    if (clusters.length === 0) {
      return [0, 0, 0, 0];
    }

    if (clusters.length === 1) {
      return clusters[0].color;
    }

    let totalWeight = 0;
    let r = 0;
    let g = 0;
    let b = 0;

    for (const cluster of clusters) {
      const w = cluster.weight;
      totalWeight += w;
      // Blend in linear space (colors assumed to already be linear RGB)
      r += w * cluster.color[0];
      g += w * cluster.color[1];
      b += w * cluster.color[2];
    }

    if (totalWeight === 0) {
      return [0, 0, 0, 0];
    }

    return [
      Math.round(r / totalWeight),
      Math.round(g / totalWeight),
      Math.round(b / totalWeight),
      255
    ];
  }

  /**
   * Determine the rendering mode and produce pixel data for a sub-cell.
   *
   * @param {Array<{color: [number,number,number,number], weight: number}>} clusters
   *   Visible clusters sorted by sim_to_centroid desc (max 8).
   * @param {number} subCellWidth - Rendered width of the sub-cell in pixels
   * @param {number} subCellHeight - Rendered height of the sub-cell in pixels
   * @param {number} cellWidth - Rendered width of the full cell in pixels
   * @param {number} cellHeight - Rendered height of the full cell in pixels
   * @returns {{mode: string, data: Uint8ClampedArray|[number,number,number,number]}}
   *   mode: "dither" or "blend"
   *   data: pixel array for dither mode, single RGBA for blend mode
   */
  renderSubCell(clusters, subCellWidth, subCellHeight, cellWidth, cellHeight) {
    if (clusters.length === 0) {
      return { mode: "blend", data: [0, 0, 0, 0] };
    }

    if (this.shouldDither(cellWidth, cellHeight) && clusters.length > 1) {
      return {
        mode: "dither",
        data: this.renderDithered(clusters, subCellWidth, subCellHeight)
      };
    }

    return {
      mode: "blend",
      data: this.blendWeighted(clusters)
    };
  }

  // --- Static pattern functions ---

  /**
   * Checkerboard pattern for 2 clusters.
   * Alternates between cluster 0 and 1 in a checkerboard.
   * @param {number} pxRow - Pixel row
   * @param {number} pxCol - Pixel column
   * @returns {number} Cluster index (0 or 1)
   */
  static checkerboard(pxRow, pxCol) {
    return (pxRow + pxCol) % 2;
  }

  /**
   * Thirds scatter pattern for 3 clusters.
   * Distributes pixels among 3 clusters using a scatter formula.
   * @param {number} pxRow - Pixel row
   * @param {number} pxCol - Pixel column
   * @returns {number} Cluster index (0, 1, or 2)
   */
  static thirdsScatter(pxRow, pxCol) {
    return (pxRow * 3 + pxCol * 7) % 3;
  }

  /**
   * 2×2 quadrant tile pattern for 4 clusters.
   * Assigns each pixel to one of 4 clusters based on its position in a 2×2 tile.
   * @param {number} pxRow - Pixel row
   * @param {number} pxCol - Pixel column
   * @returns {number} Cluster index (0, 1, 2, or 3)
   */
  static quadrantTile(pxRow, pxCol) {
    return (pxRow % 2) * 2 + (pxCol % 2);
  }

  /**
   * Modulo scatter pattern for 5–8 clusters.
   * Distributes pixels among N clusters using a scatter formula with primes.
   * @param {number} pxRow - Pixel row
   * @param {number} pxCol - Pixel column
   * @param {number} n - Number of clusters (5–8)
   * @returns {number} Cluster index (0 to n-1)
   */
  static moduloScatter(pxRow, pxCol, n) {
    return (pxRow * 11 + pxCol * 7) % n;
  }
}
