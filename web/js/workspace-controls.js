export function initializeWorkspaceControls(root) {
  const dock = root.querySelector(".control-dock");
  const divider = root.querySelector("#pane-divider");
  const widths = new Map();
  const tabs = [...root.querySelectorAll('.control-tabs [role="tab"]')];
  const clamp = (width) => Math.max(230, Math.min(width, root.clientWidth * 0.7, root.clientWidth - 280));
  const currentTab = () => root.dataset.activeTab;
  const updateDivider = () => {
    divider.setAttribute("aria-valuemin", "230");
    divider.setAttribute("aria-valuemax", String(Math.max(230, Math.min(root.clientWidth * 0.7, root.clientWidth - 280))));
    divider.setAttribute("aria-valuenow", String(Math.round(dock.getBoundingClientRect().width)));
  };
  const resize = (width) => {
    const next = clamp(width);
    widths.set(currentTab(), next);
    root.style.setProperty("--dock-width", `${next}px`);
    updateDivider();
    root.dispatchEvent(new CustomEvent("workspacechange"));
  };
  const reset = () => {
    widths.delete(currentTab());
    root.style.removeProperty("--dock-width");
    updateDivider();
    root.dispatchEvent(new CustomEvent("workspacechange"));
  };
  root.addEventListener("controltabchange", () => {
    const width = widths.get(currentTab());
    if (width === undefined) root.style.removeProperty("--dock-width");
    else resize(width);
    updateDivider();
  });

  let drag;
  divider.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    drag = { id: event.pointerId, x: event.clientX, width: dock.getBoundingClientRect().width };
    divider.setPointerCapture(event.pointerId);
    root.classList.add("pane-resizing");
  });
  divider.addEventListener("pointermove", (event) => {
    if (drag?.id === event.pointerId) resize(drag.width + event.clientX - drag.x);
  });
  const finish = () => {
    drag = undefined;
    root.classList.remove("pane-resizing");
  };
  divider.addEventListener("pointerup", finish);
  divider.addEventListener("pointercancel", finish);
  divider.addEventListener("lostpointercapture", finish);
  divider.addEventListener("dblclick", reset);
  divider.addEventListener("keydown", (event) => {
    if (event.key === "Home") {
      event.preventDefault();
      reset();
    } else if (["ArrowLeft", "ArrowRight"].includes(event.key)) {
      event.preventDefault();
      resize(dock.getBoundingClientRect().width + (event.key === "ArrowLeft" ? -1 : 1) * (event.shiftKey ? 40 : 8));
    }
  });
  window.addEventListener("resize", () => {
    if (widths.has(currentTab())) resize(widths.get(currentTab()));
    else updateDivider();
  });

  tabs.forEach((tab, index) => tab.addEventListener("keydown", (event) => {
    const offset = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : 0;
    if (!offset && !["Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1
      : (index + offset + tabs.length) % tabs.length;
    tabs[next].click();
    tabs[next].focus();
  }));
  window.addEventListener("keydown", (event) => {
    if (!event.altKey || event.ctrlKey || event.metaKey) return;
    if (/^[1-4]$/.test(event.key)) {
      event.preventDefault();
      const tab = tabs[Number(event.key) - 1];
      tab.click();
      tab.focus();
    } else if (["ArrowUp", "ArrowDown"].includes(event.key)) {
      const panel = root.querySelector('[data-control-tab-panel]:not([hidden])');
      const controls = [...panel.querySelectorAll("input, select, textarea, button, summary")]
        .filter((input) => !input.matches(":disabled") && input.getClientRects().length > 0);
      if (!controls.length) return;
      event.preventDefault();
      const index = controls.indexOf(document.activeElement);
      const next = index < 0 ? 0 : (index + (event.key === "ArrowUp" ? -1 : 1) + controls.length) % controls.length;
      controls[next].focus();
    }
  });

  // Keep real, keyboard-accessible checkboxes; the visible ON/OFF marker
  // follows their checked state directly rather than keeping another value.
  for (const input of root.querySelectorAll('.control-panel input[type="checkbox"]')) {
    const label = input.closest("label");
    if (!label || label.classList.contains("glyph-row") || label.closest(".overlay-colors")) continue;
    label.classList.add("tui-toggle");
    const state = document.createElement("span");
    state.className = "toggle-state";
    state.setAttribute("aria-hidden", "true");
    label.append(state);
  }
  const ranges = [...root.querySelectorAll('input[type="range"]')];
  const updateRange = (input) => {
    const span = Number(input.max) - Number(input.min);
    const fill = span > 0 ? (Number(input.value) - Number(input.min)) / span * 100 : 0;
    input.style.setProperty("--range-fill", `${Math.max(0, Math.min(100, fill))}%`);
  };
  ranges.forEach(updateRange);
  root.addEventListener("input", (event) => {
    if (event.target.type === "range") updateRange(event.target);
  });
  root.querySelector("#reset-adjustments").addEventListener("click", () => {
    queueMicrotask(() => {
      ranges.forEach(updateRange);
    });
  });
  updateDivider();
  return {
    snapshot: () => Object.fromEntries(widths),
    restore: (saved = {}) => {
      for (const [tab, width] of Object.entries(saved)) {
        if (Number.isFinite(width)) widths.set(tab, width);
      }
      const width = widths.get(currentTab());
      if (width !== undefined) resize(width);
    },
  };
}
