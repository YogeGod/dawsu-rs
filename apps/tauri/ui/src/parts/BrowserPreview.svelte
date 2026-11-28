<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyBrowser, outlineBars, previewLength } from "../lib/browser.js";
  import Play from "../icons/Play.svg?raw";

  const model = $derived(bridge.browser ?? emptyBrowser);
  const off = $derived(!bridge.scene || !bridge.browser);
  const preview = $derived(model.preview);
  // The outline is the sample's own loudness, decoded by the backend; without it no bars are drawn.
  const bars = $derived(outlineBars(preview?.peaks));
  const send = (...args) => bridge.callAction("sampleBrowserAction", ...args);
  const useLabel = $derived(model.target ? "Use in " + model.target : "Use in active lane");
</script>

{#if preview}
  <div class="browser-preview">
    <div class="browser-preview-head">
      <span class="browser-preview-name" title={preview.title}>{preview.name}</span>
      <button type="button" class="browser-similar-btn" disabled={off || model.busy}
        onclick={() => send("similar", preview.title)}>Find similar sound</button>
    </div>
    <!-- The line keeps its place while the length loads, so the outline doesn't jump. -->
    <span class="browser-preview-length">{#if preview.ms !== null}Sample preview · {previewLength(preview.ms)}{/if}</span>
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
<!-- Use in <lane> is last, so it takes the full-width row: its label grows with the lane's name. -->
<div class="browser-actions">
  <button type="button" id="browser-add-lane" disabled={off || !model.canAddAsLane} onclick={() => send("addLane")}>+ Add as lane</button>
  <button type="button" disabled={off || !(model.canPick || model.canBrowseAdd)} onclick={() => send("browse")}>Browse…</button>
  <button type="button" disabled={off || !model.canAssign} title={useLabel} onclick={() => send("assign")}>{useLabel}</button>
</div>
