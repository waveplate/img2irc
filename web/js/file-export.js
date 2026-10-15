const key = (kind) => `img2irc.save-name.${kind}`;
const remembered = (kind) => { try { return localStorage.getItem(key(kind)); } catch { return null; } };
const remember = (kind, name) => { try { localStorage.setItem(key(kind), name); } catch {} };

export async function saveFile(kind, createBlob, stem = "img2irc") {
  const suggestedName = remembered(kind) || `${stem}.${kind}`;
  const mime = kind === "png" ? "image/png" : "text/plain";
  if (typeof window.showSaveFilePicker === "function") {
    const handle = await window.showSaveFilePicker({
      suggestedName,
      types: [{ description: kind === "png" ? "PNG image" : "Encoded text", accept: { [mime]: [`.${kind}`] } }],
    });
    const blob = await createBlob();
    const file = await handle.createWritable();
    try { await file.write(blob); await file.close(); }
    catch (error) { await file.abort().catch(() => {}); throw error; }
    remember(kind, handle.name);
    return handle.name;
  }
  const name = window.prompt(`Save ${kind.toUpperCase()} as`, suggestedName)?.trim();
  if (!name) return;
  const blob = await createBlob();
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  remember(kind, name);
  return name;
}
