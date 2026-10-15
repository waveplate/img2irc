# Browser OCR assets

The web editor uses the official `@paddleocr/paddleocr-js` SDK. The runtime,
worker, and Tiny detection/recognition models are copied into the generated web
root. Small and Medium are downloaded on demand from the same official model
host below. All three tiers use IndexedDB to cache archives across visits;
Tiny remains the default and does not require cross-origin model downloads.

- `PP-OCRv6_tiny_det_onnx_infer.tar`
  - Source: <https://paddle-model-ecology.bj.bcebos.com/paddlex/official_inference_model/paddle3.0.0/PP-OCRv6_tiny_det_onnx_infer.tar>
  - SHA-256: `ff6ab415b0a6e0c488550f2fb5d5046f1719848df220b2dc21b56402a65bc05d`
- `PP-OCRv6_tiny_rec_onnx_infer.tar`
  - Source: <https://paddle-model-ecology.bj.bcebos.com/paddlex/official_inference_model/paddle3.0.0/PP-OCRv6_tiny_rec_onnx_infer.tar>
  - SHA-256: `1e13b22717b1edd89d4cde4fda272b6c17d5b505c97c2baea99da1a3a2d54b29`

PaddleOCR is distributed under the Apache License 2.0; see
`LICENSE-PaddleOCR.txt`.
