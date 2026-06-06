/**
 * Snap a value to a range input's min, max, and step.
 * @param {number|string} value
 * @param {HTMLInputElement} slider
 * @returns {number}
 */
export function snapSliderValue(value, slider) {
  const min = Number(slider.min);
  const max = Number(slider.max);
  const step = Number(slider.step) || 1;
  let n = Number(value);
  if (Number.isNaN(n)) {
    return Number(slider.value);
  }
  n = Math.min(max, Math.max(min, n));
  const steps = Math.round((n - min) / step);
  const snapped = min + steps * step;
  return Math.min(max, Math.max(min, snapped));
}

/**
 * Keep a range slider and number input in sync.
 * @param {HTMLInputElement} slider
 * @param {HTMLInputElement} numberInput
 * @param {Object} [options]
 * @param {() => void} [options.onInput] - continuous updates (slider drag)
 * @param {() => void} [options.onChange] - committed value (number field)
 * @param {(n: number) => string} [options.format] - display format for the number field
 */
export function bindSliderNumberInput(slider, numberInput, options = {}) {
  const format = options.format ?? ((n) => String(n));

  const syncFromSlider = () => {
    const val = Number(slider.value);
    numberInput.value = format(val);
    options.onInput?.();
  };

  const applyFromNumber = () => {
    const val = snapSliderValue(numberInput.value, slider);
    slider.value = String(val);
    numberInput.value = format(val);
    options.onChange?.();
    options.onInput?.();
  };

  numberInput.min = slider.min;
  numberInput.max = slider.max;
  numberInput.step = slider.step;

  slider.addEventListener("input", syncFromSlider);
  numberInput.addEventListener("change", applyFromNumber);
  numberInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      applyFromNumber();
      numberInput.blur();
    }
  });

  syncFromSlider();
}
