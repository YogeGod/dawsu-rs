<script>
  import { onMount } from "svelte";
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    DEFAULT_LENIENCY_MS,
    MAX_LENIENCY_MS,
    applyArgs,
    formatApplyAllSummary,
    formatApplySummary,
    formatStatusLine,
  } from "../lib/sharedBase.js";

  // Map › Shared Base layer…: optional per-mapset base hitsound layer stored in app-data.
  // menus.js calls globalThis.openSharedBase.
  let open = $state(false);
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let status = $state({
    enabled: false,
    hasBase: false,
    sourceDifficulty: null,
    baseMomentCount: 0,
    currentDiffOverrides: 0,
    currentDiffInherited: 0,
  });
  let leniencyMs = $state(DEFAULT_LENIENCY_MS);
  let overwriteOverrides = $state(false);

  const hasDoc = $derived(Boolean(bridge.scene));

  async function refreshStatus() {
    const next = await invoke("shared_base_status");
    if (next) status = next;
  }

  async function openSharedBase() {
    if (!hasDoc) return;
    error = "";
    notice = "";
    open = true;
    busy = true;
    try {
      await refreshStatus();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function toggleEnabled(event) {
    if (busy) return;
    const enabled = Boolean(event.currentTarget.checked);
    busy = true;
    error = "";
    notice = "";
    try {
      const next = await invoke("shared_base_set_enabled", { enabled });
      if (next) status = next;
      notice = enabled
        ? "Shared Base is on for this mapset. Edits on this diff are tracked as overrides."
        : "Shared Base is off for this mapset.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function captureBase() {
    if (busy) return;
    busy = true;
    error = "";
    notice = "";
    try {
      const next = await invoke("shared_base_capture");
      if (next) status = next;
      notice = `Captured ${next?.baseMomentCount ?? 0} moments as the mapset Shared Base.`;
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function applyBase() {
    if (busy || !status.hasBase) return;
    busy = true;
    error = "";
    notice = "";
    try {
      const result = await invoke("shared_base_apply", applyArgs({ leniencyMs, overwriteOverrides }));
      if (result?.status) status = result.status;
      notice = formatApplySummary(result);
      await act(null);
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function applyBaseAll() {
    if (busy || !status.hasBase) return;
    busy = true;
    error = "";
    notice = "";
    try {
      const result = await invoke("shared_base_apply_all", applyArgs({ leniencyMs, overwriteOverrides }));
      if (result?.status) status = result.status;
      notice = formatApplyAllSummary(result);
      if (Array.isArray(result?.failedDiffs) && result.failedDiffs.length > 0) {
        error = result.failedDiffs.join(" | ");
      }
      await act(null);
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function resetOverrides() {
    if (busy) return;
    busy = true;
    error = "";
    notice = "";
    try {
      const next = await invoke("shared_base_clear_overrides");
      if (next) status = next;
      notice = "Cleared per-diff overrides for this difficulty.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function clearBase() {
    if (busy) return;
    busy = true;
    error = "";
    notice = "";
    try {
      const next = await invoke("shared_base_clear");
      if (next) status = next;
      notice = "Cleared Shared Base for this mapset.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  function closeModal() {
    if (busy) return;
    open = false;
  }

  function onKeydown(event) {
    if (open && event.key === "Escape") {
      event.stopPropagation();
      closeModal();
    }
  }

  $effect(() => {
    globalThis.dawsuModalOpen = open;
    return () => { globalThis.dawsuModalOpen = false; };
  });

  onMount(() => {
    globalThis.openSharedBase = openSharedBase;
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div class="sb-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal(); }}>
    <div class="sb-modal" role="dialog" aria-modal="true" aria-label="Shared Base layer">
      <header class="sb-head">
        <span class="sb-title">Shared Base layer</span>
        <button type="button" class="sb-close" aria-label="Close" onclick={closeModal}>×</button>
      </header>

      <div class="sb-body">
        <label class="sb-check">
          <input type="checkbox" checked={status.enabled} disabled={busy} onchange={toggleEnabled} />
          <span>Use Shared Base for this mapset</span>
        </label>

        <div class="sb-summary">
          <div class="sb-summary-main">{formatStatusLine(status)}</div>
          {#if status.hasBase}
            <div class="sb-summary-counts">
              <span>Inherited on this diff: {status.currentDiffInherited}</span>
              <span>Overridden on this diff: {status.currentDiffOverrides}</span>
            </div>
          {/if}
        </div>

        <div class="sb-section">
          <div class="sb-row">
            <button type="button" disabled={busy} onclick={captureBase}>
              Capture current diff as Base
            </button>
          </div>
        </div>

        <div class="sb-section sb-divider">
          <label class="sb-field">
            <span>Leniency (ms)</span>
            <input
              type="number"
              min="0"
              max={MAX_LENIENCY_MS}
              step="1"
              disabled={busy || !status.hasBase}
              bind:value={leniencyMs}
            />
          </label>
          <label class="sb-check">
            <input
              type="checkbox"
              disabled={busy || !status.hasBase}
              bind:checked={overwriteOverrides}
            />
            <span>Overwrite per-diff overrides</span>
          </label>
          <div class="sb-row">
            <button
              type="button"
              class="sb-primary"
              disabled={busy || !status.hasBase}
              onclick={applyBase}
            >
              {busy ? "Working…" : "Apply Base to this diff"}
            </button>
            <button
              type="button"
              disabled={busy || !status.hasBase}
              onclick={applyBaseAll}
            >
              {busy ? "Working…" : "Apply Base to all diffs"}
            </button>
          </div>
        </div>

        <div class="sb-section sb-divider sb-actions-row">
          <button
            type="button"
            disabled={busy || status.currentDiffOverrides === 0}
            onclick={resetOverrides}
          >
            Reset this diff's overrides
          </button>
          <button
            type="button"
            class="sb-danger"
            disabled={busy || (!status.hasBase && !status.enabled)}
            onclick={clearBase}
          >
            Clear Shared Base
          </button>
        </div>

        {#if error}
          <p class="sb-error" role="alert">{error}</p>
        {:else if notice}
          <p class="sb-notice" role="status">{notice}</p>
        {/if}
      </div>

      <footer class="sb-foot">
        <button type="button" disabled={busy} onclick={closeModal}>Close</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .sb-backdrop {
    position: fixed;
    inset: 0;
    z-index: 62;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.68);
  }
  .sb-modal {
    display: flex;
    flex-direction: column;
    width: min(500px, calc(100vw - 40px));
    max-height: min(580px, calc(100vh - 48px));
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.62);
    overflow: hidden;
  }
  .sb-head {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 50px;
    padding: 0 18px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .sb-title {
    font-size: 14px;
    font-weight: 600;
  }
  .sb-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    font-size: 18px;
  }
  .sb-body {
    flex: 1;
    padding: 18px 20px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .sb-summary {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: 7px;
    background: var(--editor-canvas);
    border: 1px solid var(--editor-border);
    font-size: 12px;
    color: var(--editor-secondary);
    font-variant-numeric: tabular-nums;
  }
  .sb-summary-main {
    color: var(--editor-text);
    font-weight: 500;
  }
  .sb-summary-counts {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    color: var(--editor-muted);
  }
  .sb-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .sb-divider {
    padding-top: 12px;
    border-top: 1px solid var(--editor-border);
  }
  .sb-check {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
  }
  .sb-field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
    color: var(--editor-secondary);
  }
  .sb-field input {
    width: 84px;
    padding: 4px 8px;
    border: 1px solid var(--editor-border);
    border-radius: 5px;
    background: var(--editor-canvas);
    color: var(--editor-text);
    font-variant-numeric: tabular-nums;
  }
  .sb-row {
    display: flex;
    gap: 8px;
  }
  .sb-actions-row {
    flex-direction: row;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 8px;
  }
  .sb-primary {
    border-color: var(--editor-accent);
    background: var(--editor-accent);
    color: #0b0e13;
    font-weight: 600;
  }
  .sb-danger {
    color: #ff8e8e;
  }
  .sb-notice {
    margin: 0;
    padding: 8px 10px;
    border-radius: 6px;
    background: rgba(79, 168, 255, 0.12);
    color: var(--editor-text);
    font-size: 12px;
  }
  .sb-error {
    margin: 0;
    padding: 8px 10px;
    border-radius: 6px;
    background: rgba(255, 92, 92, 0.14);
    color: #ff9c9c;
    font-size: 12px;
  }
  .sb-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
</style>
