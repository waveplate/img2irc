function decimalPlaces(value) {
  const match = /\.(\d+)/.exec(String(value));
  return match ? match[1].length : 0;
}

function numericAttribute(input, name) {
  const raw = input.getAttribute(name);
  if (raw === null || raw === "") return undefined;
  const value = Number(raw);
  return Number.isFinite(value) ? value : undefined;
}

export function initializeNumberScrubbers(root) {
  for (const input of root.querySelectorAll('input[type="number"]')) {
    input.classList.add("scrubbable-number");
    input.title ||= "Type a value, or drag horizontally to adjust it";

    let gesture;
    input.addEventListener("pointerdown", (event) => {
      if (event.button !== 0 || input.disabled) return;
      const bounds = input.getBoundingClientRect();
      // Preserve the browser's native increment/decrement buttons.
      if (event.clientX >= bounds.right - 20) return;
      const parsed = Number(input.value);
      const step = numericAttribute(input, "step") ?? 1;
      gesture = {
        pointerId: event.pointerId,
        startX: event.clientX,
        lastX: event.clientX,
        value: Number.isFinite(parsed) ? parsed : (numericAttribute(input, "min") ?? 0),
        step,
        precision: Math.max(decimalPlaces(step), decimalPlaces(input.value)),
        dragged: false,
      };
      input.setPointerCapture(event.pointerId);
    });

    input.addEventListener("pointermove", (event) => {
      if (!gesture || event.pointerId !== gesture.pointerId) return;
      const totalDistance = event.clientX - gesture.startX;
      const movement = event.clientX - gesture.lastX;
      gesture.lastX = event.clientX;
      if (!gesture.dragged && Math.abs(totalDistance) < 3) return;
      if (!gesture.dragged) {
        gesture.dragged = true;
        document.body.classList.add("number-scrubbing");
      }
      event.preventDefault();

      // The farther the pointer travels from its starting point, the faster
      // the value changes. Alt provides fine control and Shift a coarse mode.
      const distanceMultiplier = Math.min(12, 1 + Math.abs(totalDistance) / 60);
      const modifier = event.altKey ? 0.25 : event.shiftKey ? 10 : 1;
      gesture.value += movement * gesture.step * distanceMultiplier * modifier / 6;
      const minimum = numericAttribute(input, "min");
      const maximum = numericAttribute(input, "max");
      if (minimum !== undefined) gesture.value = Math.max(minimum, gesture.value);
      if (maximum !== undefined) gesture.value = Math.min(maximum, gesture.value);
      const quantized = Math.round(gesture.value / gesture.step) * gesture.step;
      const next = Number(quantized.toFixed(Math.min(8, gesture.precision)));
      if (Number(input.value) === next) return;
      input.value = String(next);
      input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertReplacementText" }));
    });

    const finish = (event) => {
      if (!gesture || event.pointerId !== gesture.pointerId) return;
      if (input.hasPointerCapture(event.pointerId)) input.releasePointerCapture(event.pointerId);
      gesture = undefined;
      document.body.classList.remove("number-scrubbing");
    };
    input.addEventListener("pointerup", finish);
    input.addEventListener("pointercancel", finish);
    input.addEventListener("lostpointercapture", () => {
      gesture = undefined;
      document.body.classList.remove("number-scrubbing");
    });
  }
}
