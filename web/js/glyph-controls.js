const catalogs = new WeakMap();

function showGlyphPreview(container, group) {
  const panel = container.closest(".glyph-panel");
  const preview = panel?.querySelector("#glyph-preview-grid");
  if (!preview) return;
  panel.querySelector("#glyph-preview-label").textContent = group ? `Glyphs · ${group.name}` : "Glyphs";
  const cells = [];
  if (group) {
    for (const char of group.characters) {
      if (cells.length === 256) break;
      const cell = document.createElement("span");
      cell.className = "glyph-preview-cell";
      cell.textContent = char;
      cell.title = `U+${char.codePointAt(0).toString(16).toUpperCase().padStart(4, "0")}`;
      cells.push(cell);
    }
  }
  preview.replaceChildren(...cells);
  panel.querySelector("#glyph-preview-count").textContent = !group ? "Focus a set to inspect its glyphs."
    : group.count > cells.length ? `${cells.length} of ${group.count} glyphs shown` : `${group.count} glyphs`;
}

export function populateGlyphGroups(container, groups) {
  catalogs.set(container, new Map(groups.map((group) => [group.name, group])));
  const fragment = document.createDocumentFragment();
  for (const group of groups) {
    const row = document.createElement("label");
    row.className = "glyph-row";
    row.dataset.name = group.name.toLowerCase();

    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.value = group.name;
    checkbox.checked = group.name === "default";

    const name = document.createElement("span");
    name.textContent = group.name;
    const count = document.createElement("span");
    count.className = "glyph-count";
    count.textContent = String(group.count);
    const preview = document.createElement("code");
    preview.textContent = group.characters;
    preview.title = group.characters;
    row.append(checkbox, name, count, preview);
    row.addEventListener("focusin", () => showGlyphPreview(container, group));
    row.addEventListener("click", () => showGlyphPreview(container, group));
    fragment.append(row);
  }
  container.replaceChildren(fragment);
  showGlyphPreview(container, catalogs.get(container).get("default") ?? groups[0]);
}

export function chooseGlyphPreset(container, name) {
  container.querySelectorAll('input[type="checkbox"]').forEach((checkbox) => {
    checkbox.checked = name === "all" ? checkbox.value === "all" : checkbox.value === name;
  });
  showGlyphPreview(container, catalogs.get(container)?.get(name));
}

export function filterGlyphGroups(container, query) {
  const normalized = query.trim().toLowerCase();
  container.querySelectorAll(".glyph-row").forEach((row) => {
    row.hidden = normalized !== "" && !row.dataset.name.includes(normalized);
  });
}
