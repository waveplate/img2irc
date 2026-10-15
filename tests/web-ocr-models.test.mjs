import test from "node:test";
import assert from "node:assert/strict";
import { OcrModelCache, ocrModels } from "../web/js/ocr-models.js";

function archive() {
  const bytes = new Uint8Array(1536);
  bytes.set(new TextEncoder().encode("ustar"), 257);
  return new Blob([bytes]);
}

function memoryStore() {
  const records = new Map();
  return {
    records,
    get: async (key) => records.get(key),
    put: async (key, value) => { records.set(key, value); },
  };
}

test("each OCR tier selects a matching detection and recognition pair", () => {
  for (const tier of ["tiny", "small", "medium"]) {
    const models = ocrModels(tier);
    assert.deepEqual(models.map((model) => model.name), [`PP-OCRv6_${tier}_det`, `PP-OCRv6_${tier}_rec`]);
    for (const model of models) assert.ok(model.url.endsWith(`${model.name}_onnx_infer.tar`));
  }
  assert.throws(() => ocrModels("unknown"), /Unknown OCR model/);
});

test("downloads are deduplicated and reused by a new cache instance", async () => {
  const store = memoryStore();
  let downloads = 0;
  const fetchImpl = async () => { downloads++; return new Response(archive()); };
  const model = ocrModels("small")[0];
  const first = new OcrModelCache({ store, fetchImpl });
  const [a, b] = await Promise.all([first.load(model), first.load(model)]);
  assert.equal(downloads, 1);
  assert.equal(a, b);
  const status = [];
  const second = new OcrModelCache({ store, fetchImpl });
  assert.equal(await second.load(model, (message) => status.push(message)), a);
  assert.equal(downloads, 1);
  assert.match(status[0], /cached Small detection/);
  await second.load(ocrModels("medium")[0]);
  assert.equal(downloads, 2);
  assert.equal(store.records.size, 2);
});

test("denied browser storage does not prevent model loading", async () => {
  const store = {
    get: async () => { throw new Error("storage denied"); },
    put: async () => { throw new Error("quota exceeded"); },
  };
  const cache = new OcrModelCache({ store, fetchImpl: async () => new Response(archive()) });
  assert.equal((await cache.load(ocrModels()[0])).size, 1536);
});

test("failed downloads and HTML fallback pages are not cached and can be retried", async () => {
  const store = memoryStore();
  let attempt = 0;
  const cache = new OcrModelCache({ store, fetchImpl: async () => {
    attempt++;
    if (attempt === 1) return new Response("unavailable", { status: 503 });
    if (attempt === 2) return new Response("<html>not a model</html>");
    return new Response(archive());
  } });
  const model = ocrModels()[1];
  await assert.rejects(cache.load(model), /HTTP 503/);
  await assert.rejects(cache.load(model), /invalid model archive/);
  assert.equal(store.records.size, 0);
  await cache.load(model);
  assert.equal(attempt, 3);
  assert.equal(store.records.size, 1);
});
