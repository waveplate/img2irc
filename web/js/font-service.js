import { cachedAsset, compressedAsset } from "./asset-cache.js";

const GOOGLE_FONT_METADATA_URL =
  "https://cdn.jsdelivr.net/npm/google-font-metadata@latest/data/google-fonts-v2.json";

function findTrueTypeUrl(font) {
  const weightKeys = ["400", ...Object.keys(font.variants || {})];
  for (const weight of [...new Set(weightKeys)]) {
    const styles = font.variants?.[weight];
    if (!styles) continue;
    const styleKeys = ["normal", ...Object.keys(styles)];
    for (const style of [...new Set(styleKeys)]) {
      const subsets = styles[style];
      if (!subsets) continue;
      const subsetKeys = [font.defSubset, "latin", ...Object.keys(subsets)].filter(Boolean);
      for (const subset of [...new Set(subsetKeys)]) {
        const url = subsets[subset]?.url?.truetype;
        if (url) return url;
      }
    }
  }
  return undefined;
}

export class FontService {
  #hostedFonts;
  #fontBytes = new Map();
  #fontFaces = new Map();
  #googleFonts = new Map();

  constructor(hostedFonts) {
    this.#hostedFonts = hostedFonts;
  }

  async populateGoogleFonts(select, status) {
    const install = (fonts) => {
      for (const font of fonts) {
        const key = `google:${font.id}`;
        this.#googleFonts.set(key, font);
        if ([...select.options].some((option) => option.value === key)) continue;
        const option = document.createElement("option");
        option.value = key;
        option.textContent = `${font.family} (Google)`;
        select.append(option);
      }
      status.textContent = `${fonts.length} monospace Google Fonts available`;
    };
    try {
      const saved = JSON.parse(localStorage.getItem("img2irc.google-font-list.v1") || "null");
      if (Array.isArray(saved)) install(saved);
    } catch {}
    try {
      const response = await cachedAsset(GOOGLE_FONT_METADATA_URL);
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      const metadata = await response.json();
      const fonts = Object.values(metadata)
        .filter((font) => font.category === "monospace")
        .map((font) => ({ ...font, trueTypeUrl: findTrueTypeUrl(font) }))
        .filter((font) => font.trueTypeUrl)
        .sort((left, right) => left.family.localeCompare(right.family))
        .map(({ id, family, trueTypeUrl }) => ({ id, family, trueTypeUrl }));
      install(fonts);
      try { localStorage.setItem("img2irc.google-font-list.v1", JSON.stringify(fonts)); } catch {}
    } catch (error) {
      status.textContent = this.#googleFonts.size ? "Using saved Google Fonts list" : `Google Fonts list unavailable: ${error}`;
    }
  }

  async bytes(key) {
    if (this.#fontBytes.has(key)) return this.#fontBytes.get(key);
    const url = this.#hostedFonts.get(key) ?? this.#googleFonts.get(key)?.trueTypeUrl;
    if (!url) throw new Error("The selected font has no TrueType download");
    const promise = (this.#hostedFonts.has(key) ? compressedAsset(url) : cachedAsset(url)).then((response) => {
      if (!response.ok) throw new Error(`Could not load font: ${response.status} ${response.statusText}`);
      return response.arrayBuffer();
    }).catch((error) => { this.#fontBytes.delete(key); throw error; });
    this.#fontBytes.set(key, promise);
    return promise;
  }

  async displayFamily(key) {
    if (this.#fontFaces.has(key)) return this.#fontFaces.get(key);
    const promise = this.#loadDisplayFamily(key);
    this.#fontFaces.set(key, promise);
    try {
      return await promise;
    } catch (error) {
      this.#fontFaces.delete(key);
      throw error;
    }
  }

  async #loadDisplayFamily(key) {
    const family = `img2irc-${key.replaceAll(/[^a-z0-9]+/gi, "-")}`;
    const bytes = await this.bytes(key);
    const face = new FontFace(family, bytes.slice(0), { style: "normal", weight: "400" });
    await face.load();
    document.fonts.add(face);
    return family;
  }
}
