<script>
  import { tick } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { folderShades, laneShades } from "../lib/colour.js";
  import { laneHandlers } from "../lib/laneHandlers.js";
  import { folderChip, laneIcons } from "../lib/lanes.js";
  import Folder from "../icons/Folder.svg?raw";
  import FolderOpen from "../icons/FolderOpen.svg?raw";

  // app.js keeps the lane state and builds these rows in renderLanes(); this part only draws them
  // and hands each click back through laneHeaderAction().
  const headers = $derived(bridge.lanes);
  const rows = $derived(headers?.rows ?? []);
  const names = $derived(headers?.names !== false);
  const folder = folderShades();
  const on = laneHandlers((...args) => globalThis.laneHeaderAction?.(...args), tick);
</script>

{#each rows as row (row.key)}
  {#if row.type === "folder"}
    {@const chip = folderChip(row)}
    <div class="lane sample-folder" class:compact={!names} data-sample-set={row.set} draggable="true"
      role="group" aria-label={row.name}
      style:--lh-strip={folder.strip} style:--lh-header={folder.header}
      ondragstart={event => on.dragStart(event, row.set)} ondragover={on.dragOver} ondrop={event => on.drop(event, row.set)}>
      <button type="button" class="fold" aria-label={row.name} aria-expanded={!row.collapsed}
        title="Fold or unfold. Alt+Up or Alt+Down moves the folder"
        onclick={() => on.fold(row.set)} onkeydown={event => on.folderKey(event, row.set)}
      >{@html row.collapsed ? Folder : FolderOpen}</button>
      <button type="button" class="lane-mute ms" aria-label={"Mute " + row.name} aria-pressed={row.muted}
        onclick={() => on.folderMix(row.set, "mute")}>M</button>
      <button type="button" class="lane-solo ms" aria-label={"Solo " + row.name} aria-pressed={row.solo}
        onclick={() => on.folderMix(row.set, "solo")}>S</button>
      {#if names}
        <!-- The strip is the keyboard path; a click on the name folds as before. -->
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span class="name" onclick={() => on.foldName(row.set)}>{row.name}</span>
        <span class="chip" title={chip.title}>{chip.text}</span>
      {/if}
    </div>
  {:else if row.type === "drum"}
    <div class="lane" class:compact={!names}>
      <span class="name unassigned-drum" title={row.title}>{names ? row.name : "–"}</span>
    </div>
  {:else}
    {@const shade = laneShades(row.colour)}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="lane" class:active={row.active} class:lane-selected={row.selected} class:quiet={row.quiet}
      class:lane-layer={row.layer} class:family-head={!!row.family} class:compact={!names} data-lane-row={row.row}
      style:--lh-strip={shade.strip} style:--lh-tile={shade.tile} style:--lh-header={shade.header}
      style:--lh-hover={shade.hover} style:--lh-selected={shade.selected}
      style:--lh-spine={row.folder ? folder.strip : undefined}
      onclick={event => on.rowClick(event, row.row)}>
      <span class="spine"></span>
      <span class="num">{row.number}</span>
      <!-- A numbered layer hangs under its main lane, which folds it like a folder (Yoge, 2026-11-21). -->
      {#if row.layer}<span class="layer-guide" aria-hidden="true"></span>{/if}
      {#if names}<span class="tile" aria-hidden="true"><svg viewBox="0 0 16 16">{@html laneIcons[row.icon] ?? laneIcons.file}</svg></span>{/if}
      <button type="button" class="lane-mute ms" aria-label={"Mute " + row.display} aria-pressed={row.muted}
        onclick={() => on.mix(row.row, "mute")}>M</button>
      <button type="button" class="lane-solo ms" aria-label={"Solo " + row.display} aria-pressed={row.solo}
        onclick={() => on.mix(row.row, "solo")}>S</button>
      {#if row.family}
        <button type="button" class="lane-disclosure" aria-expanded={!row.family.collapsed} aria-label={row.family.label}
          onclick={() => on.family(row.row)}>{row.family.collapsed ? "▸" : "▾"}</button>
      {/if}
      <button type="button" class="name" data-lane-row={row.row} aria-haspopup="menu" aria-expanded="false"
        aria-label={"Select and audition " + row.label} aria-pressed={row.selected} title={row.title}
        onclick={event => on.select(event, row.row)}>{names ? row.name : "▶"}</button>
      {#if names && row.tag}<span class="tag-slot"><span class="tag">{row.tag}</span></span>{/if}
      {#if row.addLayer}
        <button type="button" class="lane-add-layer" aria-label={row.addLayer} title={row.addLayer}
          disabled={headers.busy} onclick={() => on.addLayer(row.row)}>+</button>
      {/if}
      {#if row.remove}
        <button type="button" class="lane-remove" class:alone={!row.addLayer} aria-label={row.remove.label}
          title={row.remove.label} disabled={headers.busy || row.remove.disabled}
          onclick={() => on.remove(row.row)}>−</button>
      {/if}
    </div>
  {/if}
{/each}

<style>
  /* Every row is one canvas row tall; app.js sets --lane-row-height on #lanes. */
  .lane {
    display: flex;
    align-items: center;
    gap: 4px;
    height: var(--lane-row-height, 34px);
    padding: 0 8px 0 0;
    box-sizing: border-box;
    overflow: hidden;
    border-bottom: 1px solid var(--editor-canvas);
    background: var(--lh-header, var(--editor-panel));
    font-family: var(--font-ui);
    font-size: 12px;
    color: var(--editor-text);
    cursor: default;
  }
  .lane:not(.sample-folder):hover {
    background: var(--lh-hover, var(--editor-panel));
  }
  .lane.active,
  .lane.active:hover {
    background: var(--lh-selected);
  }
  .lane.lane-selected {
    outline: 1px solid var(--editor-accent);
    outline-offset: -1px;
  }
  .spine {
    align-self: stretch;
    width: 6px;
    flex: none;
    background: var(--lh-spine, transparent);
  }
  .num {
    display: grid;
    place-items: center;
    align-self: stretch;
    width: 24px;
    flex: none;
    margin-left: -4px;
    background: var(--lh-strip);
    color: var(--editor-ink);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .tile {
    display: grid;
    place-items: center;
    width: min(26px, calc(var(--lane-row-height, 34px) - 6px));
    height: min(26px, calc(var(--lane-row-height, 34px) - 6px));
    flex: none;
    border-radius: 6px;
    background: var(--lh-tile);
    color: var(--lh-strip);
  }
  .tile svg {
    width: 14px;
    height: 14px;
    max-width: 100%;
    max-height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .ms {
    width: 24px;
    height: min(25px, calc(var(--lane-row-height, 34px) - 6px));
    min-height: 0;
    flex: none;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: var(--ms-idle);
    box-shadow: inset 0 0 0 1px var(--ms-edge), var(--shadow-raised);
    color: var(--editor-secondary);
    font: 600 12px var(--font-ui);
    cursor: pointer;
  }
  .ms:hover {
    color: var(--editor-text);
  }
  .ms.lane-mute[aria-pressed="true"] {
    border: 0;
    background: var(--mute-on);
    color: var(--editor-ink);
  }
  .ms.lane-solo[aria-pressed="true"] {
    border: 0;
    background: var(--solo-on);
    color: var(--editor-ink);
  }
  .lane-solo {
    margin-right: 2px;
  }
  .name {
    min-width: 0;
    flex: 0 1 auto;
    padding: 0 2px;
    overflow: hidden;
    border: 0;
    background: transparent;
    color: var(--editor-text);
    font: 600 13px var(--font-ui);
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  button.name {
    min-height: 0;
    cursor: pointer;
  }
  button.name:hover {
    background: transparent;
  }
  /* A layer's guide: a quiet line under its main lane's chevron, so a family reads as one block. */
  .layer-guide {
    align-self: stretch;
    width: 8px;
    flex: none;
    margin-left: 8px;
    border-left: 1px solid var(--editor-disabled);
  }
  .family-head .lane-disclosure {
    color: var(--editor-secondary);
  }
  .unassigned-drum {
    padding-left: 8px;
    color: var(--editor-muted);
    font-weight: 400;
  }
  /* The tag gives way before the name: its slot shrinks first and clips it. */
  .tag-slot {
    display: flex;
    min-width: 0;
    flex: 0 100 auto;
    overflow: hidden;
  }
  .tag {
    display: none;
    flex: none;
    padding: 1px 6px;
    border: 1px solid var(--editor-border);
    border-radius: 4px;
    background: var(--lh-tile);
    color: var(--lh-strip);
    font-size: 11px;
    line-height: 14px;
    white-space: nowrap;
  }
  .lane:is(:hover, :focus-within) .tag {
    display: inline-block;
  }
  .lane-disclosure,
  .lane-add-layer,
  .lane-remove {
    flex: none;
    width: 14px;
    min-height: 18px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    cursor: pointer;
  }
  .lane-add-layer {
    margin-left: auto;
    font-weight: 600;
  }
  .lane-remove {
    font-weight: 600;
  }
  .lane-remove.alone {
    margin-left: auto;
  }
  .lane-add-layer:disabled,
  .lane-remove:disabled {
    opacity: 0.45;
    cursor: default;
  }
  /* v3: muted lanes, and the others while any lane is soloed, go grey. */
  .quiet .num {
    background: var(--editor-disabled);
  }
  .quiet .tile {
    color: var(--editor-disabled);
  }
  .quiet .name {
    color: var(--editor-disabled);
  }
  .sample-folder {
    background: var(--lh-header);
  }
  .fold {
    display: grid;
    place-items: center;
    align-self: stretch;
    width: 30px;
    min-height: 0;
    flex: none;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: var(--lh-strip);
    color: var(--editor-ink);
    cursor: pointer;
  }
  .fold:hover {
    background: var(--editor-secondary);
  }
  .fold :global(svg) {
    width: 14px;
    height: 14px;
  }
  .sample-folder .name {
    flex: 1 1 auto;
    cursor: pointer;
  }
  .chip {
    display: inline-grid;
    place-items: center;
    height: 18px;
    min-width: 18px;
    margin-left: auto;
    padding: 0 6px;
    box-sizing: border-box;
    border-radius: 9px;
    background: rgba(0, 0, 0, 0.3);
    color: var(--editor-secondary);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .fold:focus-visible,
  .ms:focus-visible,
  .name:focus-visible,
  .lane-disclosure:focus-visible,
  .lane-add-layer:focus-visible,
  .lane-remove:focus-visible {
    outline: 2px solid var(--editor-focus);
    outline-offset: -2px;
  }
  /* Lane names hidden: a 99 px column keeps number, M, S, the layer chevron, the select button, + and −. */
  .compact {
    gap: 1px;
    padding-right: 1px;
  }
  .compact .spine {
    display: none;
  }
  .compact .num {
    width: 20px;
    margin: 0 1px 0 0;
  }
  .compact .ms {
    width: 16px;
  }
  .compact .lane-solo {
    margin-right: 1px;
  }
  .compact .fold {
    width: 22px;
  }
  /* Main lane, + and − together still fit the 99 px column. */
  .compact .lane-disclosure {
    width: 8px;
  }
  .compact .lane-add-layer,
  .compact .lane-remove {
    width: 10px;
  }
  .compact .layer-guide {
    display: none;
  }
  .compact button.name {
    flex: 1 1 auto;
    min-width: 8px;
    padding: 0;
    font-size: 11px;
    text-align: center;
  }
  @media (forced-colors: active) {
    .ms[aria-pressed="true"],
    .lane.active {
      outline: 2px solid Highlight;
      outline-offset: -2px;
    }
  }
</style>
