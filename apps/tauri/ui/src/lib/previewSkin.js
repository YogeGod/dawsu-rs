// The osu! skin picked in Inspector › Map preview › Skin (or the preview window), saved per machine.
// Ids: "argon" (dawsu's own look), "classic" (osu!lazer's Classic), or "dir:<skin folder>".

export const PREVIEW_SKIN_KEY = "dawsu.previewSkin";

export function readPreviewSkin(storage = globalThis.localStorage) {
  try {
    return storage?.getItem(PREVIEW_SKIN_KEY) || "argon";
  } catch {
    return "argon";
  }
}

export function writePreviewSkin(id, storage = globalThis.localStorage) {
  try {
    storage?.setItem(PREVIEW_SKIN_KEY, id);
  } catch {
    // A blocked localStorage only loses the pick on the next start.
  }
}

// What the timeline's objects row loads for a pick: a skin folder as picked, otherwise lazer
// Classic. Argon has no legacy sprites, so it keeps the row as it always was.
export const timelineSkinSource = id => (typeof id === "string" && id.startsWith("dir:") ? id : "classic");
