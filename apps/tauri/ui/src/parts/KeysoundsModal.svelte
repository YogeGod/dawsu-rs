<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    NO_PLUGINS, PX_PER_MS, QUANTIZE_MS, ROW_PX, findNoteIndex, generateToast, instrumentChoices, isBlackKey,
    keysoundsAdd, keysoundsAddNote, keysoundsDelete, keysoundsDeleteNote, keysoundsGenerate, keysoundsLoadVst3,
    keysoundsPreview, keysoundsQuantize, keysoundsScan, keysoundsSetVelocity, keysoundsTranspose, keysoundsView,
    msAtX, noteDragPayload, pitchAtY, pitchName, pitchRow, rollPitches, rollSpanMs, sidecarStatus, velocityAtY,
  } from "../lib/midi.js";
  import {
    readDrumVst3, sceneDrumHitsToNotes, syncDrumVst3Track, writeDrumVst3,
  } from "../lib/drumVst3.js";
  import { sustainSpanAt } from "../lib/pianoRoll.js";
  import ChordResolverPopover from "./ChordResolverPopover.svelte";
  import KeysoundBounceModal from "./KeysoundBounceModal.svelte";
  import KeysoundSetupBar from "./KeysoundSetupBar.svelte";
  import PianoKeyboard from "./PianoKeyboard.svelte";

  // Map › Keysounds & VST3…: the keysound tracks Rust keeps for this map as a piano roll, each
  // optionally played by a scanned VST3 instrument. menus.js calls globalThis.openKeysounds.
  let open = $state(false);
  let loading = $state(false);
  let error = $state("");
  let summary = $state(null);
  let name = $state("");
  let pluginId = $state("");
  let trackId = $state(null);
  let selected = $state(null);
  // The note being dragged: { note, mode, x0, y0, dx, dy } (a local ghost until pointerup).
  let drag = $state(null);
  // The velocity bar being dragged: { index, velocity }.
  let velDrag = $state(null);
  let rollScroll = $state(null);
  let velScroll = $state(null);
  let chordPopoverOpen = $state(false);
  let chordPolicy = $state("flatten_polyphonic");
  let bounceOpen = $state(false);
  let bouncePreview = $state(null);
  let bounceVelBuckets = $state(4);
  let bounceDurMs = $state(50);
  let bounceFlatten = $state(true);
  let bounceFormat = $state("ogg_vorbis");
  const storage = () => { try { return typeof window !== "undefined" ? window.localStorage : null; } catch { return null; } };
  let drumVst3 = $state(readDrumVst3(storage()));

  const VEL_PX = 64;
  const pitches = rollPitches();
  const tracks = $derived(summary?.tracks ?? []);
  const instruments = $derived(instrumentChoices(summary?.vst3Plugins));
  const track = $derived(tracks.find(t => t.id === trackId) ?? null);
  const notes = $derived(track?.notes ?? []);
  const sustainSpans = $derived(track?.sustainSpans ?? []);
  const chordConflicts = $derived(summary?.chordConflicts ?? []);
  const spanPx = $derived(rollSpanMs(notes) * PX_PER_MS);
  const beats = $derived(Array.from({ length: Math.ceil(rollSpanMs(notes) / 500) }, (_, i) => i * 500));

  async function send(payload) {
    if (!payload) return false;
    loading = true;
    error = "";
    try {
      summary = await invoke("keysounds_action", { payload });
      if (pluginId && !instrumentChoices(summary?.vst3Plugins).some(choice => choice.id === pluginId)) pluginId = "";
      if (summary?.activeTrackId != null && summary?.tracks?.some(t => t.id === summary.activeTrackId)) {
        trackId = summary.activeTrackId;
      } else if (!summary?.tracks?.some(t => t.id === trackId)) {
        trackId = summary?.tracks?.[0]?.id ?? null;
      }
      const current = summary?.tracks?.find(t => t.id === trackId);
      if (selected != null && !(selected < (current?.notes?.length ?? 0))) selected = null;
      return true;
    } catch (err) {
      error = String(err);
      return false;
    } finally {
      loading = false;
    }
  }

  async function selectTrack(nextId) {
    if (!Number.isFinite(nextId)) return;
    trackId = nextId;
    selected = null;
    await send({ kind: "select_track", trackId: nextId });
  }

  async function refreshBouncePreview(opts = {}) {
    const req = {
      kind: "preview_bounce",
      velocityBuckets: opts.velocityBuckets ?? bounceVelBuckets,
      durationBucketMs: opts.durationBucketMs ?? bounceDurMs,
      flattenChords: opts.flattenChords ?? bounceFlatten,
      format: opts.format ?? bounceFormat,
    };
    if (await send(req) && summary?.bouncePreview) {
      bouncePreview = summary.bouncePreview;
      await invoke("keysounds_dedup_report", {
        sourceNotes: bouncePreview.sourceNotes ?? 0,
        uniqueSamples: bouncePreview.samples?.length ?? 0,
        baseBytes: bouncePreview.baseBytes ?? 0,
        bouncedBytes: bouncePreview.bouncedBytes ?? 0,
      }).catch(() => {});
    }
  }

  async function openBounceModal() {
    bounceOpen = true;
    await refreshBouncePreview();
  }

  // Auditions a pitch; it does not go through send() so the buttons don't flicker per key.
  async function preview(pitch, velocity = 100) {
    try {
      await invoke("keysounds_action", { payload: keysoundsPreview(trackId, pitch, velocity) });
    } catch (err) {
      error = String(err);
    }
  }

  async function openKeysounds() {
    if (!bridge.scene) return;
    open = true;
    name = "";
    selected = null;
    drumVst3 = readDrumVst3(storage());
    if (drumVst3.pluginId) pluginId = drumVst3.pluginId;
    await send(keysoundsView());
  }

  async function toggleDrumVst3(checked) {
    const picked = summary?.vst3Plugins?.find(p => p.id === pluginId) ?? null;
    drumVst3 = writeDrumVst3(storage(), {
      enabled: checked,
      pluginId: picked?.id ?? drumVst3.pluginId ?? "",
      pluginName: picked?.name ?? drumVst3.pluginName ?? "",
      pluginPath: picked?.path ?? drumVst3.pluginPath ?? "",
    });
    if (!checked || loading) return;
    loading = true;
    error = "";
    try {
      const hits = sceneDrumHitsToNotes(bridge.scene, bridge.view?.drumView);
      const result = await syncDrumVst3Track(invoke, {
        plugin: picked ?? (drumVst3.pluginName ? { id: drumVst3.pluginId, name: drumVst3.pluginName, path: drumVst3.pluginPath } : null),
        hits,
      });
      if (result?.summary) summary = result.summary;
      if (result?.trackId != null) {
        trackId = result.trackId;
        selected = null;
      }
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  async function addTrack() {
    if (loading) return;
    const plugin = instruments.find(choice => choice.id === pluginId);
    const before = new Set(tracks.map(t => t.id));
    if (await send(keysoundsAdd(name, plugin?.name ?? null))) {
      name = "";
      trackId = summary?.tracks?.find(t => !before.has(t.id))?.id ?? trackId;
      selected = null;
    }
  }

  async function loadVst3() {
    if (loading) return;
    const plugin = summary?.vst3Plugins?.find(p => p.id === pluginId);
    await send(keysoundsLoadVst3(trackId, plugin));
  }

  async function generate() {
    if (loading) return;
    loading = true;
    error = "";
    try {
      summary = await invoke("keysounds_action", { payload: keysoundsGenerate() });
      const toast = generateToast(summary?.generated);
      if (toast) globalThis.showToast?.(toast);
      globalThis.act?.(null, true);
    } catch (err) {
      error = String(err);
      globalThis.showToast?.({ tone: "error", title: "Generate [Keysounds]", body: String(err) });
    } finally {
      loading = false;
    }
  }

  async function generateWithBounceOpts(opts) {
    if (loading) return;
    loading = true;
    error = "";
    try {
      const payload = {
        ...keysoundsGenerate(),
        velocityBuckets: opts.velocityBuckets ?? bounceVelBuckets,
        durationBucketMs: opts.durationBucketMs ?? bounceDurMs,
        flattenChords: opts.flattenChords ?? bounceFlatten,
        format: opts.format ?? bounceFormat,
      };
      summary = await invoke("keysounds_action", { payload });
      bounceOpen = false;
      const toast = generateToast(summary?.generated);
      if (toast) globalThis.showToast?.(toast);
      globalThis.act?.(null, true);
    } catch (err) {
      error = String(err);
      globalThis.showToast?.({ tone: "error", title: "Generate [Keysounds]", body: String(err) });
    } finally {
      loading = false;
    }
  }

  function trackAction(build) {
    if (loading || trackId == null) return;
    send(build(trackId));
  }

  // A press on an empty cell places a note there and plays it.
  async function onGridDown(event) {
    if (event.button !== 0 || event.target !== event.currentTarget || loading || trackId == null) return;
    const box = event.currentTarget.getBoundingClientRect();
    const startMs = msAtX(event.clientX - box.left);
    const pitch = pitchAtY(event.clientY - box.top);
    if (await send(keysoundsAddNote(trackId, startMs, pitch))) {
      selected = findNoteIndex(notes, startMs, pitch);
      preview(pitch);
    }
  }

  function onNoteDown(event, note, mode) {
    event.stopPropagation();
    if (event.button !== 0 || loading) return;
    selected = note.index;
    drag = { note, mode, x0: event.clientX, y0: event.clientY, dx: 0, dy: 0 };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onNoteMove(event) {
    if (!drag) return;
    drag.dx = event.clientX - drag.x0;
    drag.dy = event.clientY - drag.y0;
  }

  async function onNoteUp() {
    if (!drag) return;
    const { note, mode, dx, dy } = drag;
    drag = null;
    const payload = noteDragPayload(trackId, note, mode, dx, dy);
    if (!payload) return;
    if (await send(payload)) {
      if (mode === "move") {
        selected = findNoteIndex(notes, payload.startMs, payload.pitch);
        if (payload.pitch !== note.pitch) preview(payload.pitch, note.velocity);
      }
    }
  }

  function deleteNote(index) {
    if (loading || trackId == null || index == null) return;
    selected = null;
    send(keysoundsDeleteNote(trackId, index));
  }

  // The ghost's box while dragging, else the note's own.
  function noteBox(note) {
    let start = note.startMs, end = note.endMs, pitch = note.pitch;
    if (drag && drag.note.index === note.index) {
      const payload = noteDragPayload(trackId, note, drag.mode, drag.dx, drag.dy);
      if (payload?.kind === "move_note") { end = payload.startMs + (end - start); start = payload.startMs; pitch = payload.pitch; }
      if (payload?.kind === "resize_note") end = payload.endMs;
    }
    return { left: start * PX_PER_MS + "px", top: pitchRow(pitch) + "px", width: Math.max(4, (end - start) * PX_PER_MS) + "px" };
  }

  function velocityOf(note) {
    return velDrag?.index === note.index ? velDrag.velocity : note.velocity;
  }

  function onVelDown(event, note) {
    if (event.button !== 0 || loading) return;
    selected = note.index;
    const box = event.currentTarget.parentElement.getBoundingClientRect();
    velDrag = { index: note.index, velocity: velocityAtY(event.clientY - box.top, box.height), top: box.top, height: box.height };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onVelMove(event) {
    if (velDrag) velDrag.velocity = velocityAtY(event.clientY - velDrag.top, velDrag.height);
  }

  function onVelUp() {
    if (!velDrag) return;
    const { index, velocity } = velDrag;
    velDrag = null;
    const note = notes.find(n => n.index === index);
    if (note && note.velocity !== velocity) send(keysoundsSetVelocity(trackId, index, velocity));
  }

  function syncScroll() {
    if (rollScroll && velScroll) velScroll.scrollLeft = rollScroll.scrollLeft;
  }

  function closeModal() {
    if (loading) return;
    open = false;
  }

  function onKeydown(event) {
    if (!open) return;
    if (event.key === "Escape") {
      event.stopPropagation();
      closeModal();
      return;
    }
    const typing = /^(INPUT|SELECT|TEXTAREA)$/.test(event.target?.tagName ?? "");
    if (!typing && (event.key === "Delete" || event.key === "Backspace") && selected != null) {
      event.preventDefault();
      event.stopPropagation();
      deleteNote(selected);
    }
  }

  // The editor's timeline shortcuts stay off while this is open (boot.js reads the flag).
  $effect(() => {
    globalThis.dawsuModalOpen = open;
    return () => { globalThis.dawsuModalOpen = false; };
  });

  onMount(() => {
    globalThis.openKeysounds = openKeysounds;
    globalThis.keysoundsMicroAction = (action, value) => {
      if (action === "chordPolicy") {
        chordPolicy = value === "top_only" ? "top_note_only" : "flatten_polyphonic";
        send({ kind: "resolve_all_chords", policy: chordPolicy });
      } else if (action === "previewNote") {
        const note = selected != null ? notes[selected] : notes[0];
        if (note) preview(note.pitch, note.velocity);
      }
    };
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div class="ks-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal(); }}>
    <div class="ks-modal" role="dialog" aria-modal="true" aria-label="Keysounds and VST3">
      <header class="ks-head">
        <span class="ks-title">Keysounding Piano Roll</span>
        {#if summary?.targetDifficulty}
          <span class="ks-sub">{summary.targetDifficulty}</span>
        {/if}
        <button type="button" class="ks-close" aria-label="Close" onclick={closeModal}>×</button>
      </header>

      <KeysoundSetupBar
        open={true}
        {tracks}
        activeTrackId={trackId}
        chordCount={chordConflicts.length}
        canUndo={Boolean(summary?.canUndo)}
        canRedo={Boolean(summary?.canRedo)}
        dirty={Boolean(summary?.dirty)}
        onselecttrack={selectTrack}
        onimportmidi={() => { closeModal(); globalThis.openMidiImport?.(); }}
        onundo={() => send({ kind: "undo" })}
        onredo={() => send({ kind: "redo" })}
        onsave={() => send({ kind: "save_dawsuks" })}
        onopenchords={() => { chordPopoverOpen = !chordPopoverOpen; }}
        onopenbounce={openBounceModal}
      />

      <div class="ks-bar">
        <div class="ks-group">
          <select aria-label="Keysound track" value={trackId ?? ""} disabled={!tracks.length}
            onchange={(e) => { selectTrack(Number(e.currentTarget.value)); }}>
            {#if !tracks.length}<option value="">No tracks</option>{/if}
            {#each tracks as t (t.id)}
              <option value={t.id}>{t.name} · {t.noteCount} notes</option>
            {/each}
          </select>
          <button type="button" aria-label={track ? `Delete ${track.name}` : "Delete track"} disabled={loading || !track}
            onclick={() => send(keysoundsDelete(trackId))}>Delete</button>
          <input type="text" placeholder="Keysound Track" aria-label="Track name" bind:value={name}
            onkeydown={(e) => { if (e.key === "Enter") addTrack(); }} />
          <button type="button" disabled={loading} onclick={addTrack}>Add track</button>
        </div>
        <div class="ks-group">
          <select aria-label="VST3 instrument" bind:value={pluginId} disabled={!instruments.length}
            onchange={() => {
              if (!drumVst3.enabled) return;
              const picked = summary?.vst3Plugins?.find(p => p.id === pluginId) ?? null;
              drumVst3 = writeDrumVst3(storage(), {
                enabled: true,
                pluginId: picked?.id ?? "",
                pluginName: picked?.name ?? "",
                pluginPath: picked?.path ?? "",
              });
            }}>
            <option value="">No instrument</option>
            {#each instruments as choice (choice.id)}
              <option value={choice.id}>{choice.name}{choice.vendor ? ` · ${choice.vendor}` : ""}</option>
            {/each}
          </select>
          <button type="button" disabled={loading || !track || !pluginId || !summary?.sidecarAvailable}
            onclick={loadVst3}>Load VST3</button>
          <button type="button" disabled={loading} onclick={() => send(keysoundsScan())}>Scan VST3</button>
          <label class="ks-drum-vst3">
            <input type="checkbox" checked={drumVst3.enabled} disabled={loading}
              onchange={(e) => toggleDrumVst3(e.currentTarget.checked)} />
            <span>Use VST3 sounds for drum map</span>
          </label>
        </div>
        <div class="ks-group">
          <button type="button" disabled={loading || !notes.length} onclick={() => trackAction(id => keysoundsQuantize(id, QUANTIZE_MS))}>Quantize 125 ms</button>
          <button type="button" aria-label="Transpose down an octave" disabled={loading || !notes.length} onclick={() => trackAction(id => keysoundsTranspose(id, -12))}>−12</button>
          <button type="button" aria-label="Transpose down a semitone" disabled={loading || !notes.length} onclick={() => trackAction(id => keysoundsTranspose(id, -1))}>−1</button>
          <button type="button" aria-label="Transpose up a semitone" disabled={loading || !notes.length} onclick={() => trackAction(id => keysoundsTranspose(id, 1))}>+1</button>
          <button type="button" aria-label="Transpose up an octave" disabled={loading || !notes.length} onclick={() => trackAction(id => keysoundsTranspose(id, 12))}>+12</button>
        </div>
      </div>

      <div class="ks-status">
        {#if error}
          <span class="ks-error" role="alert">{error}</span>
        {:else if summary && !instruments.length}
          <span class="ks-empty">{NO_PLUGINS}</span>
        {:else}
          <span class:ks-ready={summary?.sidecarAvailable}>{sidecarStatus(summary)}</span>
          {#if track}<span class="ks-meta">{track.instrument || "No instrument"}</span>{/if}
        {/if}
      </div>

      {#if !summary && loading}
        <p class="ks-caption">Loading…</p>
      {:else if !track}
        <p class="ks-caption">No keysound tracks yet. Add one above; MIDI Record fills the first track.</p>
        <div class="ks-empty-keys">
          <PianoKeyboard lowPitch={60} highPitch={72} rowHeight={18} onpreview={pitch => preview(pitch)} />
        </div>
      {:else}
        <div class="ks-roll" bind:this={rollScroll} onscroll={syncScroll} style:--ks-row={ROW_PX + "px"}
          style:--ks-roll-h={pitches.length * ROW_PX + "px"} style:--ks-span={spanPx + "px"}>
          <div class="ks-keys">
            {#each pitches as pitch (pitch)}
              <button type="button" class="ks-key" class:black={isBlackKey(pitch)} class:c={pitch % 12 === 0}
                aria-label={`Play ${pitchName(pitch)}`}
                onpointerdown={() => preview(pitch)}>{pitch % 12 === 0 ? pitchName(pitch) : ""}</button>
            {/each}
          </div>
          <div class="ks-grid" role="grid" tabindex="-1" aria-label="Piano roll"
            onpointerdown={onGridDown} oncontextmenu={(e) => e.preventDefault()}>
            {#each pitches as pitch (pitch)}
              <div class="ks-lane" class:black={isBlackKey(pitch)} class:c={pitch % 12 === 0}
                style:--ks-top={pitchRow(pitch) + "px"}></div>
            {/each}
            {#each beats as beat (beat)}
              <div class="ks-beat" class:bar={beat % 2000 === 0} class:sustained={Boolean(sustainSpanAt(sustainSpans, beat))}
                style:--ks-left={beat * PX_PER_MS + "px"}></div>
            {/each}
            {#each notes as note (note.index)}
              <div class="ks-note" class:selected={selected === note.index} class:muted={note.muted}
                role="gridcell" tabindex="-1" aria-label={`${pitchName(note.pitch)} at ${Math.round(note.startMs)} ms`}
                style:--ks-left={noteBox(note).left} style:--ks-top={noteBox(note).top}
                style:--ks-width={noteBox(note).width} style:--ks-alpha={0.45 + 0.55 * note.velocity / 127}
                onpointerdown={(e) => onNoteDown(e, note, "move")} onpointermove={onNoteMove} onpointerup={onNoteUp}
                oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); deleteNote(note.index); }}>
                <span class="ks-note-name">{pitchName(note.pitch)}</span>
                <span class="ks-edge" role="presentation"
                  onpointerdown={(e) => onNoteDown(e, note, "resize")} onpointermove={onNoteMove} onpointerup={onNoteUp}></span>
              </div>
            {/each}
          </div>
        </div>
        <div class="ks-vel-wrap">
          <span class="ks-vel-label">Velocity</span>
          <div class="ks-vel" bind:this={velScroll}>
            <div class="ks-vel-lane" style:--ks-span={spanPx + "px"} style:--ks-vel-h={VEL_PX + "px"}>
              {#each notes as note (note.index)}
                <div class="ks-vel-hit" role="slider" tabindex="-1" aria-label={`Velocity of ${pitchName(note.pitch)}`}
                  aria-valuemin="1" aria-valuemax="127" aria-valuenow={velocityOf(note)}
                  style:--ks-left={note.startMs * PX_PER_MS - 4 + "px"}
                  onpointerdown={(e) => onVelDown(e, note)} onpointermove={onVelMove} onpointerup={onVelUp}>
                  <div class="ks-vel-bar" class:selected={selected === note.index}
                    style:--ks-bar-h={velocityOf(note) / 127 * VEL_PX + "px"}></div>
                </div>
              {/each}
            </div>
          </div>
        </div>
      {/if}

      <footer class="ks-foot">
        <span class="ks-hint">Click to place · drag to move · drag the right edge to resize · right-click or Delete to remove</span>
        <button type="button" class="ks-gold" disabled={loading || !tracks.some(t => t.noteCount)} onclick={() => generate()}>Generate [Keysounds]</button>
        <button type="button" disabled={loading} onclick={closeModal}>Close</button>
      </footer>
      <ChordResolverPopover
        open={chordPopoverOpen && chordConflicts.length > 0}
        conflict={chordConflicts[0] ?? null}
        totalConflicts={chordConflicts.length}
        bind:policy={chordPolicy}
        onaudition={c => { for (const p of c.pitches ?? []) preview(p); }}
        onapplyone={(c, pol) => send({ kind: "resolve_chord", trackId: c.trackId, timeMs: c.timeMs, policy: pol })}
        onapplyall={pol => { chordPopoverOpen = false; send({ kind: "resolve_all_chords", policy: pol }); }}
        onclose={() => { chordPopoverOpen = false; }}
      />
      <KeysoundBounceModal
        open={bounceOpen}
        busy={loading}
        sourceNotes={bouncePreview?.sourceNotes ?? notes.length}
        baseBytes={bouncePreview?.baseBytes ?? 0}
        bouncedBytes={bouncePreview?.bouncedBytes ?? 0}
        samples={bouncePreview?.samples ?? []}
        bind:velocityBuckets={bounceVelBuckets}
        bind:durationBucketMs={bounceDurMs}
        bind:flattenChords={bounceFlatten}
        bind:format={bounceFormat}
        onrender={refreshBouncePreview}
        onaudition={sample => { for (const p of sample.pitches ?? [60]) preview(p); }}
        oncommit={() => generateWithBounceOpts({ velocityBuckets: bounceVelBuckets, durationBucketMs: bounceDurMs, flattenChords: bounceFlatten, format: bounceFormat })}
        onclose={() => { bounceOpen = false; }}
      />
    </div>
  </div>
{/if}

<style>
  .ks-backdrop {
    position: fixed;
    inset: 0;
    z-index: 62;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.68);
  }
  .ks-modal {
    display: flex;
    flex-direction: column;
    width: min(1100px, calc(100vw - 40px));
    height: min(760px, calc(100vh - 48px));
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.62);
    overflow: hidden;
  }
  .ks-head {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 46px;
    padding: 0 18px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .ks-title {
    font-size: 14px;
    font-weight: 600;
  }
  .ks-sub {
    font-size: 12px;
    color: var(--editor-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ks-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    font-size: 18px;
  }
  .ks-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 16px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .ks-group {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ks-group input {
    width: 130px;
  }
  .ks-drum-vst3 {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--editor-text);
    cursor: pointer;
    white-space: nowrap;
  }
  .ks-drum-vst3 input {
    width: auto;
    margin: 0;
    accent-color: var(--editor-accent);
  }
  .ks-status {
    display: flex;
    gap: 12px;
    min-height: 26px;
    align-items: center;
    padding: 0 14px;
    font-size: 11px;
    color: var(--editor-muted);
    border-bottom: 1px solid var(--editor-border);
  }
  .ks-ready {
    color: #0a84ff;
  }
  .ks-meta {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }
  .ks-empty {
    color: var(--editor-warning);
  }
  .ks-error {
    color: var(--editor-error);
  }
  .ks-caption {
    margin: 0;
    padding: 24px;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .ks-roll {
    flex: 1;
    display: flex;
    min-height: 0;
    overflow: auto;
    background: var(--editor-panel);
  }
  .ks-keys {
    position: sticky;
    left: 0;
    z-index: 3;
    flex: 0 0 56px;
    display: flex;
    flex-direction: column;
    height: var(--ks-roll-h);
    border-right: 1px solid var(--editor-border);
    background: #1c1c1e;
  }
  .ks-key {
    flex: 0 0 var(--ks-row);
    height: var(--ks-row);
    padding: 0 6px 0 0;
    border: 0;
    border-bottom: 1px solid rgba(0, 0, 0, 0.5);
    border-radius: 0;
    background: #d8d8dc;
    color: #151517;
    font-size: 9px;
    line-height: 1;
    text-align: right;
  }
  .ks-key.black {
    background: #151517;
    color: var(--editor-muted);
  }
  .ks-key:active {
    background: #0a84ff;
  }
  .ks-grid {
    position: relative;
    flex: 0 0 auto;
    width: var(--ks-span);
    height: var(--ks-roll-h);
    cursor: crosshair;
  }
  .ks-lane {
    position: absolute;
    top: var(--ks-top);
    left: 0;
    right: 0;
    height: var(--ks-row);
    box-sizing: border-box;
    pointer-events: none;
    background: #1c1c1e;
    border-bottom: 1px solid rgba(255, 255, 255, 0.03);
  }
  .ks-lane.black {
    background: #151517;
  }
  .ks-lane.c {
    border-bottom-color: rgba(255, 255, 255, 0.1);
  }
  .ks-beat {
    position: absolute;
    top: 0;
    bottom: 0;
    left: var(--ks-left);
    width: 1px;
    pointer-events: none;
    background: rgba(255, 255, 255, 0.05);
  }
  .ks-beat.bar {
    background: rgba(255, 255, 255, 0.14);
  }
  .ks-note {
    position: absolute;
    z-index: 1;
    left: var(--ks-left);
    top: var(--ks-top);
    width: var(--ks-width);
    height: calc(var(--ks-row) - 1px);
    opacity: var(--ks-alpha);
    box-sizing: border-box;
    border: 1px solid rgba(0, 0, 0, 0.45);
    border-radius: 3px;
    background: #0a84ff;
    cursor: grab;
    overflow: hidden;
    touch-action: none;
  }
  .ks-note.selected {
    z-index: 2;
    background: #efc16a;
    border-color: #fff3d6;
  }
  .ks-note.muted {
    filter: grayscale(1);
  }
  .ks-note-name {
    position: absolute;
    left: 3px;
    top: 0;
    font-size: 9px;
    line-height: 12px;
    color: #08080a;
    pointer-events: none;
  }
  .ks-edge {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 6px;
    cursor: ew-resize;
  }
  .ks-vel-wrap {
    display: flex;
    border-top: 1px solid var(--editor-border);
    background: #151517;
  }
  .ks-vel-label {
    flex: 0 0 56px;
    padding: 6px;
    box-sizing: border-box;
    border-right: 1px solid var(--editor-border);
    font-size: 10px;
    color: var(--editor-muted);
  }
  .ks-vel {
    flex: 1;
    overflow: hidden;
  }
  .ks-vel-lane {
    position: relative;
    width: var(--ks-span);
    height: var(--ks-vel-h);
  }
  .ks-vel-hit {
    position: absolute;
    top: 0;
    bottom: 0;
    left: var(--ks-left);
    width: 9px;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    cursor: ns-resize;
    touch-action: none;
  }
  .ks-vel-bar {
    width: 4px;
    height: var(--ks-bar-h);
    border-radius: 2px 2px 0 0;
    background: #0a84ff;
    pointer-events: none;
  }
  .ks-vel-bar.selected {
    background: #efc16a;
  }
  .ks-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 50px;
    padding: 0 18px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .ks-hint {
    margin-right: auto;
    font-size: 11px;
    color: var(--editor-muted);
  }
  .ks-gold {
    background: #efc16a;
    color: #151517;
    font-weight: 600;
  }
</style>
