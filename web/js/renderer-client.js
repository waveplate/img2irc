const WORKER_URL = new URL("./render-worker.js", import.meta.url);

export class RendererClient {
  #worker = new Worker(WORKER_URL, { type: "module", name: "img2irc-renderer" });
  #requests = new Map();
  #nextRequestId = 1;
  #ready;
  #resolveReady;
  #rejectReady;
  #fatalError;

  constructor() {
    this.#ready = new Promise((resolve, reject) => {
      this.#resolveReady = resolve;
      this.#rejectReady = reject;
    });
    this.#worker.addEventListener("message", (event) => this.#handleMessage(event.data));
    this.#worker.addEventListener("error", (event) => {
      this.#fail(new Error(event.message || "The render worker failed"));
    });
    this.#worker.addEventListener("messageerror", () => {
      this.#fail(new Error("The render worker returned an unreadable message"));
    });
  }

  async setFont(fontBytes) {
    // FontService retains its cached ArrayBuffer, so transfer a copy rather
    // than detaching the cache used when a font is selected again.
    const bytes = fontBytes.slice(0);
    await this.#request("set-font", { bytes }, [bytes]);
  }

  async setImage(imageBytes) {
    await this.#request("set-image", { bytes: imageBytes }, [imageBytes]);
  }

  setFigletFonts(fonts) {
    const transfer = fonts
      .map((font) => font.data?.buffer)
      .filter((buffer) => buffer instanceof ArrayBuffer);
    return this.#request("set-figlet-fonts", { fonts }, transfer);
  }

  renderFiglet(requests) {
    return this.#request("render-figlet", { requests });
  }

  render(options) {
    return this.#request("render", { options });
  }

  getTransformedImage(options) {
    return this.#request("get-transformed-image", { options });
  }

  clearCache() {
    return this.#request("clear-cache");
  }

  terminate() {
    this.#worker.terminate();
    this.#fail(new Error("The render worker was terminated"));
  }

  async #request(type, payload = {}, transfer = []) {
    await this.#ready;
    if (this.#fatalError) throw this.#fatalError;
    const id = this.#nextRequestId++;
    const response = new Promise((resolve, reject) => {
      this.#requests.set(id, { resolve, reject });
    });
    try {
      this.#worker.postMessage({ id, type, ...payload }, transfer);
    } catch (error) {
      this.#requests.delete(id);
      throw error;
    }
    return response;
  }

  #handleMessage(message) {
    if (message.type === "ready") {
      this.#resolveReady();
      return;
    }
    if (message.type === "fatal") {
      this.#fail(new Error(message.error || "The render worker could not start"));
      return;
    }
    const request = this.#requests.get(message.id);
    if (!request) return;
    this.#requests.delete(message.id);
    if (message.ok) request.resolve(message.value);
    else request.reject(new Error(message.error || "The render worker request failed"));
  }

  #fail(error) {
    if (this.#fatalError) return;
    this.#fatalError = error;
    this.#rejectReady(error);
    for (const request of this.#requests.values()) request.reject(error);
    this.#requests.clear();
  }
}
