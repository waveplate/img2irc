export function initializeCropControls(root, getImage) {
  const fields = ["left", "top", "right", "bottom"].map((side) => root.querySelector(`#crop-${side}`));
  const notify = () => fields[0].dispatchEvent(new Event("input", { bubbles: true }));
  root.querySelector("#crop-reset").addEventListener("click", () => {
    fields.forEach((input) => { input.value = "0"; });
    notify();
  });
  const dialog = document.createElement("dialog");
  dialog.className = "crop-dialog";
  dialog.innerHTML = '<h2>Select crop area</h2><p>Drag to select the area to keep. Arrow keys move it; Shift + arrows resize it.</p><canvas tabindex="0" aria-label="Crop selection"></canvas><p role="status"></p><div class="button-row"><button type="button" data-crop-apply>Apply crop</button><button type="button" data-crop-cancel>Cancel</button></div>';
  document.body.append(dialog);
  const canvas = dialog.querySelector("canvas");
  const context = canvas.getContext("2d");
  const status = dialog.querySelector('[role="status"]');
  let image, bitmap, box, drag, loading = 0;
  const draw = () => {
    if (!bitmap) return;
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    const [x, y, right, bottom] = box;
    const sx = canvas.width / image.width, sy = canvas.height / image.height;
    context.fillStyle = "rgb(0 0 0 / 55%)";
    context.fillRect(0, 0, canvas.width, y * sy);
    context.fillRect(0, bottom * sy, canvas.width, canvas.height - bottom * sy);
    context.fillRect(0, y * sy, x * sx, (bottom - y) * sy);
    context.fillRect(right * sx, y * sy, canvas.width - right * sx, (bottom - y) * sy);
    context.strokeStyle = "white"; context.lineWidth = 2;
    context.strokeRect(x * sx, y * sy, (right - x) * sx, (bottom - y) * sy);
    status.textContent = `${right - x} × ${bottom - y} pixels kept · ${image.width} × ${image.height} original`;
  };
  const position = (event) => {
    const rect = canvas.getBoundingClientRect();
    return [Math.max(0, Math.min(image.width, Math.round((event.clientX - rect.left) / rect.width * image.width))),
      Math.max(0, Math.min(image.height, Math.round((event.clientY - rect.top) / rect.height * image.height)))];
  };
  canvas.addEventListener("pointerdown", (event) => {
    if (!bitmap || event.button !== 0) return;
    drag = position(event); canvas.setPointerCapture(event.pointerId); canvas.focus();
  });
  canvas.addEventListener("pointermove", (event) => {
    if (!drag) return;
    const point = position(event);
    box = [Math.min(drag[0], point[0]), Math.min(drag[1], point[1]), Math.max(drag[0], point[0]), Math.max(drag[1], point[1])];
    draw();
  });
  const finish = () => {
    if (!drag) return;
    drag = undefined;
    box[0] = Math.min(box[0], image.width - 1); box[1] = Math.min(box[1], image.height - 1);
    box[2] = Math.max(box[0] + 1, box[2]); box[3] = Math.max(box[1] + 1, box[3]);
    draw();
  };
  canvas.addEventListener("pointerup", finish); canvas.addEventListener("pointercancel", finish);
  canvas.addEventListener("keydown", (event) => {
    const direction = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
    if (!bitmap || !direction) return;
    event.preventDefault();
    if (event.shiftKey) {
      box[2] = Math.max(box[0] + 1, Math.min(image.width, box[2] + direction[0]));
      box[3] = Math.max(box[1] + 1, Math.min(image.height, box[3] + direction[1]));
    } else {
      const dx = Math.max(-box[0], Math.min(image.width - box[2], direction[0]));
      const dy = Math.max(-box[1], Math.min(image.height - box[3], direction[1]));
      box = [box[0] + dx, box[1] + dy, box[2] + dx, box[3] + dy];
    }
    draw();
  });
  dialog.querySelector("[data-crop-apply]").addEventListener("click", () => {
    if (!bitmap) return;
    [box[0], box[1], image.width - box[2], image.height - box[3]].forEach((value, index) => { fields[index].value = String(value); });
    dialog.close(); notify();
  });
  dialog.querySelector("[data-crop-cancel]").addEventListener("click", () => dialog.close());
  dialog.addEventListener("close", () => { ++loading; bitmap?.close(); bitmap = undefined; drag = undefined; });
  root.querySelector("#crop-select").addEventListener("click", async () => {
    const revision = ++loading;
    dialog.showModal(); status.textContent = "Loading crop preview…";
    dialog.querySelector("[data-crop-apply]").disabled = true;
    try {
      const preview = await getImage();
      if (revision !== loading) return;
      const loaded = await createImageBitmap(new ImageData(new Uint8ClampedArray(preview.rgba), preview.width, preview.height));
      if (revision !== loading) { loaded.close(); return; }
      image = preview;
      bitmap = loaded;
      const scale = Math.min(1, 1000 / image.width, 700 / image.height);
      canvas.width = Math.max(1, Math.round(image.width * scale)); canvas.height = Math.max(1, Math.round(image.height * scale));
      box = [Math.min(Number(fields[0].value), image.width - 1), Math.min(Number(fields[1].value), image.height - 1), image.width - Number(fields[2].value), image.height - Number(fields[3].value)];
      box[2] = Math.max(box[0] + 1, box[2]); box[3] = Math.max(box[1] + 1, box[3]);
      dialog.querySelector("[data-crop-apply]").disabled = false;
      draw(); canvas.focus();
    } catch (error) { if (revision === loading) status.textContent = `Could not preview crop: ${error.message ?? error}`; }
  });
}
