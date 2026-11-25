<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { additionPills, parseVolumeDisplay, sampleSetPills } from "../lib/inspector.js";
  import { clampMicroHud, KEYSOUND_CHORD_POLICIES, microHudMode } from "../lib/microHud.js";

  const hud = $derived(bridge.microHud);
  const mode = $derived(microHudMode(hud?.mode ?? bridge.mode));
  const selection = $derived(bridge.inspector?.selection ?? null);
  const setPills = $derived(sampleSetPills(selection));
  const addPills = $derived(additionPills(selection));
  const vol = $derived(parseVolumeDisplay(selection?.volume).value);
  const pos = $derived(
    hud
      ? clampMicroHud(hud.x, hud.y, globalThis.window?.innerWidth ?? 1280, globalThis.window?.innerHeight ?? 720)
      : { left: 0, top: 0 },
  );

  $effect(() => {
    if (!selection && bridge.microHud) {
      bridge.microHud = null;
    }
  });

  function dismiss() {
    bridge.microHud = null;
  }

  function onKeydown(event) {
    if (event.key === "Escape" && hud) dismiss();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if hud && selection}
  <div class="micro-hud" role="toolbar" aria-label="Quick edit selection"
    style:--hud-left="{pos.left}px" style:--hud-top="{pos.top}px">
    {#if mode === "hitsounds"}
      <span class="hud-val">{vol}%</span>
      <div class="hud-group">
        {#each setPills as pill (pill.id)}
          <button type="button" class="hud-btn" class:on={pill.active} disabled={pill.disabled}
            title={pill.title} onclick={() => globalThis.inspectorAction?.("sampleSet", pill.id)}>{pill.label[0]}</button>
        {/each}
      </div>
      <div class="hud-group">
        {#each addPills as pill (pill.id)}
          <button type="button" class="hud-btn" class:on={pill.active} disabled={pill.disabled}
            title={pill.title} onclick={() => globalThis.inspectorAction?.("addition", pill.id)}>{pill.label}</button>
        {/each}
      </div>
      <button type="button" class="hud-btn" aria-label="Audition" title="Audition selected hits"
        onclick={() => globalThis.inspectorAction?.("auditionSelection")}>▶</button>
    {:else if mode === "drums"}
      <span class="hud-val">Vel {Math.max(1, Math.min(127, Math.round(vol * 1.27)))}</span>
      <button type="button" class="hud-btn" aria-label="Audition drum hit"
        onclick={() => globalThis.inspectorAction?.("auditionSelection")}>▶</button>
    {:else}
      <span class="hud-val">{hud.noteLabel || "Note"}</span>
      <div class="hud-group">
        {#each KEYSOUND_CHORD_POLICIES as policy (policy.id)}
          <button type="button" class="hud-btn" class:on={hud.chordPolicy === policy.id}
            onclick={() => globalThis.keysoundsMicroAction?.("chordPolicy", policy.id)}>{policy.label}</button>
        {/each}
      </div>
      <button type="button" class="hud-btn" aria-label="Audition note"
        onclick={() => globalThis.keysoundsMicroAction?.("previewNote")}>▶</button>
    {/if}
  </div>
{/if}

<style>
  .micro-hud {
    position: fixed;
    left: var(--hud-left);
    top: var(--hud-top);
    z-index: 35;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border: 1px solid var(--editor-border);
    border-radius: 999px;
    background: rgba(21, 21, 23, 0.94);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    color: var(--editor-text);
    font-size: 11px;
  }
  .hud-val {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    padding: 0 4px;
  }
  .hud-group {
    display: inline-flex;
    gap: 2px;
  }
  .hud-btn {
    min-width: 22px;
    min-height: 22px;
    padding: 0 6px;
    border: 1px solid var(--editor-border);
    border-radius: 999px;
    background: var(--editor-panel);
    color: var(--editor-secondary);
    font-size: 11px;
    cursor: pointer;
  }
  .hud-btn.on {
    border-color: var(--editor-accent);
    background: var(--editor-selected);
    color: var(--editor-accent);
  }
</style>
