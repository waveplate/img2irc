import { EXTRA_EFFECTS, installExtraEffectControls } from "./effect-catalog.js";

const EFFECTS = [
  ["brightness", "Brightness", "Adjustments", { brightness: 0 }],
  ["contrast", "Contrast", "Adjustments", { contrast: 0 }],
  ["luma-contrast", "Luma Contrast", "Adjustments", { "luma-contrast": 0 }],
  ["saturation", "Saturation", "Adjustments", { saturation: 0 }],
  ["gamma", "Gamma", "Adjustments", { gamma: 0 }],
  ["hue", "Hue", "Adjustments", { hue: 0 }],
  ["rotate", "Rotate", "Geometry", { rotate: 0 }],
  ["crop", "Crop", "Geometry", { "crop-top": 0, "crop-right": 0, "crop-bottom": 0, "crop-left": 0 }],
  ["scale", "Scale", "Geometry", { "scale-x": 1, "scale-y": 1 }],
  ["flip-horizontal", "Flip Horizontal", "Geometry", { "flip-horizontal": false }],
  ["flip-vertical", "Flip Vertical", "Geometry", { "flip-vertical": false }],
  ["gaussian-blur", "Gaussian Blur", "Effects", { "gaussian-blur": 0 }],
  ["median-blur", "Median Blur", "Effects", { "median-blur": 0 }],
  ["line-thickness", "Line Thickness", "Effects", { "line-thickness": 0 }],
  ["box-blur", "Box Blur", "Effects", { "box-blur": false }],
  ["pixelize", "Pixelize", "Effects", { pixelize: 0 }],
  ["dither", "Dither", "Effects", { dither: 0 }],
  ["invert", "Invert", "Stylize", { invert: false }],
  ["grayscale", "Grayscale", "Stylize", { grayscale: false }],
  ["replace", "Replace Colour", "Colour", { "replace-enabled": false }],
  ["quantize", "Quantize Colours", "Colour", { "quantize-enabled": false }],
  ...EXTRA_EFFECTS.map(([id, title, group, , parameters]) => [id, title, group,
    parameters ? Object.fromEntries(parameters.map(([key, , , , , value]) => [key, value])) : { [id]: false }]),
];

// Keep parameter inputs when an effect is disabled. DOM order is also the
// execution order sent to the renderer, including after dragging an entry.
export function initializePipelineControls(root) {
  installExtraEffectControls(root);
  const element = (id) => root.querySelector(`#${id}`);
  const catalog = element("pipeline-catalog");
  const effects = element("pipeline-effects");
  const states = [];
  const entries = () => [...effects.children].filter((node) => !node.hidden);
  const order = () => entries().map((node) => node.dataset.pipelineEffect).join(",");
  const reordered = () => effects.dispatchEvent(new CustomEvent("pipelinechange", { bubbles: true }));
  let drag;
  effects.addEventListener("pointermove", (event) => {
    if (!drag || drag.id !== event.pointerId) return;
    if (!drag.moved && Math.abs(event.clientY - drag.y) < 5) return;
    drag.moved = true;
    drag.node.classList.add("effect-dragging");
    root.classList.add("effect-reordering");
    const target = entries().find((node) => {
      if (node === drag.node) return false;
      const rect = node.getBoundingClientRect();
      return event.clientY < rect.top + rect.height / 2;
    });
    if (target) effects.insertBefore(drag.node, target);
    else effects.append(drag.node);
    const panel = effects.closest(".control-panel");
    const bounds = panel.getBoundingClientRect();
    if (event.clientY < bounds.top + 32) panel.scrollTop -= 16;
    else if (event.clientY > bounds.bottom - 32) panel.scrollTop += 16;
  });
  const finishDrag = () => {
    if (!drag) return;
    const previous = drag;
    drag = undefined;
    previous.node.classList.remove("effect-dragging");
    root.classList.remove("effect-reordering");
    if (effects.hasPointerCapture(previous.id)) effects.releasePointerCapture(previous.id);
    if (order() !== previous.order) reordered();
  };
  effects.addEventListener("pointerup", finishDrag);
  effects.addEventListener("pointercancel", finishDrag);
  effects.addEventListener("lostpointercapture", finishDrag);
  const value = (input) => input.type === "checkbox" ? input.checked : Number(input.value);
  const write = (input, next) => {
    if (input.type === "checkbox") input.checked = next;
    else {
      input.value = String(next);
      if (input.nextElementSibling?.tagName === "OUTPUT") input.nextElementSibling.value = String(next);
    }
  };
  const notify = (input) => {
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const refresh = () => {
    for (const state of states) {
      state.node.hidden = !state.added;
      state.node.dataset.disabled = String(!state.enabled);
      state.parameters.disabled = !state.enabled;
      state.toggle.checked = state.enabled;
      state.add.disabled = state.added;
    }
    const count = states.filter((state) => state.added).length;
    element("pipeline-count").textContent = `${count} effect${count === 1 ? "" : "s"}`;
    element("pipeline-empty").hidden = count !== 0;
  };

  let category;
  const groupOrder = ["Adjustments", "Geometry", "Effects", "Stylize", "Colour"];
  const catalogEffects = [...EFFECTS].sort((a, b) => groupOrder.indexOf(a[2]) - groupOrder.indexOf(b[2]));
  for (const [id, title, group, neutral] of catalogEffects) {
    const node = root.querySelector(`[data-pipeline-effect="${id}"]`);
    const fields = Object.entries(neutral).map(([key, off]) => [element(key), off]);
    for (const [input, off] of fields) input.dataset.neutralValue = String(off);
    const primary = fields[0][0];
    const isToggle = primary.type === "checkbox";
    const enabled = fields.some(([input, off]) => value(input) !== off);
    const oldParameters = node.querySelector(".effect-parameters");
    const parameters = document.createElement("fieldset");
    parameters.className = "effect-parameters range-grid";
    parameters.setAttribute("aria-label", `${title} parameters`);
    parameters.append(...oldParameters.childNodes);
    oldParameters.replaceWith(parameters);

    const header = document.createElement("div");
    header.className = "effect-header";
    const handle = document.createElement("button");
    handle.type = "button";
    handle.className = "effect-drag-handle";
    handle.textContent = "⠿";
    handle.title = "Drag to reorder · ↑/↓ move";
    handle.setAttribute("aria-label", `Move ${title}; drag or use Up and Down`);
    handle.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      handle.focus();
      drag = { node, id: event.pointerId, y: event.clientY, order: order(), moved: false };
      effects.setPointerCapture(event.pointerId);
    });
    handle.addEventListener("keydown", (event) => {
      if (!["ArrowUp", "ArrowDown"].includes(event.key)) return;
      event.preventDefault();
      const nodes = entries();
      const index = nodes.indexOf(node);
      if (event.key === "ArrowUp" && index > 0) effects.insertBefore(node, nodes[index - 1]);
      else if (event.key === "ArrowDown" && index < nodes.length - 1) effects.insertBefore(nodes[index + 1], node);
      else return;
      handle.focus();
      node.scrollIntoView({ block: "nearest" });
      reordered();
    });
    const label = document.createElement("label");
    label.className = "inline-control";
    const toggle = isToggle ? primary : document.createElement("input");
    if (isToggle) primary.closest("label").remove();
    toggle.type = "checkbox";
    toggle.setAttribute("aria-label", `Enable ${title}`);
    const caption = document.createElement("span");
    caption.textContent = title;
    label.append(toggle, caption);
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "effect-remove";
    remove.textContent = "×";
    remove.title = `Remove ${title}`;
    remove.setAttribute("aria-label", `Remove ${title}`);
    header.append(handle, label, remove);
    node.prepend(header);
    parameters.hidden = !parameters.querySelector("input, select, textarea, button");

    if (category !== group) {
      category = group;
      const heading = document.createElement("h2");
      heading.className = "group-heading";
      heading.textContent = group;
      catalog.append(heading);
    }
    const add = document.createElement("button");
    add.type = "button";
    add.className = "effect-option";
    add.textContent = title;
    add.dataset.name = `${title} ${group}`.toLowerCase();
    catalog.append(add);
    const state = { node, parameters, toggle, add, fields, enabled, added: enabled };
    states.push(state);

    toggle.addEventListener("input", () => {
      state.enabled = toggle.checked;
      refresh();
    });
    remove.addEventListener("click", () => {
      for (const [input, off] of fields) write(input, off);
      state.added = false;
      state.enabled = false;
      refresh();
      for (const [input] of fields) notify(input);
      element("pipeline-add").querySelector("summary").focus();
    });
    add.addEventListener("click", () => {
      state.added = true;
      state.enabled = true;
      if (isToggle) primary.checked = true;
      effects.append(node);
      refresh();
      node.scrollIntoView({ block: "nearest" });
      (isToggle ? toggle : primary).focus();
      notify(primary);
    });
  }

  element("pipeline-filter").addEventListener("input", (event) => {
    const query = event.target.value.trim().toLowerCase();
    for (const state of states) state.add.hidden = !state.add.dataset.name.includes(query);
  });
  element("pipeline-add").addEventListener("toggle", (event) => {
    if (event.currentTarget.open) element("pipeline-filter").focus();
  });
  element("reset-adjustments").addEventListener("click", () => {
    // The app's reset listener restores the input defaults in this event too.
    queueMicrotask(() => {
      for (const state of states) {
        for (const [input, off] of state.fields) write(input, off);
        state.added = false;
        state.enabled = false;
      }
      refresh();
    });
  });
  refresh();
  return {
    snapshot: () => entries().map((node) => ({
      id: node.dataset.pipelineEffect,
      enabled: states.find((state) => state.node === node).enabled,
    })),
    restore: (saved = []) => {
      for (const state of states) { state.added = false; state.enabled = false; }
      for (const entry of saved) {
        const state = states.find((state) => state.node.dataset.pipelineEffect === entry.id);
        if (!state) continue;
        state.added = true;
        state.enabled = entry.enabled;
        effects.append(state.node);
      }
      refresh();
    },
  };
}
