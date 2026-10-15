const MODEL_BASE = "https://paddle-model-ecology.bj.bcebos.com/paddlex/official_inference_model/paddle3.0.0/";
// Bump when replacing model archives so previously cached bytes are refreshed.
const CACHE_REVISION = "ppocr-v6-1";

export function ocrModels(tier = "tiny") {
  if (!["tiny", "small", "medium"].includes(tier)) throw new Error(`Unknown OCR model: ${tier}`);
  const label = tier[0].toUpperCase() + tier.slice(1);
  return ["det", "rec"].map((kind) => {
    const name = `PP-OCRv6_${tier}_${kind}`;
    const archive = `${name}_onnx_infer.tar`;
    return {
      name,
      label: `${label} ${kind === "det" ? "detection" : "recognition"}`,
      url: tier === "tiny"
        ? new URL(`../assets/ocr/${archive}`, import.meta.url).href
        : new URL(archive, MODEL_BASE).href,
    };
  });
}

class IndexedDbModelStore {
  #database;

  async #open() {
    if (!this.#database) {
      this.#database = new Promise((resolve, reject) => {
        // Access can throw in browsers that prohibit persistent storage.
        const request = globalThis.indexedDB.open("img2irc-ocr-models", 1);
        let abandoned = false;
        request.onupgradeneeded = () => request.result.createObjectStore("archives");
        request.onerror = () => reject(request.error);
        request.onblocked = () => {
          abandoned = true;
          reject(new Error("OCR model cache is blocked by another tab"));
        };
        request.onsuccess = () => {
          const database = request.result;
          if (abandoned) { database.close(); return; }
          database.onversionchange = () => {
            database.close();
            this.#database = undefined;
          };
          resolve(database);
        };
      }).catch(() => undefined);
    }
    return this.#database;
  }

  async get(key) {
    const database = await this.#open();
    if (!database) return undefined;
    return new Promise((resolve, reject) => {
      const transaction = database.transaction("archives", "readonly");
      const request = transaction.objectStore("archives").get(key);
      transaction.oncomplete = () => resolve(request.result);
      transaction.onerror = transaction.onabort = () => reject(transaction.error);
    });
  }

  async put(key, blob) {
    const database = await this.#open();
    if (!database) return;
    return new Promise((resolve, reject) => {
      const transaction = database.transaction("archives", "readwrite");
      transaction.objectStore("archives").put(blob, key);
      // Wait for commit, not just the individual put request.
      transaction.oncomplete = () => resolve();
      transaction.onerror = transaction.onabort = () => reject(transaction.error);
    });
  }
}

async function isModelArchive(blob) {
  // Reject empty responses and HTML fallback pages before persisting them.
  return blob instanceof Blob && blob.size > 512
    && await blob.slice(257, 262).text() === "ustar";
}

export class OcrModelCache {
  #store;
  #fetch;
  #pending = new Map();

  constructor({ store = new IndexedDbModelStore(), fetchImpl = (...args) => fetch(...args) } = {}) {
    this.#store = store;
    this.#fetch = fetchImpl;
  }

  async load(model, status = () => {}) {
    const key = `${CACHE_REVISION}:${model.url}`;
    if (this.#pending.has(key)) return this.#pending.get(key);
    const pending = this.#load(key, model, status);
    this.#pending.set(key, pending);
    try {
      return await pending;
    } finally {
      this.#pending.delete(key);
    }
  }

  async #load(key, model, status) {
    try {
      const cached = await this.#store.get(key);
      if (await isModelArchive(cached)) {
        status(`Loading cached ${model.label} model…`);
        return cached;
      }
    } catch { /* Storage failures must not prevent OCR. */ }

    status(`Downloading ${model.label} model…`);
    const response = await this.#fetch(model.url);
    if (!response.ok) throw new Error(`${model.label} model: HTTP ${response.status}`);
    const blob = await response.blob();
    if (!await isModelArchive(blob)) throw new Error(`${model.label}: invalid model archive`);
    try {
      await this.#store.put(key, blob);
    } catch { /* Quota/private mode: use the downloaded bytes without caching. */ }
    return blob;
  }
}
