<script>
  import { fxSlotSummary, latencyLabel, normalizeMixerFxState, wetDryPercent } from "../lib/mixerFx.js";

  let {
    stripKey,
    stripLabel = "Strip",
    fxState = null,
    disabled = false,
    onclose = null,
    onupdate = null,
    onaction = null,
  } = $props();

  let busy = $state(false);
  let error = $state("");
  let selectedEffectIdx = $state(0);

  const normalized = $derived(normalizeMixerFxState(fxState));
  const slots = $derived(normalized.strips[stripKey] ?? []);
  const effects = $derived(normalized.availableEffects);
  const summary = $derived(fxSlotSummary(slots));

  async function dispatch(action) {
    if (disabled || busy) return;
    error = "";
    busy = true;
    try {
      let next = null;
      if (onaction) {
        next = await onaction(action);
      } else if (typeof window !== "undefined" && typeof window.__TAURI__?.core?.invoke === "function") {
        next = await window.__TAURI__.core.invoke("mixer_fx_action", { action });
      }
      if (next && typeof next === "object") {
        onupdate?.(next);
      }
    } catch (err) {
      error = typeof err === "string" ? err : err?.message ?? String(err);
    } finally {
      busy = false;
    }
  }

  function addSelectedInsert() {
    const picked = effects[selectedEffectIdx];
    if (!picked) return;
    dispatch({
      kind: "add_insert",
      strip: stripKey,
      plugin_path: picked.path,
      plugin_uid: picked.uid ?? null,
      name: picked.name ?? null,
    });
  }
</script>

<section class="fx-wrapper" aria-label={stripLabel + " inserts"}>
  <header class="fx-head">
    <div class="fx-title-group">
      <h4>{stripLabel} FX</h4>
      <span class="fx-pdc" title="Total compensated graph latency">
        PDC {latencyLabel(normalized.totalPdcSamples, normalized.sampleRate)} · Strip {latencyLabel(summary.totalLatency, normalized.sampleRate)}
      </span>
    </div>
    <div class="fx-head-actions">
      <button type="button" class="fx-btn" aria-label="Undo FX change"
        disabled={disabled || busy || !normalized.canUndo}
        onclick={() => dispatch({ kind: "undo" })}>Undo</button>
      <button type="button" class="fx-btn" aria-label="Redo FX change"
        disabled={disabled || busy || !normalized.canRedo}
        onclick={() => dispatch({ kind: "redo" })}>Redo</button>
      <button type="button" class="fx-btn" aria-label="Close FX panel"
        {disabled}
        onclick={() => onclose?.()}>×</button>
    </div>
  </header>

  {#if error}
    <p class="fx-error" role="alert">{error}</p>
  {/if}

  <div class="fx-list">
    {#if slots.length === 0}
      <p class="fx-empty">No VST3 insert effects on this strip yet.</p>
    {:else}
      {#each slots as slot, idx (slot.slotId)}
        <div class="fx-slot" class:bypassed={slot.bypassed}>
          <div class="fx-slot-top">
            <button type="button" class="fx-btn power" aria-label={"Soft bypass " + slot.name}
              aria-pressed={!slot.bypassed} disabled={disabled || busy}
              onclick={() => dispatch({ kind: "set_bypass", strip: stripKey, slot_id: slot.slotId, bypassed: !slot.bypassed })}>
              {slot.bypassed ? "Off" : "On"}
            </button>
            <span class="fx-name" title={slot.pluginPath}>{slot.name}</span>
            {#if !slot.available}
              <span class="fx-badge warn" title={slot.unavailableReason ?? "Unavailable"}>Offline</span>
            {:else if slot.latencySamples > 0}
              <span class="fx-badge">{slot.latencySamples} smp</span>
            {/if}
            <div class="fx-move">
              <button type="button" class="fx-btn" aria-label={"Move " + slot.name + " up"}
                disabled={disabled || busy || idx === 0}
                onclick={() => dispatch({ kind: "move_insert", strip: stripKey, slot_id: slot.slotId, to_index: idx - 1 })}>↑</button>
              <button type="button" class="fx-btn" aria-label={"Move " + slot.name + " down"}
                disabled={disabled || busy || idx + 1 >= slots.length}
                onclick={() => dispatch({ kind: "move_insert", strip: stripKey, slot_id: slot.slotId, to_index: idx + 1 })}>↓</button>
              <button type="button" class="fx-btn danger" aria-label={"Remove " + slot.name}
                disabled={disabled || busy}
                onclick={() => dispatch({ kind: "remove_insert", strip: stripKey, slot_id: slot.slotId })}>✕</button>
            </div>
          </div>
          <div class="fx-slot-controls">
            <button type="button" class="fx-btn" aria-label={"Toggle A/B bank for " + slot.name}
              aria-pressed={slot.activeBank === "b"} disabled={disabled || busy}
              onclick={() => dispatch({ kind: "switch_ab", strip: stripKey, slot_id: slot.slotId })}>
              {slot.activeBank === "b" ? "B" : "A"}
            </button>
            <button type="button" class="fx-btn" aria-label={"Copy bank for " + slot.name}
              disabled={disabled || busy}
              onclick={() => dispatch({ kind: "copy_ab", strip: stripKey, slot_id: slot.slotId })}>
              {slot.activeBank === "b" ? "B→A" : "A→B"}
            </button>
            <label class="fx-mix">
              <span class="fx-mix-caption">Mix</span>
              <input type="range" min="0" max="100" step="1"
                value={wetDryPercent(slot.wetDry)}
                aria-label={"Wet dry mix for " + slot.name}
                disabled={disabled || busy}
                onchange={e => dispatch({
                  kind: "set_wet_dry",
                  strip: stripKey,
                  slot_id: slot.slotId,
                  wet_dry: Number(e.currentTarget.value) / 100,
                })} />
              <span class="fx-mix-pct">{wetDryPercent(slot.wetDry)}%</span>
            </label>
            <button type="button" class="fx-btn" aria-label={(slot.editorOpen ? "Close UI for " : "Open UI for ") + slot.name}
              aria-pressed={slot.editorOpen} disabled={disabled || busy || !slot.available}
              onclick={() => dispatch({
                kind: slot.editorOpen ? "close_editor" : "open_editor",
                strip: stripKey,
                slot_id: slot.slotId,
              })}>UI</button>
            <button type="button" class="fx-btn" aria-label={"Pin window for " + slot.name}
              aria-pressed={slot.pinned} disabled={disabled || busy}
              onclick={() => dispatch({ kind: "set_pinned", strip: stripKey, slot_id: slot.slotId, pinned: !slot.pinned })}>Pin</button>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <footer class="fx-add">
    {#if effects.length > 0}
      <select class="fx-select" aria-label="Choose VST3 effect" disabled={disabled || busy}
        bind:value={selectedEffectIdx}>
        {#each effects as effect, idx (effect.path + "|" + (effect.uid ?? idx))}
          <option value={idx}>{effect.name}{effect.vendor ? " — " + effect.vendor : ""}</option>
        {/each}
      </select>
      <button type="button" class="fx-btn add" disabled={disabled || busy || slots.length >= 8}
        onclick={addSelectedInsert}>Add FX</button>
    {:else}
      <span class="fx-scan-hint">Scan VST3 folders in Preferences to list effects.</span>
    {/if}
  </footer>
</section>

<style>
  .fx-wrapper {
    flex: none;
    width: 316px;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    box-sizing: border-box;
    border-radius: 4px;
    background: var(--editor-panel);
    box-shadow: var(--shadow-card);
    overflow: hidden;
  }
  .fx-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 6px;
    flex: none;
  }
  .fx-title-group {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  h4 {
    margin: 0;
    font: 600 12px var(--font-ui);
    color: var(--editor-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fx-pdc {
    font: 10px var(--font-ui);
    color: var(--editor-secondary);
    font-variant-numeric: tabular-nums;
  }
  .fx-head-actions {
    display: flex;
    gap: 4px;
    flex: none;
  }
  .fx-error {
    margin: 0;
    padding: 4px 6px;
    border-radius: 3px;
    background: rgba(255, 69, 58, 0.16);
    color: var(--editor-error, #ff453a);
    font: 11px var(--font-ui);
  }
  .fx-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
    scrollbar-width: thin;
  }
  .fx-empty,
  .fx-scan-hint {
    margin: 0;
    font: 11px var(--font-ui);
    color: var(--editor-muted);
  }
  .fx-slot {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 5px 6px;
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.32);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
  }
  .fx-slot.bypassed {
    opacity: 0.68;
  }
  .fx-slot-top,
  .fx-slot-controls,
  .fx-move,
  .fx-add {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .fx-name {
    flex: 1;
    min-width: 0;
    font: 600 11px var(--font-ui);
    color: var(--editor-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fx-badge {
    padding: 1px 4px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.08);
    color: var(--editor-secondary);
    font: 10px var(--font-ui);
    font-variant-numeric: tabular-nums;
  }
  .fx-badge.warn {
    background: rgba(255, 159, 10, 0.18);
    color: var(--editor-warning, #ffd60a);
  }
  .fx-mix {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    font: 10px var(--font-ui);
    color: var(--editor-secondary);
  }
  .fx-mix input[type="range"] {
    flex: 1;
    min-width: 44px;
    margin: 0;
    accent-color: var(--editor-accent);
  }
  .fx-mix-pct {
    width: 30px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--editor-readout);
  }
  .fx-btn {
    height: 19px;
    min-height: 19px;
    padding: 0 5px;
    border: 0;
    border-radius: 3px;
    background: var(--editor-control);
    color: var(--editor-secondary);
    font: 600 10px var(--font-ui);
    cursor: pointer;
    flex: none;
  }
  .fx-btn[aria-pressed="true"],
  .fx-btn.add {
    background: var(--editor-accent);
    color: var(--editor-ink);
  }
  .fx-btn.danger:hover:not(:disabled) {
    color: var(--editor-error, #ff453a);
  }
  .fx-btn:disabled {
    opacity: 0.42;
    cursor: default;
  }
  .fx-add {
    flex: none;
  }
  .fx-select {
    flex: 1;
    min-width: 0;
    height: 21px;
    border-radius: 3px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: var(--editor-control);
    color: var(--editor-text);
    font: 11px var(--font-ui);
  }
</style>
