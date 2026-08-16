const MANIFEST_URL = new URL("../figlet-fonts.json", import.meta.url);

function validateManifest(manifest) {
  if (manifest?.version !== 1 || !Array.isArray(manifest.fonts)) {
    throw new Error("figlet-fonts.json must contain a version 1 font list");
  }
  return manifest.fonts.map((font, index) => {
    if (!font || typeof font.name !== "string" || typeof font.url !== "string"
      || !Number.isInteger(font.height) || font.height < 1) {
      throw new Error(`Invalid FIGlet manifest entry ${index + 1}`);
    }
    return font;
  });
}

async function fetchFont(font) {
  const url = new URL(font.url, MANIFEST_URL);
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${font.name}: HTTP ${response.status}`);
  return {
    name: font.name,
    height: font.height,
    data: new Uint8Array(await response.arrayBuffer()),
  };
}

export class FigletService {
  #renderer;
  #manifestPromise;
  #loadPromise;
  #artCache = new Map();

  constructor(renderer) {
    this.#renderer = renderer;
  }

  reload() {
    this.#manifestPromise = undefined;
    this.#loadPromise = undefined;
    this.#artCache.clear();
  }

  async fontOptions() {
    return (await this.#manifest()).map(({ name, height }) => ({ name, height }));
  }

  async render(requests, status = () => {}) {
    const needsFonts = requests.some((request) => request.useFiglet);
    let loadResult = { loaded: 0, failed: [] };
    if (needsFonts) {
      status("Loading hosted FIGlet fonts…");
      try {
        loadResult = await this.#load();
      } catch (error) {
        loadResult = { loaded: 0, failed: [String(error)] };
      }
    }
    const arts = Array(requests.length);
    const missing = [];
    requests.forEach((request, index) => {
      const key = JSON.stringify(request);
      if (this.#artCache.has(key)) arts[index] = this.#artCache.get(key);
      else missing.push({ request, index, key });
    });
    if (missing.length) {
      const rendered = await this.#renderer.renderFiglet(missing.map(({ request }) => request));
      missing.forEach(({ index, key }, renderedIndex) => {
        const art = rendered[renderedIndex];
        arts[index] = art;
        if (this.#artCache.size >= 512) {
          this.#artCache.delete(this.#artCache.keys().next().value);
        }
        this.#artCache.set(key, art);
      });
    }
    return { arts, ...loadResult };
  }

  async #load() {
    if (!this.#loadPromise) {
      this.#loadPromise = this.#loadFonts().catch((error) => {
        this.#loadPromise = undefined;
        throw error;
      });
    }
    return this.#loadPromise;
  }

  async #manifest() {
    if (!this.#manifestPromise) {
      this.#manifestPromise = fetch(MANIFEST_URL)
        .then(async (response) => {
          if (!response.ok) {
            throw new Error(`Could not load FIGlet manifest: HTTP ${response.status}`);
          }
          return validateManifest(await response.json());
        })
        .catch((error) => {
          this.#manifestPromise = undefined;
          throw error;
        });
    }
    return this.#manifestPromise;
  }

  async #loadFonts() {
    const manifest = await this.#manifest();
    const fetched = await Promise.allSettled(manifest.map(fetchFont));
    const fonts = fetched
      .filter((result) => result.status === "fulfilled")
      .map((result) => result.value);
    const failed = fetched
      .filter((result) => result.status === "rejected")
      .map((result) => String(result.reason));
    await this.#renderer.setFigletFonts(fonts);
    return { loaded: fonts.length, failed };
  }
}
