export class GlyphCatalog {
  #url;
  #groups = new Map();

  constructor(url) {
    this.#url = url;
  }

  async load() {
    const response = await fetch(this.#url);
    if (!response.ok) {
      throw new Error(`Could not load glyph catalog: ${response.status} ${response.statusText}`);
    }
    const catalog = await response.json();
    if (catalog.version !== 1 || !Array.isArray(catalog.groups)) {
      throw new Error("Unsupported glyph catalog format");
    }

    this.#groups.clear();
    for (const group of catalog.groups) {
      if (typeof group.name !== "string" || !Array.isArray(group.codepoints)) {
        throw new Error("Invalid glyph group in catalog");
      }
      const characters = [...new Set(group.codepoints)]
        .filter((codepoint) => Number.isInteger(codepoint)
          && codepoint >= 0
          && codepoint <= 0x10ffff
          && !(codepoint >= 0xd800 && codepoint <= 0xdfff))
        .map((codepoint) => String.fromCodePoint(codepoint))
        .join("");
      this.#groups.set(group.name, characters);
    }

    const rank = (name) => name === "default" ? 0 : name === "smooth" ? 1 : name === "all" ? 2 : 3;
    return [...this.#groups]
      .map(([name, characters]) => ({ name, characters, count: [...characters].length }))
      .sort((left, right) => rank(left.name) - rank(right.name)
        || left.name.localeCompare(right.name));
  }

  characters(names) {
    const characters = new Set();
    for (const name of names) {
      const group = this.#groups.get(name);
      if (group === undefined) throw new Error(`Glyph group "${name}" is not in the catalog`);
      for (const character of group) characters.add(character);
    }
    return [...characters].join("");
  }
}
