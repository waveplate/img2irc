const WORKER_URL = new URL("./render-worker.js", import.meta.url);

export class RendererClient {
  #worker;
  #requests = new Map();
  #nextRequestId = 1;
  #ready;
  #resolveReady;
  #rejectReady;
  #fatalError;
  #generation = 0;
  #restoreReady;
  #fontBytes;
  #imageBytes;
  #figletFonts;

  constructor() {
    this.#startWorker();
  }

  #startWorker() {
    const generation = ++this.#generation;
    this.#fatalError = undefined;
    this.#restoreReady = undefined;
    this.#worker = new Worker(WORKER_URL, { type: "module", name: "img2irc-renderer" });
    this.#ready = new Promise((resolve, reject) => {
      this.#resolveReady = resolve;
      this.#rejectReady = reject;
    });
    this.#ready.catch(() => {});
    this.#worker.addEventListener("message", (event) => {
      if (generation === this.#generation) this.#handleMessage(event.data);
    });
    this.#worker.addEventListener("error", (event) => {
      if (generation !== this.#generation) return;
      this.#fail(new Error(event.message || "The render worker failed"));
    });
    this.#worker.addEventListener("messageerror", () => {
      if (generation !== this.#generation) return;
      this.#fail(new Error("The render worker returned an unreadable message"));
    });
  }

  async setFont(fontBytes) {
    // FontService retains its cached ArrayBuffer, so transfer a copy rather
    // than detaching the cache used when a font is selected again.
    const bytes = fontBytes.slice(0);
    await this.#request("set-font", { bytes }, [bytes]);
    this.#fontBytes = fontBytes.slice(0);
  }

  async setImage(imageBytes) {
    const retained = imageBytes.slice(0);
    await this.#request("set-image", { bytes: imageBytes }, [imageBytes]);
    this.#imageBytes = retained;
  }

  async setFigletFonts(fonts) {
    const retained = fonts.map((font) => ({ ...font, data: font.data?.slice() }));
    const transfer = fonts
      .map((font) => font.data?.buffer)
      .filter((buffer) => buffer instanceof ArrayBuffer);
    await this.#request("set-figlet-fonts", { fonts }, transfer);
    this.#figletFonts = retained;
  }

  renderFiglet(requests) {
    return this.#request("render-figlet", { requests });
  }

  render(options, onProgress) {
    return this.#request("render", { options }, [], onProgress);
  }

  optimizeFontSize(options, onProgress) {
    return this.#request("optimize-font-size", { options }, [], onProgress);
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

  // WASM rendering is synchronous inside the worker. Terminating that worker
  // is the cancellation path that also works without SharedArrayBuffer.
  restart() {
    this.#worker.terminate();
    this.#fail(new DOMException("Rendering interrupted", "AbortError"));
    this.#startWorker();
    const generation = this.#generation;
    this.#restoreReady = this.#ready.then(async () => {
      if (this.#fontBytes) {
        const bytes = this.#fontBytes.slice(0);
        await this.#postRequest("set-font", { bytes }, [bytes], undefined, generation);
      }
      if (this.#imageBytes) {
        const bytes = this.#imageBytes.slice(0);
        await this.#postRequest("set-image", { bytes }, [bytes], undefined, generation);
      }
      if (this.#figletFonts) {
        const fonts = this.#figletFonts.map((font) => ({ ...font, data: font.data?.slice() }));
        await this.#postRequest("set-figlet-fonts", { fonts }, fonts.map((font) => font.data?.buffer).filter(Boolean), undefined, generation);
      }
    });
    this.#restoreReady.catch(() => {});
  }

  async #request(type, payload = {}, transfer = [], onProgress) {
    const generation = this.#generation;
    await this.#ready;
    await this.#restoreReady;
    if (generation !== this.#generation) throw new DOMException("Rendering interrupted", "AbortError");
    if (this.#fatalError) throw this.#fatalError;
    return this.#postRequest(type, payload, transfer, onProgress);
  }

  #postRequest(type, payload = {}, transfer = [], onProgress, generation = this.#generation) {
    if (generation !== this.#generation) return Promise.reject(new DOMException("Rendering interrupted", "AbortError"));
    const id = this.#nextRequestId++;
    const response = new Promise((resolve, reject) => {
      this.#requests.set(id, { resolve, reject, onProgress });
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
    if (message.type === "progress") {
      request.onProgress?.(message.progress);
      return;
    }
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
