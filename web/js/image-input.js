export const isImageFile = (file) => Boolean(file?.type?.startsWith("image/") || /\.svg$/i.test(file?.name || ""));

// Rust's image decoder accepts raster formats. Decode SVG with the browser,
// then retain a PNG for rendering, OCR and colour picking.
export async function rasterizeSvg(blob, name = "") {
  if (blob.type !== "image/svg+xml" && !/\.svg$/i.test(name)) return blob;
  const url = URL.createObjectURL(new Blob([blob], { type: "image/svg+xml" }));
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const scale = 2048 / Math.max(image.naturalWidth, image.naturalHeight, 1);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
    canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
    canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height);
    return await new Promise((resolve, reject) => canvas.toBlob(
      (png) => png ? resolve(png) : reject(new Error("Could not rasterize SVG")), "image/png",
    ));
  } finally { URL.revokeObjectURL(url); }
}
