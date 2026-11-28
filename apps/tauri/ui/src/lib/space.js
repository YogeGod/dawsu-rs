// Space plays and stops the editor from the Export, Drum and Node Graph windows too, unless a text
// or number field has it. Answers whether it sent the key on, so a window's own keys can skip it.
export function forwardSpace(event, events = globalThis.window?.__TAURI__?.event) {
  if (event.key !== " " || event.repeat || event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) return false;
  if (event.defaultPrevented || event.target?.closest?.("input, textarea, select, [contenteditable]")) return false;
  event.preventDefault();
  events?.emitTo("main", "preview-space").catch(reason => console.error(reason));
  return true;
}

// The Export modal inside the editor takes Space and Escape only while it is the one modal open
// and no confirm or error <dialog> is over it.
export function exportModalOnTop(onlyModalOpen, page = globalThis.document) {
  return onlyModalOpen("export") && !page?.querySelector?.("dialog[open]");
}
