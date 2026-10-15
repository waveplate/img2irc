import { BUILD_ID } from "./build-info.js";

const CACHE_NAME = `img2irc-assets-${BUILD_ID}`;
const pending = new Map();
let pruned = false;

// Immutable assets share the deployment version, so a new build cannot reuse
// an older WASM binary or font. Cache failure still permits a normal download.
export async function cachedAsset(url) {
  const key = new URL(url, globalThis.location.href).href;
  if (!pending.has(key)) {
    pending.set(key, (async () => {
      let cache;
      try {
        cache = await globalThis.caches?.open(CACHE_NAME);
        if (cache && !pruned) {
          pruned = true;
          void globalThis.caches.keys().then((names) => Promise.all(names
            .filter((name) => name.startsWith("img2irc-assets-") && name !== CACHE_NAME)
            .slice(0, -1).map((name) => globalThis.caches.delete(name)))).catch(() => {});
        }
        const saved = await cache?.match(key);
        if (saved) return saved;
      } catch {}
      const response = await fetch(key);
      if (response.ok && cache) void cache.put(key, response.clone()).catch(() => {});
      return response;
    })().catch((error) => { pending.delete(key); throw error; }));
  }
  return (await pending.get(key)).clone();
}

export async function compressedAsset(url) {
  if (typeof DecompressionStream !== "function") return cachedAsset(url);
  const compressed = new URL(url, globalThis.location.href);
  compressed.pathname += ".gz";
  const response = await cachedAsset(compressed);
  if (!response.ok) return cachedAsset(url);
  const body = /\bgzip\b/i.test(response.headers.get("Content-Encoding") || "")
    ? response.body : response.body.pipeThrough(new DecompressionStream("gzip"));
  return new Response(body, {
    headers: { "Content-Type": String(url).includes(".wasm") ? "application/wasm" : "application/octet-stream" },
  });
}
