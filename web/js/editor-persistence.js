const METADATA_KEY = "img2irc.editor-session.v1";

export class EditorPersistence {
  #database;
  #error;

  constructor(onError) { this.#error = onError; }

  metadata() {
    try {
      const saved = JSON.parse(localStorage.getItem(METADATA_KEY) || "null");
      return saved?.version === 1 ? saved : undefined;
    } catch { return undefined; }
  }

  saveMetadata(snapshot) {
    try { localStorage.setItem(METADATA_KEY, JSON.stringify({ ...snapshot, version: 1 })); }
    catch (error) { this.#error(error); }
  }

  async #open() {
    if (!this.#database) {
      this.#database = new Promise((resolve, reject) => {
        const request = indexedDB.open("img2irc-editor-session", 1);
        request.onupgradeneeded = () => request.result.createObjectStore("document");
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
    }
    return this.#database;
  }

  async loadDocument() {
    try {
      const db = await this.#open();
      const transaction = db.transaction("document", "readonly");
      const store = transaction.objectStore("document");
      const read = (key) => new Promise((resolve, reject) => {
        const request = store.get(key);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const [source, result] = await Promise.all([read("source"), read("result")]);
      return { source, result };
    } catch (error) { this.#error(error); return {}; }
  }

  async store(key, value) {
    try {
      const db = await this.#open();
      await new Promise((resolve, reject) => {
        const transaction = db.transaction("document", "readwrite");
        transaction.objectStore("document").put(value, key);
        transaction.oncomplete = resolve;
        transaction.onerror = () => reject(transaction.error);
        transaction.onabort = () => reject(transaction.error || new Error("Session save interrupted"));
      });
    } catch (error) { this.#error(error); }
  }
}

export function captureControls(root) {
  return Object.fromEntries([...root.querySelectorAll("input[id], select[id], textarea[id]")]
    .filter((input) => input.type !== "file" && input.id !== "raw-output" && !input.id.startsWith("overlay-"))
    .map((input) => [input.id, input.type === "checkbox" ? input.checked : input.value]));
}

export function restoreControls(root, values = {}) {
  for (const input of root.querySelectorAll("input[id], select[id], textarea[id]")) {
    if (!Object.hasOwn(values, input.id) || input.type === "file") continue;
    if (input.type === "checkbox") input.checked = Boolean(values[input.id]);
    else if (input.tagName !== "SELECT" || [...input.options].some((option) => option.value === values[input.id])) {
      input.value = String(values[input.id]);
    }
  }
}
