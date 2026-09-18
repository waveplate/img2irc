export function populateGlyphGroups(container, groups) {
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
    fragment.append(row);
  }
  container.replaceChildren(fragment);
}

export function chooseGlyphPreset(container, name) {
  container.querySelectorAll('input[type="checkbox"]').forEach((checkbox) => {
    checkbox.checked = name === "all" ? checkbox.value === "all" : checkbox.value === name;
  });
}

export function filterGlyphGroups(container, query) {
  const normalized = query.trim().toLowerCase();
  container.querySelectorAll(".glyph-row").forEach((row) => {
    row.hidden = normalized !== "" && !row.dataset.name.includes(normalized);
  });
}
