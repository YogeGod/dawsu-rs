<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import { applyArgs } from "../lib/sharedBase.js";

  const scene = $derived(bridge.scene);
  const disabled = $derived(bridge.disabled);
  const difficulties = $derived(scene?.difficulties ?? []);
  const activePath = $derived(scene?.path ?? null);

  let status = $state(null);
  let projections = $state([]);
  let busy = $state(false);
  let lastLoadedPath = null;

  function diffLabel(path) {
    const file = String(path ?? "").split(/[\\/]/).pop() ?? "";
    return file.match(/\[([^\]]+)\]\.osu$/i)?.[1] || file.replace(/\.osu$/i, "");
  }

  function fileNameOf(path) {
    return String(path ?? "").split(/[\\/]/).pop() ?? "";
  }

  const projectionByFile = $derived.by(() => {
    const map = new Map();
    for (const p of projections ?? []) {
      if (p?.file) map.set(p.file.toLowerCase(), p);
    }
    return map;
  });

  const hsDiffPath = $derived.by(() => {
    const hsSet = new Set(scene?.hsDifficulties ?? []);
    for (const p of difficulties) {
      if (hsSet.has(p)) return p;
    }
    return null;
  });

  async function refreshBase() {
    if (!scene?.path) {
      status = null;
      projections = [];
      return;
    }
    try {
      const [nextStatus, nextProj] = await Promise.all([
        invoke("shared_base_action", { action: { kind: "status" } }),
        invoke("shared_base_action", { action: { kind: "diff_projections", leniencyMs: 5 } }).catch(() => []),
      ]);
      status = nextStatus ?? null;
      projections = Array.isArray(nextProj) ? nextProj : [];
    } catch {
      // Non-Tauri test environments without shared_base IPC stay quiet.
    }
  }

  $effect(() => {
    const p = activePath;
    if (p && p !== lastLoadedPath) {
      lastLoadedPath = p;
      refreshBase();
    } else if (!p) {
      lastLoadedPath = null;
      status = null;
      projections = [];
    }
  });

  function switchDiff(path) {
    if (disabled || !path || path === activePath) return;
    act({ kind: "open", path });
  }

  async function captureBase() {
    if (busy || disabled) return;
    busy = true;
    try {
      status = await invoke("shared_base_action", { action: { kind: "capture" } });
      await refreshBase();
      globalThis.showToast?.({ tone: "success", title: "Shared Base captured", body: `${status?.momentCount ?? 0} moments saved to Base.` });
    } catch (err) {
      globalThis.showToast?.({ tone: "error", title: "Shared Base", body: String(err) });
    } finally {
      busy = false;
    }
  }

  async function applyBase(all = false) {
    if (busy || disabled) return;
    busy = true;
    try {
      const args = applyArgs({ leniencyMs: 5, overwriteOverrides: false });
      const res = all
        ? await invoke("shared_base_action", { action: { kind: "apply_all", ...args } })
        : await invoke("shared_base_action", { action: { kind: "apply", ...args } });
      await globalThis.update?.(null, true);
      await refreshBase();
      globalThis.showToast?.({
        tone: "success",
        title: all ? "Applied Base to all diffs" : "Applied Base to difficulty",
        body: res?.summary ?? "Shared Base applied.",
      });
    } catch (err) {
      globalThis.showToast?.({ tone: "error", title: "Apply Base", body: String(err) });
    } finally {
      busy = false;
    }
  }

  function openStudio() {
    globalThis.openSharedBase?.();
  }

  const showStrip = $derived(!!scene && (difficulties.length > 1 || !!status?.enabled));
</script>

{#if showStrip}
  <div class="base-diff-strip" role="tablist" aria-label="Base and difficulties">
    <button
      type="button"
      role="tab"
      class="bds-tab bds-base"
      class:active={!!hsDiffPath && activePath === hsDiffPath}
      class:enabled={!!status?.enabled}
      aria-selected={!!hsDiffPath && activePath === hsDiffPath}
      title={status?.enabled
        ? `Shared Base active (${status.momentCount ?? 0} moments) — click to switch to [HS] diff or open Base Studio`
        : "Open Shared Base Studio"}
      disabled={disabled || busy}
      onclick={() => (hsDiffPath && hsDiffPath !== activePath ? switchDiff(hsDiffPath) : openStudio())}
    >
      <span class="bds-dot" aria-hidden="true"></span>
      Base
      {#if status?.enabled}
        <span class="bds-count">{status.momentCount ?? 0}</span>
      {/if}
    </button>

    <span class="bds-sep" aria-hidden="true"></span>

    <div class="bds-diffs">
      {#each difficulties as path (path)}
        {@const proj = projectionByFile.get(fileNameOf(path).toLowerCase())}
        {@const isCurrent = path === activePath}
        <button
          type="button"
          role="tab"
          class="bds-tab"
          class:active={isCurrent}
          aria-selected={isCurrent}
          disabled={disabled || busy}
          title={proj
            ? `${diffLabel(path)}: ${proj.matched} matched, ${proj.empty} empty, ${proj.ruleCount} rules`
            : diffLabel(path)}
          onclick={() => switchDiff(path)}
        >
          <span class="bds-name">{diffLabel(path)}</span>
          {#if status?.enabled && proj}
            <span class="bds-meta" class:warn={proj.empty > 0}>
              {proj.matched}{#if proj.empty > 0}/{proj.matched + proj.empty}{/if}
            </span>
            {#if proj.ruleCount > 0}
              <span class="bds-rule">{proj.ruleCount}r</span>
            {/if}
          {/if}
        </button>
      {/each}
    </div>

    <div class="bds-actions">
      <button type="button" class="bds-btn" disabled={disabled || busy} onclick={captureBase} title="Capture this difficulty's hitsounds into the Shared Base">
        Capture
      </button>
      {#if status?.enabled}
        <button type="button" class="bds-btn" disabled={disabled || busy} onclick={() => applyBase(false)} title="Apply Shared Base to the open difficulty">
          Apply
        </button>
        <button type="button" class="bds-btn" disabled={disabled || busy} onclick={() => applyBase(true)} title="Apply Shared Base to every difficulty in the mapset">
          Apply all
        </button>
      {/if}
      <button type="button" class="bds-btn bds-studio" disabled={disabled || busy} onclick={openStudio} title="Open Shared Base rules, overrides & volume curve">
        Base Studio…
      </button>
    </div>
  </div>
{/if}

<style>
  .base-diff-strip {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 26px;
    padding: 2px 10px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-panel);
    color: var(--editor-text);
    font: 11px/1 var(--font-ui, sans-serif);
    overflow: hidden;
  }
  .bds-diffs {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1 1 auto;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .bds-diffs::-webkit-scrollbar {
    display: none;
  }
  .bds-tab {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: transparent;
    color: var(--editor-secondary, #9aa4b2);
    font: inherit;
    white-space: nowrap;
    cursor: pointer;
  }
  .bds-tab:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.06);
    color: var(--editor-text, #e6edf3);
  }
  .bds-tab.active {
    border-color: var(--editor-accent, #58a6ff);
    background: rgba(88, 166, 255, 0.14);
    color: var(--editor-text, #e6edf3);
    font-weight: 600;
  }
  .bds-base {
    flex: 0 0 auto;
    font-weight: 600;
  }
  .bds-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--editor-secondary, #6e7681);
  }
  .bds-base.enabled .bds-dot {
    background: #3fb950;
  }
  .bds-count, .bds-meta, .bds-rule {
    padding: 1px 4px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.08);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
  }
  .bds-meta.warn {
    background: rgba(210, 153, 34, 0.22);
    color: #f0b72f;
  }
  .bds-rule {
    background: rgba(88, 166, 255, 0.2);
    color: #79c0ff;
  }
  .bds-sep {
    width: 1px;
    height: 14px;
    background: var(--editor-border, #2d333b);
    flex: 0 0 auto;
  }
  .bds-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
  }
  .bds-btn {
    height: 21px;
    padding: 0 7px;
    border: 1px solid var(--editor-border, #30363d);
    border-radius: 4px;
    background: var(--editor-panel, #1d2128);
    color: var(--editor-text, #e6edf3);
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }
  .bds-btn:hover:not(:disabled) {
    border-color: var(--editor-accent, #58a6ff);
  }
  .bds-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
