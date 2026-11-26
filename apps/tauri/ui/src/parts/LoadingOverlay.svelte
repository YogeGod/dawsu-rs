<script>
  import { onDestroy } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";

  const busy = $derived(Boolean(bridge.view?.busy));
  const status = $derived(bridge.view?.status || "Reading map…");
  let visible = $state(false);
  let timer = null;

  $effect(() => {
    clearTimeout(timer);
    if (busy) {
      timer = setTimeout(() => {
        if (bridge.view?.busy) visible = true;
      }, 300);
    } else {
      visible = false;
    }
  });

  onDestroy(() => clearTimeout(timer));

  function cancelJob() {
    bridge.callAction("statusBarAction", "cancelJob");
  }
</script>

{#if visible}
  <div class="loading-overlay" role="status" aria-live="polite" aria-label="Loading map">
    <div class="loading-card">
      <div class="loading-spinner" aria-hidden="true"></div>
      <div class="loading-text">
        <strong>Loading…</strong>
        <span>{status}</span>
      </div>
      <button type="button" onclick={cancelJob}>Cancel</button>
    </div>
  </div>
{/if}
