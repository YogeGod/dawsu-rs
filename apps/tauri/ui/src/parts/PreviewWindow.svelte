<script>
  import { onMount } from "svelte";
  import PreviewCanvas from "./PreviewCanvas.svelte";

  // The whole page of the map preview's own window (preview.html). The editor window sends the
  // playhead and the open difficulty as "preview-view" events; this page asks for the objects
  // itself and draws them to fill the window. It says "preview-ready" once it listens, so the
  // editor sends the current state straight away.
  let shown = $state.raw({ view: null, path: null, previewKey: null });

  onMount(() => {
    const events = globalThis.window?.__TAURI__?.event;
    if (!events) return;
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
    };
  });
</script>

<PreviewCanvas view={shown.view} path={shown.path} previewKey={shown.previewKey} fill />

<style>
  /* preview.html has no style.css: the page's own frame lives here, scoped to that body. */
  :global(body.preview-page) {
    margin: 0;
    height: 100vh;
    overflow: hidden;
    background: #0b0b0f;
    color: var(--editor-text);
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
    color: var(--editor-muted);
  }
</style>
