<script>
  import { onMount, tick } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { items, keyOnly, menus } from "../lib/commands.js";
  import { invoke } from "../lib/ipc.js";
  import {
    buildEntries,
    filterAndRank,
    firstEnabledIndex,
    nextEnabledIndex,
    readRecent,
    recordRecent,
  } from "../lib/commandPalette.js";

  const storage = () => {
    try {
      return globalThis.localStorage;
    } catch {
      return null;
    }
  };

  let query = $state("");
  let selectedIndex = $state(-1);
  let recentIds = $state(readRecent(storage()));
  let inputEl = $state(null);
  let listEl = $state(null);
  let previouslyFocused = null;

  const open = $derived(Boolean(bridge.commandPaletteOpen));
  const view = $derived(bridge.view ?? null);
  const scene = $derived(bridge.scene ?? null);
  const shortcuts = $derived(bridge.commandPaletteShortcuts);

  const allEntries = $derived.by(() => {
    if (!open) return [];
    const paletteMenu = globalThis.dawsuPaletteMenu;
    return buildEntries({
      items,
      menus,
      keyOnly,
      shortcuts,
      scene,
      view,
      isEnabled: paletteMenu?.isEnabled ?? null,
      isChecked: paletteMenu?.isChecked ?? null,
    });
  });

  const visibleEntries = $derived.by(() => {
    if (!open) return [];
    return filterAndRank(allEntries, query, recentIds);
  });

  // Keep selectedIndex pointing at an enabled row whenever the filtered list changes.
  $effect(() => {
    if (!open) return;
    const list = visibleEntries;
    if (selectedIndex < 0 || selectedIndex >= list.length || !list[selectedIndex]?.enabled) {
      selectedIndex = firstEnabledIndex(list);
    }
  });

  // Expose open flag so boot.js keeps timeline shortcuts off while the palette is open.
  $effect(() => {
    globalThis.dawsuCommandPaletteOpen = open;
    return () => {
      globalThis.dawsuCommandPaletteOpen = false;
    };
  });

  async function refreshShortcuts() {
    try {
      const res = await invoke("editor_shortcuts");
      if (res) bridge.commandPaletteShortcuts = res;
    } catch {
      // Palette remains usable even if shortcuts IPC is unavailable.
    }
  }

  async function openPalette() {
    if (open) return;
    previouslyFocused = globalThis.document?.activeElement ?? null;
    globalThis.closeEditorMenu?.(false);
    query = "";
    recentIds = readRecent(storage());
    bridge.commandPaletteOpen = true;
    refreshShortcuts();
    await tick();
    inputEl?.focus?.();
    inputEl?.select?.();
  }

  function restoreFocus() {
    const target = previouslyFocused;
    previouslyFocused = null;
    if (target && typeof target.focus === "function" && globalThis.document?.contains?.(target)) {
      target.focus();
      return;
    }
    globalThis.document?.getElementById?.("grid")?.focus?.();
  }

  function closePalette(shouldRestoreFocus = true) {
    if (!open) return;
    bridge.commandPaletteOpen = false;
    query = "";
    selectedIndex = -1;
    if (shouldRestoreFocus) restoreFocus();
    else previouslyFocused = null;
  }

  function togglePalette() {
    if (open) closePalette(true);
    else openPalette();
  }

  function scrollSelectedIntoView(index) {
    if (index < 0 || !listEl) return;
    const row = listEl.querySelector?.(`[data-palette-index="${index}"]`);
    row?.scrollIntoView?.({ block: "nearest" });
  }

  function executeEntry(entry) {
    if (!entry || !entry.enabled) return;
    recentIds = recordRecent(storage(), entry.id, recentIds);
    closePalette(false);

    if (entry.source === "menu" && entry.item) {
      globalThis.dawsuPaletteMenu?.runItem?.(entry.item);
    } else if (entry.source === "keyOnly" && entry.command) {
      globalThis.command?.(entry.command);
    }

    // If the command did not open a dialog or focus another input, return focus cleanly to the editor.
    queueMicrotask(() => {
      const active = globalThis.document?.activeElement;
      const inDialog = Boolean(
        globalThis.dawsuPreferencesOpen ||
        globalThis.dawsuModalOpen ||
        globalThis.document?.getElementById?.("dialog")?.open ||
        active?.closest?.("input, textarea, select, [role=dialog], dialog"),
      );
      if (!inDialog) {
        globalThis.document?.getElementById?.("grid")?.focus?.();
      }
    });
  }

  function onInputKeydown(event) {
    if (!open) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closePalette(true);
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      const next = nextEnabledIndex(visibleEntries, selectedIndex, 1);
      if (next >= 0) {
        selectedIndex = next;
        scrollSelectedIntoView(next);
      }
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      const next = nextEnabledIndex(visibleEntries, selectedIndex, -1);
      if (next >= 0) {
        selectedIndex = next;
        scrollSelectedIntoView(next);
      }
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      const next = firstEnabledIndex(visibleEntries);
      if (next >= 0) {
        selectedIndex = next;
        scrollSelectedIntoView(next);
      }
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      const next = nextEnabledIndex(visibleEntries, 0, -1);
      if (next >= 0) {
        selectedIndex = next;
        scrollSelectedIntoView(next);
      }
      return;
    }
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      const chosen = visibleEntries[selectedIndex];
      if (chosen?.enabled) executeEntry(chosen);
    }
  }

  function onGlobalKeydown(event) {
    if (event.isComposing || event.repeat) return;
    if (globalThis.dawsuShortcutCapturing) return;

    if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "k") {
      event.preventDefault();
      event.stopPropagation();
      togglePalette();
      return;
    }

    if (open && event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closePalette(true);
    }
  }

  onMount(() => {
    globalThis.openCommandPalette = openPalette;
    globalThis.toggleCommandPalette = togglePalette;
    globalThis.addEventListener("keydown", onGlobalKeydown, true);
    return () => {
      if (globalThis.openCommandPalette === openPalette) delete globalThis.openCommandPalette;
      if (globalThis.toggleCommandPalette === togglePalette) delete globalThis.toggleCommandPalette;
      globalThis.removeEventListener("keydown", onGlobalKeydown, true);
    };
  });
</script>

{#if open}
  <div class="cmdp-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closePalette(true); }}>
    <div class="cmdp-modal" role="dialog" aria-modal="true" aria-label="Command palette">
      <div class="cmdp-search-row">
        <svg class="cmdp-search-icon" viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="11" cy="11" r="8" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <input
          bind:this={inputEl}
          type="text"
          class="cmdp-input"
          placeholder="Type a command or shortcut…"
          aria-label="Search commands"
          aria-autocomplete="list"
          aria-controls="cmdp-listbox"
          aria-activedescendant={selectedIndex >= 0 && visibleEntries[selectedIndex] ? `cmdp-opt-${selectedIndex}` : undefined}
          bind:value={query}
          onkeydown={onInputKeydown}
        />
        <kbd class="cmdp-esc-chip">Esc</kbd>
      </div>

      <div id="cmdp-listbox" class="cmdp-list" role="listbox" aria-label="Commands" bind:this={listEl}>
        {#if visibleEntries.length === 0}
          <div class="cmdp-empty">No matching commands</div>
        {:else}
          {#each visibleEntries as entry, index (entry.id)}
            <button
              id={`cmdp-opt-${index}`}
              type="button"
              role="option"
              data-palette-index={index}
              class="cmdp-row"
              class:active={index === selectedIndex}
              class:disabled={!entry.enabled}
              aria-selected={index === selectedIndex}
              aria-disabled={!entry.enabled}
              disabled={!entry.enabled}
              onmouseenter={() => { if (entry.enabled) selectedIndex = index; }}
              onclick={() => executeEntry(entry)}
            >
              <span class="cmdp-icon-slot" aria-hidden="true">
                {#if entry.icon}
                  <img class="cmdp-icon" src={`studio-assets/icons/${entry.icon}.svg`} alt="" />
                {/if}
              </span>
              <span class="cmdp-main">
                <span class="cmdp-label">{entry.label}</span>
                {#if entry.checked === true}
                  <span class="cmdp-state-dot" title="Active" aria-label="Active"></span>
                {/if}
                {#if !query.trim() && entry.recent}
                  <span class="cmdp-badge">Recent</span>
                {/if}
              </span>
              <span class="cmdp-group">{entry.group}</span>
              {#if entry.shortcutText}
                <kbd class="cmdp-key">{entry.shortcutText}</kbd>
              {/if}
            </button>
          {/each}
        {/if}
      </div>

      <footer class="cmdp-foot" aria-hidden="true">
        <span><kbd>↑</kbd><kbd>↓</kbd> Navigate</span>
        <span><kbd>↵</kbd> Run</span>
        <span><kbd>Esc</kbd> Close</span>
      </footer>
    </div>
  </div>
{/if}

<style>
  .cmdp-backdrop {
    position: fixed;
    inset: 0;
    z-index: 950;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 13vh;
    background: rgba(0, 0, 0, 0.56);
  }
  .cmdp-modal {
    width: 560px;
    max-width: calc(100vw - 32px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: var(--radius-8, 8px);
    border: 1px solid var(--editor-border, #2b2b30);
    background: var(--editor-panel, #151517);
    color: var(--editor-text, #f5f5f7);
    box-shadow: var(--shadow-pop, 0 14px 36px rgba(0, 0, 0, 0.55));
    font-family: var(--font-ui, Inter, system-ui, sans-serif);
  }
  .cmdp-search-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    background: var(--editor-chrome, #1c1c1f);
    border-bottom: 1px solid var(--editor-border, #2b2b30);
  }
  .cmdp-search-icon {
    width: 16px;
    height: 16px;
    flex: none;
    fill: none;
    stroke: var(--editor-muted, #98989f);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .cmdp-input {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--editor-text, #f5f5f7);
    font-family: inherit;
    font-size: 14px;
  }
  .cmdp-input::placeholder {
    color: var(--editor-muted, #98989f);
  }
  .cmdp-esc-chip {
    flex: none;
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--editor-border, #2b2b30);
    background: var(--editor-canvas, #0e0e10);
    color: var(--editor-muted, #98989f);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10px;
  }
  .cmdp-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: min(400px, 54vh);
    overflow-y: auto;
    padding: 6px;
  }
  .cmdp-empty {
    padding: 24px 12px;
    text-align: center;
    font-size: 12px;
    color: var(--editor-muted, #98989f);
  }
  .cmdp-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 34px;
    padding: 6px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-6, 6px);
    background: transparent;
    color: var(--editor-text, #f5f5f7);
    font-family: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .cmdp-row:hover:not(:disabled) {
    background: var(--editor-hover, #303036);
  }
  .cmdp-row.active:not(:disabled) {
    background: var(--editor-rowfocus, #084266);
    border-color: var(--editor-accent, #5ac8fa);
  }
  .cmdp-row.disabled,
  .cmdp-row:disabled {
    color: var(--editor-disabled, #6e6e75);
    cursor: default;
  }
  .cmdp-icon-slot {
    width: 16px;
    height: 16px;
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .cmdp-icon {
    width: 14px;
    height: 14px;
    opacity: 0.85;
  }
  .cmdp-row.disabled .cmdp-icon {
    opacity: 0.35;
  }
  .cmdp-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cmdp-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cmdp-state-dot {
    width: 6px;
    height: 6px;
    flex: none;
    border-radius: 50%;
    background: var(--editor-accent, #5ac8fa);
  }
  .cmdp-badge {
    flex: none;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--editor-control, #242428);
    border: 1px solid var(--editor-border, #2b2b30);
    color: var(--editor-accent, #5ac8fa);
    font-size: 10px;
    font-weight: 500;
  }
  .cmdp-group {
    flex: none;
    font-size: 11px;
    color: var(--editor-muted, #98989f);
  }
  .cmdp-row.active .cmdp-group {
    color: var(--editor-secondary, #c7c7cc);
  }
  .cmdp-key {
    flex: none;
    padding: 2px 7px;
    border-radius: 4px;
    border: 1px solid var(--editor-border, #2b2b30);
    background: var(--editor-canvas, #0e0e10);
    color: var(--editor-readout, #f5f5f7);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .cmdp-row.disabled .cmdp-key {
    color: var(--editor-disabled, #6e6e75);
  }
  .cmdp-foot {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 8px 14px;
    border-top: 1px solid var(--editor-border, #2b2b30);
    background: var(--editor-chrome, #1c1c1f);
    color: var(--editor-muted, #98989f);
    font-size: 11px;
  }
  .cmdp-foot span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .cmdp-foot kbd {
    padding: 1px 5px;
    border-radius: 3px;
    border: 1px solid var(--editor-border, #2b2b30);
    background: var(--editor-canvas, #0e0e10);
    color: var(--editor-secondary, #c7c7cc);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10px;
  }
</style>
