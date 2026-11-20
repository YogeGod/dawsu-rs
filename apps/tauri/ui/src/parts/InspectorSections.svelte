<script>
  import { untrack } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { inspectorHandlers, laneTint } from "../lib/inspector.js";
  import ChevronDown from "../icons/ChevronDown.svg?raw";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));

  // app.js's inspectorModel(); before its first hand-over everything waits.
  const model = $derived(bridge.inspector ?? {
    map: false, busy: false, doc: false, selection: null, scope: "all", hint: "", range: false,
    insertVolume: 100, protect: false, audition: false, hs: false, lane: null,
  });
  const selection = $derived(model.selection);
  const lane = $derived(model.lane);
  const tint = $derived(laneTint(lane));

  // Folding is kept in this window only, as the old <details> were.
  let open = $state({ selection: true, lane: true, difficulty: true });

  // A field being typed in keeps its text until it loses focus (the old setValue rule).
  let startHeld = $state(null);
  let insertHeld = $state(null);
  const startValue = $derived(startHeld ?? selection?.start ?? "");
  const insertValue = $derived(insertHeld ?? model.insertVolume);

  // Change volume: Set all starts at 100, Relative at 0, as before.
  let mode = $state("absolute");
  let amount = $state(100);
  function changeMode(event) {
    mode = event.currentTarget.value;
    amount = mode === "relative" ? 0 : 100;
  }

  // The slider body fields re-seed only when the difficulty or the slider body changes.
  const bodyKey = $derived(selection?.sliderBody?.key ?? null);
  const seed = () => untrack(() => bridge.inspector?.selection?.sliderBody);
  let bodySet = $derived.by(() => (bodyKey, seed()?.set ?? ""));
  let bodyWhistle = $derived.by(() => (bodyKey, !!seed()?.whistle));
  let bodyWhistleSet = $derived.by(() => (bodyKey, seed()?.whistleSet ?? "auto"));
  let bodyVolume = $derived.by(() => (bodyKey, seed()?.volume ?? 100));
</script>

{#snippet head(key, title)}
  <h2 class="title">
    <button type="button" class="head" aria-expanded={open[key]} onclick={() => (open[key] = !open[key])}>
      <span class="chevron" class:folded={!open[key]}>{@html ChevronDown}</span>{title}
    </button>
  </h2>
{/snippet}

<div class="inspector">
  <section class="sec">
    {@render head("selection", "Selection")}
    {#if open.selection}
      <p class="summary">
        {#if selection}{selection.count}<span class="dim">{selection.lanes}</span>{:else}Nothing selected{/if}
      </p>
      {#if selection}
        <div class="rows">
          <form class="row" onsubmit={event => on.moveStart(event, event.currentTarget.elements.start)}>
            <span>Start</span>
            <input name="start" class="field time" type="text" required spellcheck="false"
              autocomplete="off" placeholder="00:00.000" aria-label="Start" title="Start, as 01:45.263 or in milliseconds"
              value={startValue} disabled={model.busy}
              onfocus={() => (startHeld = selection?.start ?? "")}
              onblur={event => { startHeld = null; event.currentTarget.value = selection?.start ?? ""; }}>
            <button type="submit" class="small" disabled={model.busy}>Move</button>
          </form>
          <div class="row"><span>Volume</span><output>{selection.volume}</output></div>
          <div class="row"><span>On object</span><output>{selection.object}</output></div>
        </div>
        <form class="group" aria-label="Change volume" onsubmit={event => on.volume(event, mode, amount)}>
          <p class="caption">Change volume</p>
          <div class="controls">
            <select aria-label="Change" value={mode} onchange={changeMode} disabled={model.busy}>
              <option value="absolute">Set all</option><option value="relative">Relative</option>
            </select>
            <input class="field number" type="number" min={mode === "relative" ? -100 : 0} max="100" required
              aria-label="Volume (%)" bind:value={amount} disabled={model.busy}>
            <button type="submit" disabled={model.busy}>Apply</button>
          </div>
          <div class="controls">
            <button type="button" onclick={() => on.nudge(-5)} disabled={model.busy}>−5</button>
            <button type="button" onclick={() => on.nudge(5)} disabled={model.busy}>+5</button>
            <select aria-label="Applies to" value={model.scope} onchange={event => on.scope(event.currentTarget.value)}
              disabled={model.busy}>
              <option value="all">All lanes</option><option value="selected">Selected lanes</option>
            </select>
          </div>
          <p class="caption hint">{model.hint}</p>
        </form>
        <div class="actions">
          <button type="button" onclick={() => on.command("quantizeDialog")} disabled={model.busy}>Quantize…</button>
          <button type="button" onclick={() => on.command("repeat")} disabled={model.busy}>Repeat…</button>
          <button type="button" onclick={() => on.command("duplicate")} disabled={model.busy}>Duplicate</button>
          <button type="button" onclick={() => on.command("delete")} disabled={model.busy}>Delete</button>
        </div>
      {/if}
      <div class="actions">
        <button type="button" onclick={() => on.command("editRange")} disabled={model.busy || !model.range}>Edit range…</button>
        <button type="button" onclick={() => on.command("scaleByVolume")} disabled={!model.doc}>Scale by volume…</button>
      </div>
      {#if selection?.sliderBody}
        <form class="group" onsubmit={event => on.sliderBody(event,
          { set: bodySet, whistleSet: bodyWhistleSet, whistle: bodyWhistle, volume: bodyVolume })}>
          <p class="caption">Slider body sounds</p>
          <div class="rows">
            <label class="row">Slide and tick sample set
              <select aria-label="Slide and tick sample set" bind:value={bodySet}
                disabled={model.busy || !selection.sliderBody.set}>
                <option value="normal">Normal</option><option value="soft">Soft</option><option value="drum">Drum</option>
                <option value="" disabled>Current sample</option>
              </select>
            </label>
            <label class="row">Whistle on slide
              <input type="checkbox" aria-label="Whistle on slide" bind:checked={bodyWhistle} disabled={model.busy}>
            </label>
            <label class="row">Whistle sample set
              <select aria-label="Whistle sample set" bind:value={bodyWhistleSet} disabled={model.busy}>
                <option value="auto">Auto</option><option value="normal">Normal</option><option value="soft">Soft</option>
                <option value="drum">Drum</option>
              </select>
            </label>
            <label class="row">Body and head volume
              <span class="value"><input class="field number" type="number" min="0" max="100"
                aria-label="Body and head volume" bind:value={bodyVolume} disabled={model.busy}>%</span>
            </label>
          </div>
          <div class="actions"><button type="submit" disabled={model.busy}>Apply</button></div>
        </form>
      {/if}
      <div class="rows">
        <label class="row">Insert volume
          <span class="value"><input class="field number" type="number" min="0" max="100" aria-label="Insert volume"
            value={insertValue} onfocus={() => (insertHeld = model.insertVolume)}
            onblur={event => { insertHeld = null; event.currentTarget.value = String(model.insertVolume); }}
            onchange={on.insert}>%</span>
        </label>
        <label class="row">Protect normals
          <input type="checkbox" aria-label="Protect normals" checked={model.protect} onchange={event => on.toggle("protect", event)}>
        </label>
        <label class="row">Audition
          <input type="checkbox" aria-label="Audition" checked={model.audition} onchange={event => on.toggle("audition", event)}>
        </label>
      </div>
    {/if}
  </section>

  <section class="sec">
    {@render head("lane", "Lane")}
    {#if open.lane}
      {#if lane}
        <p class="lane-name"><strong title={lane.title}>{lane.title}</strong>
          {#if lane.sample}<span class="file" title={lane.sample}>{lane.sample}</span>{/if}</p>
      {/if}
      <div class="rows">
        <div class="row">
          <span>Lane tint</span>
          <button id="lane-colour" type="button" class="tint" aria-haspopup="dialog"
            aria-label={lane ? "Lane tint: " + tint.name : "Lane tint"} title="Lane colour…"
            disabled={!model.doc || !lane?.canColour} onclick={() => on.open("laneColour")}>
            {#if lane}<span class="swatch" style:--swatch={tint.swatch} aria-hidden="true"></span>{tint.name}{/if}
          </button>
        </div>
      </div>
      <div class="actions">
        <button id="replace-sample" type="button" disabled={!model.doc || !lane?.canReplace}
          onclick={() => on.open("replaceSample")}>Replace sample…</button>
      </div>
    {/if}
  </section>

  <section class="sec">
    {@render head("difficulty", "Difficulty")}
    {#if open.difficulty}
      <div class="rows">
        <label class="row">Hitsounding mode
          <input type="checkbox" aria-label="Hitsounding mode" checked={model.hs} onchange={event => on.toggle("hs_mode", event)}>
        </label>
      </div>
      <div class="actions">
        <button id="generate" type="button" disabled={!model.doc} onclick={() => on.open("generate")}>Generate…</button>
        <button id="new" type="button" onclick={() => on.open("new")}>New…</button>
        <button id="copy" type="button" onclick={() => on.open("copy")}>Copy…</button>
        <button id="merge-settings" type="button" aria-haspopup="dialog" disabled={!model.doc}
          onclick={() => on.open("mergeSettings")}>Merge settings…</button>
      </div>
    {/if}
  </section>
</div>

<style>
  /* v3 Inspector: 30 px fold headers, 30 px label/value rows on --editor-chrome. */
  .inspector {
    padding: 0 8px;
    font-size: 12px;
  }
  .sec {
    padding: 0 0 4px;
    border: 0;
  }
  .title {
    margin: 12px 0 0;
    padding: 0;
    border: 0;
    font-size: 12px;
  }
  .sec:first-child .title {
    margin-top: 6px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 30px;
    min-height: 30px;
    padding: 0 8px;
    border: 0;
    border-bottom: 1px solid var(--editor-border);
    border-radius: 0;
    background: transparent;
    color: var(--editor-text);
    font-weight: 500;
    text-align: left;
  }
  .head:hover:not(:disabled) {
    border-color: var(--editor-border);
    background: transparent;
    color: var(--editor-white);
  }
  .chevron {
    display: grid;
    width: 14px;
    height: 14px;
    color: var(--editor-muted);
    transition: transform 0.12s;
  }
  .chevron.folded {
    transform: rotate(-90deg);
  }
  .chevron :global(svg) {
    width: 14px;
    height: 14px;
  }
  .summary,
  .lane-name {
    margin: 0;
    padding: 8px 12px 6px;
    color: var(--editor-secondary);
    overflow-wrap: anywhere;
  }
  .dim {
    margin: 0;
    color: var(--editor-muted);
  }
  .lane-name {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .lane-name strong {
    overflow: hidden;
    color: var(--editor-text);
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file {
    overflow: hidden;
    color: var(--editor-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 4px 0;
  }
  /* A long value (On object) wraps under its label, right-aligned, as before. */
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 2px 6px;
    min-height: 30px;
    margin: 0;
    padding: 3px 6px 3px 12px;
    border-radius: 4px;
    background: var(--editor-chrome);
    color: var(--editor-secondary);
  }
  .row > span:first-child {
    flex: 1 0 auto;
    margin: 0;
    white-space: nowrap;
  }
  output,
  .value {
    margin-left: auto;
    color: var(--editor-readout);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .row output {
    padding-right: 6px;
  }
  .value {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: 11px;
  }
  .row select {
    max-width: 92px;
    min-height: 24px;
    padding: 0 4px;
  }
  .field {
    min-height: 24px;
    padding: 0 4px;
    border: 0;
    border-radius: 4px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-well);
    color: var(--editor-readout);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .time {
    width: 72px;
  }
  .number {
    width: 48px;
  }
  .small {
    min-height: 24px;
    padding: 0 6px;
  }
  .group {
    margin: 10px 0 6px;
  }
  .caption {
    margin: 0 0 4px;
    color: var(--editor-muted);
    font-size: 11px;
  }
  .hint {
    margin-top: 4px;
  }
  .controls {
    display: flex;
    gap: 4px;
    margin: 4px 0;
  }
  .controls > * {
    min-height: 26px;
  }
  .controls select {
    flex: 1;
    min-width: 0;
  }
  .controls .number {
    width: 44px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 6px 0;
  }
  .actions button {
    min-height: 26px;
    padding: 2px 8px;
    font-size: 12px;
  }
  .tint {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    min-height: 26px;
    margin-left: auto;
    padding: 0 6px;
    border: 0;
    background: transparent;
    color: var(--editor-readout);
  }
  .swatch {
    flex: none;
    width: 18px;
    height: 18px;
    border-radius: 8px;
    background: var(--swatch);
    box-shadow: 0 0 0 2px var(--editor-text);
  }
  input[type="checkbox"] {
    margin-left: auto;
  }
</style>
