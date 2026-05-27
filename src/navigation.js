// Counterpart Navigation — scrolls grid to target page, pulses the macro-cell,
// and highlights the corresponding sub-cell until the next click.
// Requirement 25.3

const PULSE_DURATION_MS = 1500;
const CELL_SIZE = 20; // native canvas resolution (matches grid.js)

/**
 * NavigationController handles counterpart link clicks from the detail panel.
 * It scrolls the grid to the target page, applies a pulse animation to the
 * macro-cell canvas, and draws a persistent sub-cell outline overlay.
 */
export class NavigationController {
  /**
   * @param {HTMLElement} gridContainer - The #grid-container element
   */
  constructor(gridContainer) {
    this._gridContainer = gridContainer;
    /** @type {HTMLElement|null} Currently highlighted overlay element */
    this._activeOverlay = null;
    /** @type {HTMLCanvasElement|null} Currently pulsing canvas */
    this._pulsingCanvas = null;

    this._dismissHighlight = this._dismissHighlight.bind(this);
    document.addEventListener("click", this._dismissHighlight);
  }

  /**
   * Navigate to a target page's macro-cell.
   * Scrolls the grid container's scrollable parent to bring the cell into view,
   * then applies a 1.5-second pulse animation.
   *
   * @param {number} page - 1-based page number to navigate to
   */
  navigateToPage(page) {
    const canvas = this._getCanvasForPage(page);
    if (!canvas) return;

    // Scroll the scrollable parent (#main) to bring the canvas into view
    const scrollParent = this._gridContainer.closest("#main");
    if (scrollParent) {
      this._scrollIntoView(canvas, scrollParent);
    } else {
      canvas.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" });
    }

    // Apply pulse animation
    this._applyPulse(canvas);
  }

  /**
   * Highlight a specific sub-cell within a page's canvas.
   * Renders a visible outline overlay on the corresponding sub-cell position.
   * The outline persists until the user clicks anywhere else.
   *
   * @param {number} page - 1-based page number
   * @param {number} row - Sub-cell row (0–19)
   * @param {number} col - Sub-cell column (0–19)
   */
  highlightSubCell(page, row, col) {
    // Remove any existing highlight first
    this._removeOverlay();

    const canvas = this._getCanvasForPage(page);
    if (!canvas) return;

    // Scroll to the page and pulse it
    this.navigateToPage(page);

    // Create an overlay div positioned over the specific sub-cell
    const overlay = document.createElement("div");
    overlay.className = "subcell-highlight";

    // Position the overlay relative to the canvas.
    // The canvas is 20×20 native pixels, each sub-cell is 1×1 native pixel.
    // We use percentage-based positioning so it scales with CSS zoom.
    const leftPct = (col / CELL_SIZE) * 100;
    const topPct = (row / CELL_SIZE) * 100;
    const sizePct = (1 / CELL_SIZE) * 100;

    overlay.style.left = `${leftPct}%`;
    overlay.style.top = `${topPct}%`;
    overlay.style.width = `${sizePct}%`;
    overlay.style.height = `${sizePct}%`;

    // The canvas needs to be a positioning context
    const wrapper = this._ensureWrapper(canvas);
    wrapper.appendChild(overlay);

    this._activeOverlay = overlay;
  }

  /**
   * Scroll the target element into view within the scroll parent,
   * accounting for CSS transform scaling on the grid container.
   *
   * @param {HTMLCanvasElement} target
   * @param {HTMLElement} scrollParent
   * @private
   */
  _scrollIntoView(target, scrollParent) {
    // Get the zoom level from the CSS variable
    const zoom = parseFloat(
      getComputedStyle(this._gridContainer).getPropertyValue("--zoom")
    ) || 1;

    // Calculate the target's position relative to the grid container
    const gridRect = this._gridContainer.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();

    // Compute the offset within the scroll parent
    const parentRect = scrollParent.getBoundingClientRect();

    const targetCenterX = targetRect.left + targetRect.width / 2 - parentRect.left + scrollParent.scrollLeft;
    const targetCenterY = targetRect.top + targetRect.height / 2 - parentRect.top + scrollParent.scrollTop;

    const scrollX = targetCenterX - parentRect.width / 2;
    const scrollY = targetCenterY - parentRect.height / 2;

    scrollParent.scrollTo({
      left: Math.max(0, scrollX),
      top: Math.max(0, scrollY),
      behavior: "smooth"
    });
  }

  /**
   * Apply a 1.5-second pulse animation to the target canvas.
   * @param {HTMLCanvasElement} canvas
   * @private
   */
  _applyPulse(canvas) {
    // Remove pulse from previously pulsing canvas
    if (this._pulsingCanvas) {
      this._pulsingCanvas.classList.remove("nav-pulse");
    }

    // Force reflow to restart animation if same element
    canvas.classList.remove("nav-pulse");
    void canvas.offsetWidth;

    canvas.classList.add("nav-pulse");
    this._pulsingCanvas = canvas;

    // Remove the class after animation completes
    setTimeout(() => {
      if (this._pulsingCanvas === canvas) {
        canvas.classList.remove("nav-pulse");
        this._pulsingCanvas = null;
      }
    }, PULSE_DURATION_MS);
  }

  /**
   * Ensure the canvas has a positioned wrapper for overlay placement.
   * @param {HTMLCanvasElement} canvas
   * @returns {HTMLElement} The wrapper element
   * @private
   */
  _ensureWrapper(canvas) {
    // Check if canvas already has a wrapper
    if (canvas.parentElement && canvas.parentElement.classList.contains("cell-wrapper")) {
      return canvas.parentElement;
    }

    // Create a wrapper and insert it in place of the canvas
    const wrapper = document.createElement("div");
    wrapper.className = "cell-wrapper";
    canvas.parentElement.insertBefore(wrapper, canvas);
    wrapper.appendChild(canvas);

    return wrapper;
  }

  /**
   * Remove the active sub-cell highlight overlay.
   * @private
   */
  _removeOverlay() {
    if (this._activeOverlay) {
      this._activeOverlay.remove();
      this._activeOverlay = null;
    }
  }

  /**
   * Dismiss the sub-cell highlight on any click.
   * The highlight persists until the next click anywhere.
   * @param {Event} event
   * @private
   */
  _dismissHighlight(event) {
    // Don't dismiss if clicking on a counterpart link (data-navigate-page attribute)
    if (event.target.closest("[data-navigate-page]")) {
      return;
    }

    this._removeOverlay();
  }

  /**
   * Get the canvas element for a given page number.
   * @param {number} page - 1-based page number
   * @returns {HTMLCanvasElement|null}
   * @private
   */
  _getCanvasForPage(page) {
    return this._gridContainer.querySelector(`canvas[data-page="${page}"]`);
  }

  /**
   * Navigate to a target page and optionally highlight a sub-cell.
   * Convenience method combining navigateToPage and highlightSubCell.
   *
   * @param {number} page - 1-based page number
   * @param {number} [row] - Sub-cell row (0–19), optional
   * @param {number} [col] - Sub-cell column (0–19), optional
   */
  navigateTo(page, row, col) {
    if (row !== undefined && col !== undefined) {
      this.highlightSubCell(page, row, col);
    } else {
      this.navigateToPage(page);
    }
  }

  /**
   * Clean up event listeners and overlays.
   */
  destroy() {
    document.removeEventListener("click", this._dismissHighlight);
    this._removeOverlay();
    if (this._pulsingCanvas) {
      this._pulsingCanvas.classList.remove("nav-pulse");
      this._pulsingCanvas = null;
    }
  }
}
