<script>
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import { forwardSpace } from "../lib/space.js";
  import {
    applyRequest, autoStart, blockDrops, canCheck, checkLine, forget, initialState, isChecked, isFresh, landsText,
    laneLabel, listenFrom, nextPoll, progressText, reviewRequest, settle, startRequest, stripMarks, take, ticked, timeText,
  } from "../lib/drum.js";

  // The editor's answer to drum_context and the window's own state beside it (drum.js `take`).
  // A test may pass an answer in.
  let { initialView = null } = $props();
  let state = $state.raw(initialState(initialView));
  // Option changes sent but not yet held by the editor, so quick changes add up.
  let pending = {};
  let error = $state(null);
  let busy = $state(false);
  // Bumped when the editor refuses an option, so the controls show its options again.
  let resetKey = $state(0);
  // Each request is numbered; an answer older than the last one shown is dropped.
  let sent = 0;
  let shown = 0;
  let pollTimer;
  let closed = false;
  let canvas = $state(null);

  const view = $derived(state.view);
  const rows = $derived(state.rows);
  const overrides = $derived(state.overrides);
  const picked = $derived(state.picked);
  const sourceKind = $derived(state.source);
  const stemPath = $derived(state.stemPath);
  const duration = $derived(state.view?.duration);
  const ready = $derived(view?.state === "ready" && rows !== null);
  const options = $derived(view?.options ?? null);
  const checkedCount = $derived(ready ? ticked(rows, overrides).length : 0);
  const stemName = $derived(view?.source?.kind === "stem" ? view.source.name
    : stemPath ? String(stemPath).split(/[\/]/).pop() : null);
  const gridValue = $derived(options ? String(options.grid ?? view.editorGrid) : "4");
  const loose = $derived(ready ? rows.some(row => row.lands === "grid" && isChecked(row, overrides)) : false);
  const canDetect = $derived(!!view?.map && view.state === "idle" && !ready
    && (sourceKind === "song" ? view.hasSong : !!stemPath));

  const update = change => (state = { ...state, ...change });

  // Runs one drum command (`call` invokes it) and shows its answer, unless a newer one was shown.
  async function send(call) {
    const sequence = ++sent;
    busy = true;
    try {
      const next = await call();
      if (isFresh(sequence, shown)) {
        shown = sequence;
        state = take(state, next);
        pending = settle(pending, next);
      }
      error = null;
      return true;
    } catch (failure) {
      error = String(failure);
      return false;
    } finally {
      if (sequence === sent) busy = false;
      startIfDue();
    }
  }

  function schedulePoll() {
    clearTimeout(pollTimer);
    const delay = nextPoll(closed, view);
    if (delay !== null) pollTimer = setTimeout(poll, delay);
  }

  async function poll() {
    if (closed) return;
    await send(() => invoke("drum_context", { known: rows ? view?.review ?? null : null }));
    schedulePoll();
  }

  function startIfDue() {
    const key = autoStart(state, busy);
    if (key) start(key);
  }

  async function start(key) {
    update({ started: [...state.started, key], rows: null });
    // A refused start (another job, an open edit) is forgotten, so Detect can try again.
    if (!await send(() => invoke("drum_detect_start", startRequest(state)))) state = forget(state, key);
  }

  function detect() {
    const key = sourceKind === "song" ? "song" : stemPath;
    state = forget(state, key);
    start(key);
  }

  function chooseSource(kind) {
    if (kind === sourceKind) return;
    update({ source: kind });
    startIfDue();
  }

  async function pickStem() {
    try {
      const path = await invoke("drum_pick_stem");
      if (!path) return;
      update({ source: "stem", stemPath: path });
      state = forget(state, path);
      startIfDue();
    } catch (failure) {
      error = String(failure);
    }
  }

  async function review(change) {
    const asked = reviewRequest(view, pending, change);
    if (asked.error) {
      error = asked.error;
      resetKey++;
      return;
    }
    pending = asked.pending;
    if (!await send(() => invoke("drum_review", asked.request))) {
      pending = {};
      resetKey++;
    }
  }

  const lane = value => (value === "" ? null : value);
  const number = (event, fallback) => {
    const value = event.currentTarget.valueAsNumber;
    return Number.isFinite(value) ? value : fallback;
  };

  function toggle(row) {
    if (!canCheck(row)) return;
    update({ overrides: { ...overrides, [row.id]: !isChecked(row, overrides) } });
  }

  function pick(row) {
    update({ picked: row.id });
  }

  function rowKey(event, row) {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (event.key === " ") toggle(row);
    pick(row);
  }

  function listen() {
    if (view.listening) send(() => invoke("drum_listen_stop"));
    else send(() => invoke("drum_listen", { map: view.map, from: listenFrom(rows, picked), ticked: ticked(rows, overrides) }));
  }

  async function apply() {
    const request = applyRequest(state);
    if (request && await send(() => invoke("drum_apply", request))) close();
  }

  async function close() {
    closed = true;
    clearTimeout(pollTimer);
    try {
      await invoke("close_drum_window");
    } catch (failure) {
      closed = false;
      error = String(failure);
      schedulePoll();
    }
  }

  // The strip: diamonds for each hit across the song, snares above kicks, unchecked ones faint.
  // It reads only the rows, ticks, pick and duration, so a poll with the same review redraws nothing.
  $effect(() => {
    if (!canvas) return;
    const marks = stripMarks(rows, overrides, duration);
    const pickedRow = (rows ?? []).find(row => row.id === picked);
    const scale = window.devicePixelRatio || 1;
    const { width, height } = canvas.getBoundingClientRect();
    canvas.width = Math.round(width * scale);
    canvas.height = Math.round(height * scale);
    const draw = canvas.getContext("2d");
    draw.setTransform(scale, 0, 0, scale, 0, 0);
    draw.clearRect(0, 0, width, height);
    const look = getComputedStyle(canvas);
    const colour = { kick: look.getPropertyValue("--editor-kick"), snare: look.getPropertyValue("--editor-clap") };
    for (const mark of marks) {
      const x = 56 + mark.x * (width - 68);
      const y = mark.drum === "snare" ? height * 0.32 : height * 0.68;
      draw.globalAlpha = mark.on ? 1 : 0.3;
      draw.fillStyle = colour[mark.drum];
      draw.beginPath();
      draw.moveTo(x, y - 6);
      draw.lineTo(x + 6, y);
      draw.lineTo(x, y + 6);
      draw.lineTo(x - 6, y);
      draw.fill();
    }
    if (pickedRow && duration > 0) {
      draw.globalAlpha = 1;
      draw.fillStyle = look.getPropertyValue("--editor-accent");
      draw.fillRect(56 + (pickedRow.time / duration) * (width - 68), 0, 1, height);
    }
  });

  let release;
  onMount(() => {
    release = blockDrops(window);
    poll();
  });
  onDestroy(() => {
    closed = true;
    clearTimeout(pollTimer);
    release?.();
  });
</script>

<svelte:window onkeydown={forwardSpace} />

<div class="drum">
  <aside class="side">
    <h2>Source</h2>
    <div class="segments" role="group" aria-label="Source">
      <button type="button" aria-pressed={sourceKind === "song"} disabled={view?.hasSong === false}
        onclick={() => chooseSource("song")}>Song</button>
      <button type="button" aria-pressed={sourceKind === "stem"} onclick={() => chooseSource("stem")}>Drum stem</button>
    </div>
    {#if sourceKind === "stem"}
      <div class="file">
        <span class="name">{stemName ?? "No file picked"}</span>
        <button type="button" class="link" onclick={pickStem}>{stemName ? "Change" : "Pick a file…"}</button>
      </div>
      {#if options}
        <label class="field-row">
          Stem starts at
          <span class="field">
            <input type="number" step="1" value={options.stemOffset}
              onchange={event => review({ stemOffset: number(event, options.stemOffset) })}>
            ms
          </span>
        </label>
      {/if}
    {/if}

    {#key resetKey}
    <h2>Lanes</h2>
    <div class="lanes">
      {#each [["kick", "Kicks"], ["snare", "Snares"]] as [drum, label] (drum)}
        <label for={"lane-" + drum}>{label}</label>
        <select id={"lane-" + drum} class={drum} value={options?.[drum] ?? ""} disabled={!options}
          onchange={event => review({ [drum]: lane(event.currentTarget.value) })}>
          <option value="">None</option>
          {#each view?.lanes ?? [] as choice (choice.key)}
            <option value={choice.key}>{laneLabel(choice)}</option>
          {/each}
        </select>
      {/each}
    </div>

    <h2>Detection</h2>
    {#if options}
      <div class="detection">
        <label class="slider">
          <span class="line"><span>Sensitivity</span><span class="value">{options.sensitivity}</span></span>
          <input type="range" min="0" max="100" step="1" value={options.sensitivity}
            onchange={event => review({ sensitivity: number(event, options.sensitivity) })}>
        </label>
        <label class="field-row">
          Snap to objects within
          <span class="field">
            <input type="number" min="0" max={view.maxObjectMs} step="1" value={options.objectMs}
              onchange={event => review({ objectMs: number(event, options.objectMs) })}>
            ms
          </span>
        </label>
        <label class="field-row">
          Beat grid
          <select class="grid" value={gridValue}
            onchange={event => review({ grid: Number(event.currentTarget.value) })}>
            {#each view.grids as grid (grid)}<option value={String(grid)}>1/{grid}</option>{/each}
          </select>
        </label>
      </div>
    {/if}
    {/key}

    <div class="notes">
      {#each view?.state === "ready" ? view.unhides ?? [] : [] as name (name)}
        <p>Applying shows the hidden lane {name}.</p>
      {/each}
      {#if loose}<p>Hits on grid lines have no object. Save writes them only in Hitsounding mode.</p>{/if}
      {#if options && !options.kick && !options.snare}<p>Pick a lane for kicks or snares to add hits.</p>{/if}
      <p>Runs on your PC with signal processing. No audio is uploaded. Works best on a drum stem.</p>
    </div>
  </aside>

  <section class="review">
    <div class="strip">
      <span class="label snare">Snare</span>
      <span class="label kick">Kick</span>
      <canvas bind:this={canvas} aria-hidden="true"></canvas>
    </div>
    <div class="head" aria-hidden="true">
      <span class="tick"></span><span class="time">Time</span><span class="sound">Sound</span>
      <span class="strength" title="Relative to the strongest hit">Confidence</span><span class="lands">Lands on</span>
    </div>
    <div class="rows">
      {#if !view}
        <p class="empty">Loading…</p>
      {:else if !view.map}
        <p class="empty">Open a difficulty in the editor first.</p>
      {:else if view.state === "running"}
        <p class="empty">{progressText(view)}</p>
      {:else if ready}
        {#each rows as row (row.id)}
          {@const on = isChecked(row, overrides)}
          <div class="row {row.lands}" class:off={!on} class:picked={row.id === picked} tabindex="0" role="row"
            aria-selected={row.id === picked} onclick={() => pick(row)} onkeydown={event => rowKey(event, row)}>
            <input type="checkbox" class="tick" checked={on} disabled={!canCheck(row)}
              aria-label={"Add the " + row.drum + " at " + timeText(row.time)}
              onclick={event => event.stopPropagation()} onchange={() => toggle(row)}>
            <span class="time">{timeText(row.time)}</span>
            <span class="sound {row.drum}">{row.drum === "kick" ? "Kick" : "Snare"}</span>
            <span class="strength">
              <span class="bar"><span style:--strength={row.strength}></span></span>
              <span class="percent">{Math.round(row.strength * 100)}%</span>
            </span>
            <span class="lands">{landsText(row.lands)}</span>
          </div>
        {:else}
          <p class="empty">No drum hits found. Try a higher sensitivity.</p>
        {/each}
      {:else if view.error}
        <p class="empty error">{view.error}</p>
      {:else if sourceKind === "stem" && !stemPath}
        <p class="empty">Pick a drum stem file.</p>
      {/if}
      {#if canDetect}
        <button type="button" class="detect" disabled={busy} onclick={detect}>Detect</button>
      {/if}
    </div>
    <footer class="actions">
      <button type="button" class="listen" disabled={!ready || busy} onclick={listen}>
        {view?.listening ? "Stop" : "Listen"}
      </button>
      <span class="hint">Song on the left, cues on the right</span>
      {#if ready}<span class="count">{checkLine(rows, overrides)}</span>{/if}
      <span class="status" role="status">
        {#if error}<span class="error" role="alert">{error}</span>{/if}
      </span>
      <button type="button" class="secondary" onclick={close}>Cancel</button>
      <button type="button" class="primary" disabled={!ready || !checkedCount || busy} onclick={apply}>Apply</button>
    </footer>
  </section>
</div>

<style>
  /* drum.html has no style.css: the page's own frame lives here, scoped to that body. */
  :global(body.drum-page) {
    margin: 0;
    height: 100vh;
    overflow: hidden;
    background: var(--editor-canvas);
    color: var(--editor-text);
    font: 12px/1.4 var(--font-ui);
    color-scheme: dark;
  }
  :global(body.drum-page *) { box-sizing: border-box; }

  .drum { display: flex; height: 100vh; }
  .side {
    width: 300px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 10px;
    background: var(--editor-panel);
    border-right: 1px solid var(--editor-border);
    overflow: auto;
  }
  h2 {
    margin: 0;
    height: 33px;
    display: flex;
    align-items: center;
    padding: 0 8px;
    border-bottom: 1px solid var(--editor-border);
    font-size: 12px;
    font-weight: 500;
  }
  .segments {
    display: flex;
    gap: 2px;
    height: 28px;
    margin: 10px 6px 4px;
    padding: 2px;
    border-radius: 6px;
    background: var(--editor-control);
  }
  .segments button {
    flex: 1;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--editor-secondary);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
  }
  .segments button[aria-pressed="true"] { background: var(--editor-selected); color: var(--editor-accent); }
  .segments button:disabled { color: var(--editor-disabled); cursor: default; }
  .file {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    margin: 4px 6px;
    padding: 0 10px;
    border-radius: 4px;
    background: var(--editor-control);
  }
  .file .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lanes {
    display: grid;
    grid-template-columns: 70px 1fr;
    gap: 8px 10px;
    align-items: center;
    padding: 10px 6px;
    color: var(--editor-secondary);
  }
  select {
    height: 30px;
    min-width: 0;
    padding: 0 8px;
    border: 0;
    border-radius: 6px;
    background: var(--editor-control);
    color: var(--editor-text);
    font: inherit;
  }
  select.grid { width: 84px; background: var(--editor-selected); color: var(--editor-accent); font-weight: 500; }
  .detection { display: flex; flex-direction: column; gap: 14px; padding: 12px 6px; color: var(--editor-secondary); }
  .slider { display: flex; flex-direction: column; gap: 8px; }
  .slider .line { display: flex; justify-content: space-between; }
  .slider .value { color: var(--editor-readout); font-variant-numeric: tabular-nums; }
  .slider input { width: 100%; accent-color: var(--editor-accent); }
  .field-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin: 0 6px; color: var(--editor-secondary); }
  .detection .field-row { margin: 0; }
  .field {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 8px;
    border-radius: 4px;
    background: var(--editor-control);
    color: var(--editor-muted);
  }
  .field input {
    width: 40px;
    border: 0;
    background: transparent;
    color: var(--editor-readout);
    font: inherit;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .field input:focus { outline: none; }
  .field:focus-within { box-shadow: inset 0 0 0 1px var(--editor-focus); }
  .notes { margin-top: auto; padding: 0 6px 8px; color: var(--editor-muted); line-height: 1.5; }
  .notes p { margin: 8px 0 0; }

  .review { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .strip {
    position: relative;
    height: 110px;
    flex: none;
    background: var(--editor-tealtint);
    border-bottom: 1px solid var(--editor-border);
  }
  .strip canvas { position: absolute; inset: 0; width: 100%; height: 100%; }
  .label { position: absolute; left: 10px; font-weight: 500; transform: translateY(-50%); }
  .label.snare { top: 32%; color: var(--editor-clap); }
  .label.kick { top: 68%; color: var(--editor-kick); }
  .head, .row { display: flex; align-items: center; gap: 12px; }
  .head {
    height: 36px;
    flex: none;
    padding: 0 16px;
    border-bottom: 1px solid var(--editor-border);
    color: var(--editor-muted);
  }
  .tick { width: 15px; flex: none; margin: 0; accent-color: var(--editor-accent); }
  .time { width: 90px; flex: none; font-variant-numeric: tabular-nums; }
  .sound { width: 80px; flex: none; display: flex; align-items: center; gap: 8px; }
  .strength { width: 200px; flex: none; display: flex; align-items: center; gap: 10px; }
  .lands { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .rows { flex: 1; min-height: 0; overflow: auto; padding: 4px 6px; }
  .row { height: 32px; padding: 0 10px; border-radius: 4px; cursor: default; }
  .row .time { font-weight: 600; }
  .row.off .time, .row.off .sound { color: var(--editor-muted); font-weight: 400; }
  .row.picked { background: var(--editor-control); }
  .row.flagged .lands { color: var(--editor-warning); }
  .row.duplicate .lands, .row.noLane .lands { color: var(--editor-disabled); }
  .row .sound::before {
    content: "";
    width: 8px;
    height: 8px;
    transform: rotate(45deg);
    background: var(--editor-kick);
  }
  .row .sound.snare::before { background: var(--editor-clap); }
  .row.off .sound::before { opacity: .5; }
  .bar { flex: 1; height: 4px; border-radius: 2px; background: var(--editor-border); }
  .bar span {
    display: block;
    height: 4px;
    width: calc(var(--strength) * 100%);
    border-radius: 2px;
    background: var(--editor-accent);
  }
  .percent { width: 32px; color: var(--editor-secondary); font-variant-numeric: tabular-nums; }
  .empty { margin: 16px 10px; color: var(--editor-muted); }
  .detect {
    height: 30px;
    margin: 0 10px;
    padding: 0 14px;
    border: 0;
    border-radius: 6px;
    background: var(--editor-control);
    color: var(--editor-text);
    font-weight: 500;
    cursor: pointer;
  }
  .row:focus-visible { outline: 2px solid var(--editor-focus); outline-offset: -2px; }
  .error { color: var(--editor-error); }

  .actions {
    height: 62px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 20px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-panel);
  }
  .hint { color: var(--editor-muted); white-space: nowrap; }
  .count { color: var(--editor-secondary); white-space: nowrap; }
  .status { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  button { font: inherit; }
  .link { padding: 0; border: 0; background: none; color: var(--editor-accent); cursor: pointer; white-space: nowrap; }
  .listen, .secondary, .primary {
    height: 30px;
    padding: 0 14px;
    border: 0;
    border-radius: 6px;
    font-weight: 500;
    cursor: pointer;
  }
  .listen { background: var(--editor-play); color: var(--editor-ink); }
  .secondary, .primary { height: 36px; min-width: 88px; font-weight: 600; box-shadow: var(--shadow-raised); }
  .secondary { background: var(--editor-control); color: var(--editor-text); }
  .primary { background: var(--editor-accent); color: var(--editor-accentink); }
  button:disabled { background: var(--editor-selected); color: var(--editor-disabled); cursor: default; }
  .segments button:disabled { background: none; }
  button:focus-visible, select:focus-visible, input:focus-visible { outline: 2px solid var(--editor-focus); outline-offset: 2px; }
</style>
