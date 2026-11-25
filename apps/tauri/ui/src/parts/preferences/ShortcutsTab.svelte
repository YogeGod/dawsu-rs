<script>
  import { shortcutName } from "../../editor/helpers.js";

  let {
    shortcuts = [],
    preset = "DAW",
    shortcutQuery = $bindable(""),
    capturingId = $bindable(null),
    onpickpreset,
    onresetall,
    onshortcutkeydown,
    onresetshortcut,
  } = $props();

  // Rust sends each key as its formatted name ("Ctrl+O"); an older shape may still be a chord object.
  function keyLabel(key) {
    if (typeof key === "string") return key;
    if (key && typeof key.code === "string") return shortcutName(key) || key.key || "";
    return "";
  }

  function keysLabel(keys) {
    const names = (keys ?? []).map(keyLabel).filter(Boolean);
    return names.length ? names.join(", ") : "Unbound";
  }

  const filteredShortcuts = $derived.by(() => {
    const q = shortcutQuery.trim().toLowerCase();
    if (!q) return shortcuts;
    return shortcuts.filter(s =>
      (s.label || s.id || "").toLowerCase().includes(q) ||
      (s.keys || []).map(keyLabel).join(" ").toLowerCase().includes(q)
    );
  });
</script>

<div class="pref-group pref-shortcuts">
  <div class="pref-shortcut-bar">
    <div class="pref-seg">
      <button type="button" class:active={preset === "osu!"} onclick={() => onpickpreset("osu!")}>osu!</button>
      <button type="button" class:active={preset === "DAW"} onclick={() => onpickpreset("DAW")}>DAW-like</button>
    </div>
    <input type="search" class="pref-search" placeholder="Search commands or keys" bind:value={shortcutQuery} />
    <button type="button" onclick={onresetall}>Reset all</button>
  </div>
  <div class="pref-shortcut-list">
    {#each filteredShortcuts as entry (entry.id)}
      <div class="pref-shortcut-row" class:capturing={capturingId === entry.id}
        role="group" onkeydown={(e) => onshortcutkeydown(e, entry)}>
        <span class="pref-shortcut-name">{entry.label || entry.id}</span>
        <button type="button" class="pref-key-chip" class:capturing={capturingId === entry.id}
          onclick={() => (capturingId = capturingId === entry.id ? null : entry.id)}>
          {capturingId === entry.id ? "Press keys… · Esc cancels" : keysLabel(entry.keys)}
        </button>
        <button type="button" class="pref-reset-btn" title="Clear shortcut" onclick={() => onresetshortcut(entry)}>Clear</button>
      </div>
    {/each}
  </div>
</div>
