<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyBrowser, outlineBars, previewLength } from "../lib/browser.js";
  import Play from "../icons/Play.svg?raw";

  const model = $derived(bridge.browser ?? emptyBrowser);
  const off = $derived(!bridge.scene || !bridge.browser);
  const preview = $derived(model.preview);
  // The outline is the sample's own loudness, decoded by the backend; without it no bars are drawn.
  const bars = $derived(outlineBars(preview?.peaks));
  const send = (...args) => globalThis.sampleBrowserAction?.(...args);
</script>

{#if preview}
  <div class="browser-preview">
    <span class="browser-preview-name" title={preview.title}>{preview.name}</span>
    {#if preview.ms !== null}<span class="browser-preview-length">Sample preview · {previewLength(preview.ms)}</span>{/if}
    <div class="browser-wave">
      <button type="button" class="browser-play" aria-label={"Play " + preview.name} title={"Play " + preview.name}
        disabled={off || model.busy} onclick={() => send("play")}>{@html Play}</button>
      {#if bars.length}
        <svg viewBox="0 0 {bars.length} 1" preserveAspectRatio="none" aria-hidden="true">
          {#each bars as bar, index (index)}<rect x={bar.x} y={bar.y} width={bar.width} height={bar.height} />{/each}
        </svg>
      {/if}
    </div>
  </div>
{/if}
<div class="browser-actions">
  <button type="button" disabled={off || !model.canAssign} onclick={() => send("assign")}>Use in active lane</button>
  <button type="button" disabled={off || !model.canPick} onclick={() => send("browse")}>Browse…</button>
</div>
