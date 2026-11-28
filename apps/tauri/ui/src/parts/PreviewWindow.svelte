<script>
  import { onMount } from "svelte";
  import PreviewCanvas from "./PreviewCanvas.svelte";
  import { PREVIEW_SKIN_KEY, readPreviewSkin } from "../lib/previewSkin.js";

  // The whole page of the map preview's own window (preview.html). The editor window sends the
  // playhead and the open difficulty as "preview-view" events; this page asks for the objects
  // itself and draws them to fill the window. It says "preview-ready" once it listens, so the
  // editor sends the current state straight away.
  let shown = $state.raw({ view: null, path: null, previewKey: null });
  let skinId = $state(readPreviewSkin());

  onMount(() => {
    const onStorage = e => {
      if (e.key === PREVIEW_SKIN_KEY) skinId = e.newValue || "argon";
    };
    globalThis.window?.addEventListener?.("storage", onStorage);
    const events = globalThis.window?.__TAURI__?.event;
    if (!events) return () => globalThis.window?.removeEventListener?.("storage", onStorage);
    let stop = null;
    let gone = false;
    events.listen("preview-view", ({ payload }) => { shown = payload; }).then(unlisten => {
      if (gone) return unlisten();
      stop = unlisten;
      events.emitTo("main", "preview-ready").catch(reason => console.error(reason));
    });
    return () => {
      gone = true;
      stop?.();
      globalThis.window?.removeEventListener?.("storage", onStorage);
    };
  });

  // Space plays and stops the editor from this window too, unless a text or number field has it.
  function forwardSpace(event) {
    if (event.key !== " " || event.repeat || event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) return;
    if (event.target?.closest?.("input, textarea, select, [contenteditable]")) return;
    event.preventDefault();
    globalThis.window?.__TAURI__?.event?.emitTo("main", "preview-space").catch(reason => console.error(reason));
  }
</script>

<svelte:window onkeydown={forwardSpace} />

<PreviewCanvas view={shown.view} path={shown.path} previewKey={shown.previewKey} {skinId} fill />

<style>
  /* preview.html has no style.css: the page's own frame lives here, scoped to that body. */
  :global(body.preview-page) {
    margin: 0;
    height: 100vh;
    overflow: hidden;
    background: #0b0b0f;
    color: var(--ink-on-dark);
    font: 12px/1.4 var(--font-ui);
    color-scheme: dark;
  }
  :global(body.preview-page main) {
    height: 100vh;
  }
  :global(body.preview-page .insp-summary) {
    margin: 0;
    padding: 24px;
    text-align: center;
    color: #98989f;
  }
</style>
