// Keep the last active value per input, including values entered with the mouse
// or keyboard before the user manually returns a control to zero.
export function initializeSliderToggles(root) {
  for (const input of root.querySelectorAll('input[type="range"], input[type="number"]')) {
    const min = input.min === "" ? -Infinity : Number(input.min);
    const max = input.max === "" ? Infinity : Number(input.max);
    const clamp = (value) => Math.max(min, Math.min(max, value));
    const off = input.hasAttribute("data-off-value")
      ? input.getAttribute("data-off-value")
      : String(clamp(input.id === "scale-x" || input.id === "scale-y" ? 1 : 0));
    const isOff = (value) => value === off
      || (value !== "" && off !== "" && Number(value) === Number(off));
    let previous;
    const remember = () => {
      if (input.value !== "" && input.validity.valid && !isOff(input.value)) previous = input.value;
    };
    remember();
    input.addEventListener("input", remember);
    input.addEventListener("keydown", (event) => {
      if (event.key !== " " || event.ctrlKey || event.altKey || event.metaKey
          || input.disabled || input.readOnly) return;
      event.preventDefault();
      if (event.repeat) return;
      if (!isOff(input.value)) {
        remember();
        input.value = off;
      } else {
        const step = Number(input.step) > 0 ? Number(input.step) : 1;
        let fallback = input.defaultValue;
        if (fallback === "" || isOff(fallback)) {
          let next = clamp(Number(off) + step);
          if (isOff(String(next))) next = clamp(Number(off) - step);
          fallback = String(Number(next.toFixed(8)));
        }
        input.value = previous ?? fallback;
      }
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
}
