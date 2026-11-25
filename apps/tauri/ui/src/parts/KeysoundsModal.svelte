<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    NO_PLUGINS, PX_PER_MS, findNoteIndex, generateToast, instrumentChoices,
    keysoundsAdd, keysoundsAddNote, keysoundsDelete, keysoundsDeleteNote, keysoundsGenerate, keysoundsLoadVst3,
    keysoundsPreview, keysoundsSetVelocity, keysoundsView,
    rollPitches, rollSpanMs, sidecarStatus,
  } from "../lib/midi.js";
  import {
    readDrumVst3, sceneDrumHitsToNotes, syncDrumVst3Track, writeDrumVst3,
  } from "../lib/drumVst3.js";
  import ChordResolverPopover from "./ChordResolverPopover.svelte";
  import KeysoundBounceModal from "./KeysoundBounceModal.svelte";
  import KeysoundSetupBar from "./KeysoundSetupBar.svelte";
  import PianoKeyboard from "./PianoKeyboard.svelte";
  import KeysoundTrackBar from "./keysounds/KeysoundTrackBar.svelte";
  import KeysoundPianoRoll from "./keysounds/KeysoundPianoRoll.svelte";
  import KeysoundVelocityLane from "./keysounds/KeysoundVelocityLane.svelte";

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

  async function openDawsuks() {
    if (loading) return;
    loading = true;
    error = "";
    try {
      const res = await invoke("keysounds_action", { payload: { kind: "browse_dawsuks" } });
      if (res?.picked) {
        summary = res;
        trackId = res.activeTrackId ?? res.tracks?.[0]?.id ?? null;
        selected = null;
        globalThis.showToast?.({ tone: "success", title: "Opened Keysound project", body: res.loadedPath ?? "" });
      }
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  async function cancelBounce() {
    await invoke("keysounds_action", { payload: { kind: "cancel_bounce" } }).catch(() => {});
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

  function onPluginChange() {
    if (!drumVst3.enabled) return;
    const picked = summary?.vst3Plugins?.find(p => p.id === pluginId) ?? null;
    drumVst3 = writeDrumVst3(storage(), {
      enabled: true,
      pluginId: picked?.id ?? "",
      pluginName: picked?.name ?? "",
      pluginPath: picked?.path ?? "",
    });
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
        await selectTrack(result.trackId);
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

  async function loadSoundfont() {
    if (loading || trackId == null) return;
    loading = true;
    error = "";
    try {
      const res = await invoke("keysounds_action", { payload: { kind: "browse_soundfont", trackId } });
      if (res?.picked) {
        summary = res;
        globalThis.showToast?.({ tone: "success", title: "SoundFont loaded", body: res.loadedPath ?? "" });
      }
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  async function selectSoundfontPreset(presetIndex) {
    if (loading || trackId == null) return;
    await send({ kind: "set_soundfont_preset", trackId, presetIndex });
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

  async function addNote(startMs, pitch) {
    if (await send(keysoundsAddNote(trackId, startMs, pitch))) {
      selected = findNoteIndex(notes, startMs, pitch);
      preview(pitch);
    }
  }

  async function dragNote(note, mode, payload) {
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

  function setVelocity(index, velocity) {
    send(keysoundsSetVelocity(trackId, index, velocity));
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
      if (bounceOpen) {
        bounceOpen = false;
      } else if (chordPopoverOpen) {
        chordPopoverOpen = false;
      } else {
        closeModal();
      }
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
        onopen={openDawsuks}
        onundo={() => send({ kind: "undo" })}
        onredo={() => send({ kind: "redo" })}
        onsave={() => send({ kind: "save_dawsuks" })}
        onopenchords={() => { chordPopoverOpen = !chordPopoverOpen; }}
        onopenbounce={openBounceModal}
      />

      <KeysoundTrackBar
        {track}
        {loading}
        bind:name
        bind:pluginId
        {instruments}
        sidecarAvailable={Boolean(summary?.sidecarAvailable)}
        {drumVst3}
        notesCount={notes.length}
        ondeletetrack={() => send(keysoundsDelete(trackId))}
        onaddtrack={addTrack}
        onpluginchange={onPluginChange}
        onloadvst3={loadVst3}
        onloadsoundfont={loadSoundfont}
        onselectpreset={selectSoundfontPreset}
        onsend={send}
        ontoggledrumvst3={toggleDrumVst3}
        ontrackaction={trackAction}
      />

      <div class="ks-status">
        {#if error}
          <span class="ks-error" role="alert">{error}</span>
        {:else if track?.isSoundfont}
          <span class="ks-ready">SoundFont / SFZ ready · {track.instrument}</span>
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
        <KeysoundPianoRoll
          bind:rollScroll
          {pitches}
          {spanPx}
          {beats}
          {sustainSpans}
          {notes}
          bind:selected
          {loading}
          {trackId}
          onsyncscroll={syncScroll}
          {preview}
          onaddnote={addNote}
          ondragnote={dragNote}
          {deleteNote}
        />
        <KeysoundVelocityLane
          bind:velScroll
          {spanPx}
          {notes}
          bind:selected
          {loading}
          onsetvelocity={setVelocity}
        />
      {/if}

      <footer class="ks-foot">
        <span class="ks-hint">Click to place · drag to move · drag the right edge to resize · right-click or Delete to remove</span>
        <button type="button" class="ks-gold" disabled={loading || !tracks.some(t => t.noteCount)} onclick={() => generate()}>Generate [Keysounds]</button>
        <button type="button" disabled={loading} onclick={closeModal}>Close</button>
      </footer>
      {#if chordPopoverOpen && chordConflicts.length > 0}
        <div class="ks-chord-layer" role="presentation"
          onclick={e => { if (e.target === e.currentTarget) chordPopoverOpen = false; }}>
          <ChordResolverPopover
            open={true}
            conflict={chordConflicts[0] ?? null}
            totalConflicts={chordConflicts.length}
            bind:policy={chordPolicy}
            onaudition={c => { for (const p of c.pitches ?? []) preview(p); }}
            onapplyone={(c, pol) => send({ kind: "resolve_chord", trackId: c.trackId, timeMs: c.timeMs, policy: pol })}
            onapplyall={pol => { chordPopoverOpen = false; send({ kind: "resolve_all_chords", policy: pol }); }}
            onclose={() => { chordPopoverOpen = false; }}
          />
        </div>
      {/if}
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
        oncancel={cancelBounce}
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
  .ks-chord-layer {
    position: fixed;
    inset: 0;
    z-index: 66;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
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
