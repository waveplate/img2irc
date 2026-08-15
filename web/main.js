import { EditorApp } from "./js/editor-app.js";

try {
  const app = new EditorApp();
  await app.start();
} catch (error) {
  const status = document.querySelector("#status");
  status.textContent = `Initialization failed: ${error}`;
  status.dataset.error = "true";
}
