import { EXTRA_EFFECTS, effectControl, installExtraEffectControls } from "./effect-catalog.js";

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
  const templates = new Map();
  let nextInstance = 0;
  const entries = () => [...effects.children].filter((node) => !node.hidden);
  const order = () => entries().map((node) => node.dataset.pipelineInstance).join(",");
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
    const count = states.filter((state) => state.added).length;
    for (const state of states) {
      state.node.hidden = !state.added;
      state.node.dataset.disabled = String(!state.enabled);
      state.parameters.disabled = !state.enabled;
      state.toggle.checked = state.enabled;
      state.add.disabled = count >= 64;
    }
    element("pipeline-count").textContent = `${count} effect${count === 1 ? "" : "s"}`;
    element("pipeline-empty").hidden = count !== 0;
  };

  const createState = ([id, title, , neutral], add, original) => {
    const node = original ?? templates.get(id).cloneNode(true);
    const instance = String(++nextInstance);
    node.dataset.pipelineInstance = instance;
    const renamed = new Map();
    for (const input of node.querySelectorAll("[id]")) {
      input.dataset.effectControl = input.id;
      if (!original) {
        renamed.set(input.id, `effect-${instance}-${input.id}`);
        input.id = renamed.get(input.id);
        if (input.matches("input, textarea, select")) {
          if (input.type === "checkbox") input.checked = input.defaultChecked;
          else input.value = input.defaultValue;
        }
      }
    }
    for (const control of node.querySelectorAll("[aria-describedby], [for]")) {
      for (const attribute of ["aria-describedby", "for"]) {
        if (!control.hasAttribute(attribute)) continue;
        control.setAttribute(attribute, control.getAttribute(attribute).split(/\s+/).map((key) => renamed.get(key) ?? key).join(" "));
      }
    }
    if (!original) effects.append(node);
    const fields = Object.entries(neutral).map(([key, off]) => [effectControl(node, key), off]);
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

    const state = { node, parameters, toggle, add, fields, enabled, added: enabled };
    states.push(state);

    toggle.addEventListener("input", () => {
      state.enabled = toggle.checked;
      refresh();
    });
    remove.addEventListener("click", () => {
      state.added = false;
      state.enabled = false;
      refresh();
      state.node.dispatchEvent(new Event("effectreset"));
      toggle.dispatchEvent(new Event("change", { bubbles: true }));
      reordered();
      element("pipeline-add").querySelector("summary").focus();
    });
    effects.dispatchEvent(new CustomEvent("pipelineentry", { bubbles: true, detail: { node } }));
    return state;
  };

  let category;
  const groupOrder = ["Adjustments", "Geometry", "Effects", "Stylize", "Colour"];
  const catalogEffects = [...EFFECTS].sort((a, b) => groupOrder.indexOf(a[2]) - groupOrder.indexOf(b[2]));
  for (const definition of catalogEffects) {
    const [id, title, group] = definition;
    const node = root.querySelector(`[data-pipeline-effect="${id}"]`);
    templates.set(id, node.cloneNode(true));
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
    createState(definition, add, node);
    add.addEventListener("click", () => {
      if (entries().length >= 64) return;
      const state = states.find((entry) => entry.node.dataset.pipelineEffect === id && !entry.added)
        ?? createState(definition, add);
      for (const input of state.node.querySelectorAll("input, textarea, select")) {
        if (input.type === "checkbox") input.checked = input.defaultChecked;
        else input.value = input.defaultValue;
        if (input.type === "range") input.nextElementSibling.value = input.value;
      }
      state.added = true;
      state.enabled = true;
      state.toggle.checked = true;
      effects.append(state.node);
      refresh();
      state.node.dispatchEvent(new Event("effectreset"));
      state.node.scrollIntoView({ block: "nearest" });
      (state.fields[0][0].type === "checkbox" ? state.toggle : state.fields[0][0]).focus();
      notify(state.toggle);
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
    for (const state of states) {
      for (const [input, off] of state.fields) write(input, off);
      state.added = false;
      state.enabled = false;
    }
    refresh();
    for (const state of states) {
      for (const input of state.node.querySelectorAll("input, textarea, select")) {
        if (input.type === "checkbox") input.checked = input.defaultChecked;
        else input.value = input.defaultValue;
        if (input.type === "range") input.nextElementSibling.value = input.value;
      }
      state.toggle.checked = false;
      state.node.dispatchEvent(new Event("effectreset"));
    }
  });
  refresh();
  return {
    snapshot: () => entries().map((node) => ({
      id: node.dataset.pipelineEffect,
      enabled: states.find((state) => state.node === node).enabled,
      values: Object.fromEntries([...node.querySelectorAll("input[data-effect-control], textarea[data-effect-control], select[data-effect-control]")]
        .map((input) => [input.dataset.effectControl, input.type === "checkbox" ? input.checked : input.value])),
    })),
    restore: (saved = []) => {
      for (const state of states) { state.added = false; state.enabled = false; }
      for (const entry of saved) {
        const definition = EFFECTS.find(([id]) => id === entry.id);
        if (!definition || states.filter((state) => state.added).length >= 64) continue;
        const state = states.find((state) => state.node.dataset.pipelineEffect === entry.id && !state.added)
          ?? createState(definition, states.find((state) => state.node.dataset.pipelineEffect === entry.id).add);
        for (const [key, next] of Object.entries(entry.values ?? {})) {
          const input = effectControl(state.node, key);
          if (!input?.matches("input, textarea, select")) continue;
          if (input.type === "checkbox") input.checked = Boolean(next);
          else input.value = String(next);
        }
        state.added = true;
        state.enabled = entry.enabled;
        effects.append(state.node);
      }
      refresh();
      return saved.some((entry) => !EFFECTS.some(([id]) => id === entry.id));
    },
    nodes: () => states.map((state) => state.node),
  };
}
