<script>
  import { filterDiffGroups } from "../../lib/export.js";

  let { diff, open = false, onopenchange = null, id = null } = $props();
  let activeSection = $state("all");

  $effect(() => {
    const sections = diff?.sections ?? [];
    if (activeSection !== "all" && !sections.some(sec => sec.section === activeSection)) {
      activeSection = "all";
    }
  });

  const filtered = $derived(filterDiffGroups(diff, activeSection));
</script>

{#if diff}
  <details class="osu-diff" {id} {open} ontoggle={e => onopenchange?.(e.currentTarget.open)}>
    <summary class="osu-diff-toggle">
      Compare <code>.osu</code>
      (<span class="osu-diff-add">+{diff.added}</span> / <span class="osu-diff-rem">-{diff.removed}</span> lines)
    </summary>
    {#if (diff.sections?.length ?? 0) > 1}
      <div class="osu-diff-filters" role="group" aria-label="Filter diff by section">
        <button
          type="button"
          class="diff-chip"
          aria-pressed={activeSection === "all"}
          onclick={() => (activeSection = "all")}
        >All</button>
        {#each diff.sections as sec (sec.section)}
          <button
            type="button"
            class="diff-chip"
            aria-pressed={activeSection === sec.section}
            onclick={() => (activeSection = sec.section)}
          >
            {sec.section}
            <span class="osu-diff-add">+{sec.added}</span>/<span class="osu-diff-rem">-{sec.removed}</span>
          </button>
        {/each}
      </div>
    {/if}
    <div class="osu-diff-body">
      {#each filtered.groups as group, gi (gi)}
        {#if group.section}<div class="osu-diff-section">{group.section}</div>{/if}
        {#each group.entries ?? [] as entry, ei (ei)}
          <div class="osu-diff-line {entry.kind}">
            <span class="osu-diff-ln">{entry.line}</span>
            <span class="osu-diff-sign">{entry.kind === "added" ? "+" : "-"}</span>
            <span class="osu-diff-text">{entry.text}</span>
          </div>
        {/each}
      {/each}
      {#if filtered.truncated > 0}
        <div class="osu-diff-more">… {filtered.truncated} more changed {filtered.truncated === 1 ? "line" : "lines"}</div>
      {/if}
    </div>
  </details>
{/if}

<style>
  .osu-diff {
    margin-top: 10px;
    border-top: 1px solid var(--editor-border);
    padding-top: 8px;
  }
  .osu-diff-toggle {
    cursor: pointer;
    color: var(--editor-secondary);
    font-size: 12px;
    user-select: none;
    list-style-position: outside;
  }
  .osu-diff-toggle code {
    font-family: var(--font-mono);
    font-size: 11px;
  }
  .osu-diff-add { color: var(--editor-play); font-variant-numeric: tabular-nums; }
  .osu-diff-rem { color: var(--editor-error); font-variant-numeric: tabular-nums; }
  .osu-diff-filters {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
  }
  .diff-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--editor-border);
    border-radius: 999px;
    background: var(--editor-control);
    color: var(--editor-secondary);
    font: inherit;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .diff-chip:hover {
    border-color: var(--editor-accent);
    color: var(--editor-text);
  }
  .diff-chip[aria-pressed="true"] {
    border-color: var(--editor-accent);
    background: var(--editor-selected);
    color: var(--editor-text);
    font-weight: 600;
  }
  .osu-diff-body {
    margin-top: 8px;
    max-height: 240px;
    overflow: auto;
    border-radius: 6px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-well);
    color: var(--ink-on-dark);
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.45;
  }
  .osu-diff-section {
    padding: 4px 10px 2px;
    color: var(--editor-accent);
    font-weight: 600;
    background: rgba(255, 255, 255, .03);
    border-top: 1px solid var(--editor-border);
  }
  .osu-diff-section:first-child { border-top: 0; }
  .osu-diff-line {
    display: grid;
    grid-template-columns: 44px 16px 1fr;
    padding: 1px 10px 1px 6px;
    white-space: pre;
    user-select: text;
  }
  .osu-diff-line.added {
    background: rgba(52, 199, 89, .10);
    color: var(--ink-on-dark);
  }
  .osu-diff-line.removed {
    background: rgba(255, 69, 58, .10);
    color: var(--ink-on-dark);
  }
  .osu-diff-ln {
    text-align: right;
    padding-right: 8px;
    color: var(--editor-disabled);
    font-variant-numeric: tabular-nums;
    user-select: none;
  }
  .osu-diff-sign {
    text-align: center;
    font-weight: 600;
    user-select: none;
  }
  .osu-diff-line.added .osu-diff-sign { color: var(--editor-play); }
  .osu-diff-line.removed .osu-diff-sign { color: var(--editor-error); }
  .osu-diff-text { min-width: 0; }
  .osu-diff-more {
    padding: 5px 10px;
    color: #98989f;
    border-top: 1px solid var(--editor-border);
    font-style: italic;
  }
</style>
