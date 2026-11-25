<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import InspectorHead from "./InspectorHead.svelte";
  import PreviewCanvas from "./PreviewCanvas.svelte";

  // The map preview's Inspector panel, shown from View › Map preview (on or off is kept in this
  // window only). It can open the preview in its own window; while that is open the panel draws
  // nothing and offers to bring it back.
  let open = $state(true);
  let skins = $state([{ id: "argon", name: "Argon (osu!lazer)" }]);
  let skinId = $state(readSavedSkin());
  const on = $derived(bridge.mapPreview);

  function readSavedSkin() {
    try {
      return globalThis.localStorage?.getItem("dawsu.previewSkin") || "argon";
    } catch {
      return "argon";
    }
  }

  function saveSkin(next) {
    skinId = next || "argon";
    try {
      globalThis.localStorage?.setItem("dawsu.previewSkin", skinId);
    } catch {}
  }

  async function refreshSkins() {
    try {
      const list = await invoke("preview_skins");
      if (Array.isArray(list) && list.length) {
        skins = list;
        if (skinId.startsWith("dir:") && !skins.some(s => s.id === skinId)) {
          const folderName = skinId.slice(4).split(/[\\/]/).filter(Boolean).pop() || "Custom skin";
          skins = [...skins, { id: skinId, name: folderName }];
        }
      }
    } catch {}
  }

  async function browseSkin() {
    try {
      const folder = await invoke("pick_skin_folder");
      if (!folder) return;
      const id = `dir:${folder}`;
      const name = folder.split(/[\\/]/).filter(Boolean).pop() || "Custom skin";
      if (!skins.some(s => s.id === id)) skins = [...skins, { id, name }];
      saveSkin(id);
    } catch (reason) {
      console.error(reason);
    }
  }

  onMount(() => {
    refreshSkins();
    const onStorage = e => {
      if (e.key === "dawsu.previewSkin") skinId = e.newValue || "argon";
    };
    globalThis.window?.addEventListener?.("storage", onStorage);
    return () => globalThis.window?.removeEventListener?.("storage", onStorage);
  });

  const popOut = () => invoke("preview_window_open").catch(reason => console.error(reason));
  const bringBack = () => invoke("close_preview_window").catch(reason => console.error(reason));
</script>

{#if on}
  <div class="insp">
    <InspectorHead title="Map preview" body="inspector-preview-body" bind:open />
    <div id="inspector-preview-body" class="preview" hidden={!open}>
      {#if bridge.previewWindow}
        <p class="insp-summary">The map preview is in its own window.</p>
        <div class="insp-actions"><button type="button" onclick={bringBack}>Show it here</button></div>
      {:else}
        <div class="preview-frame">
          <PreviewCanvas view={bridge.playhead} path={bridge.scene?.path ?? null} previewKey={bridge.scene?.previewKey ?? null}
            active={open} {skinId} />
        </div>
        <div class="skin-row">
          <label class="skin-label" for="preview-skin-select">Skin</label>
          <select id="preview-skin-select" class="skin-select" value={skinId} onchange={e => saveSkin(e.currentTarget.value)}>
            {#each skins as s (s.id)}
              <option value={s.id}>{s.name}</option>
            {/each}
          </select>
          <button type="button" class="skin-browse" onclick={browseSkin} title="Pick any osu! skin folder">Browse…</button>
        </div>
        <div class="insp-actions"><button type="button" class="popout-btn" onclick={popOut}>Open in its own window</button></div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .preview-frame {
    max-height: 132px;
    overflow: hidden;
    border-radius: 6px;
    border: 1px solid var(--editor-border);
    background: #08080a;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .preview-frame :global(canvas) {
    max-height: 132px;
    width: auto !important;
    max-width: 100%;
    object-fit: contain;
  }
  .skin-row {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-top: 6px;
    min-width: 0;
  }
  .skin-label {
    font-size: 11px;
    color: var(--editor-muted);
    flex: none;
  }
  .skin-select {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border-radius: 5px;
    border: 1px solid var(--editor-border);
    background: var(--editor-control);
    color: var(--editor-text);
    color-scheme: dark;
    font: 11px var(--font-ui);
    text-overflow: ellipsis;
  }
  .skin-select option {
    background: var(--editor-panel);
    color: var(--editor-text);
  }
  .skin-browse {
    flex: none;
    height: 24px;
    padding: 0 7px;
    border-radius: 5px;
    font-size: 11px;
  }
  .popout-btn {
    width: 100%;
  }
</style>
