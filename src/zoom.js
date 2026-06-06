// Zoom Controller — scales page cells via CSS variables (no transform scaling).
// Cell display size = --cell-size × --zoom; #main scrolls the laid-out content.

const MIN_ZOOM = 1;
const MAX_ZOOM = 10;
const ZOOM_STEP = 0.5;

/**
 * ZoomController updates the --zoom CSS variable on the grid container.
 * Cells grow in place so scroll extents match the visible layout inside #main.
 */
export class ZoomController {
  /**
   * @param {HTMLElement} container - The #grid-container element
   * @param {object} [options]
   * @param {function} [options.onZoomChange] - Callback invoked after zoom level changes
   */
  constructor(container, options = {}) {
    this._container = container;
    this._zoom = 1;
    this._onZoomChange = options.onZoomChange || null;

    this._handleKeyDown = this._handleKeyDown.bind(this);
    this._handleWheel = this._handleWheel.bind(this);

    this._attachListeners();
    this._applyZoom();
  }

  /** Current zoom level. */
  get zoom() {
    return this._zoom;
  }

  /** @returns {number} Current zoom level (same as `.zoom`). */
  getZoom() {
    return this._zoom;
  }

  /**
   * Set the zoom level. Clamped to [MIN_ZOOM, MAX_ZOOM].
   * @param {number} level
   */
  setZoom(level) {
    const clamped = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, level));
    if (clamped === this._zoom) return;
    this._zoom = clamped;
    this._applyZoom();
    if (this._onZoomChange) {
      this._onZoomChange(this._zoom);
    }
  }

  /** Zoom in by one step. */
  zoomIn() {
    this.setZoom(this._zoom + ZOOM_STEP);
  }

  /** Zoom out by one step. */
  zoomOut() {
    this.setZoom(this._zoom - ZOOM_STEP);
  }

  /** Reset zoom to 1x. */
  resetZoom() {
    this.setZoom(1);
  }

  /** @private */
  _applyZoom() {
    this._container.style.setProperty("--zoom", String(this._zoom));
  }

  /**
   * Handle keyboard shortcuts for zoom.
   * Ctrl+= / Ctrl++ : zoom in
   * Ctrl+- : zoom out
   * Ctrl+0 : reset
   * @private
   */
  _handleKeyDown(e) {
    if (!e.ctrlKey && !e.metaKey) return;

    if (e.key === "=" || e.key === "+") {
      e.preventDefault();
      this.zoomIn();
    } else if (e.key === "-") {
      e.preventDefault();
      this.zoomOut();
    } else if (e.key === "0") {
      e.preventDefault();
      this.resetZoom();
    }
  }

  /**
   * Handle Ctrl+wheel for zoom.
   * @private
   */
  _handleWheel(e) {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();

    if (e.deltaY < 0) {
      this.zoomIn();
    } else if (e.deltaY > 0) {
      this.zoomOut();
    }
  }

  /** @private */
  _attachListeners() {
    document.addEventListener("keydown", this._handleKeyDown);
    const scrollParent = this._container.closest("#main") || this._container.parentElement;
    if (scrollParent) {
      scrollParent.addEventListener("wheel", this._handleWheel, { passive: false });
    }
  }

  /** Remove event listeners. */
  destroy() {
    document.removeEventListener("keydown", this._handleKeyDown);
    const scrollParent = this._container.closest("#main") || this._container.parentElement;
    if (scrollParent) {
      scrollParent.removeEventListener("wheel", this._handleWheel);
    }
  }
}
