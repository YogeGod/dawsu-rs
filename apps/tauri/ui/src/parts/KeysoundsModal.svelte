<script>
  import { onMount, untrack } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    NO_PLUGINS, findNoteIndex, generateToast, instrumentChoices,
    keysoundsAdd, keysoundsAddNote, keysoundsDelete, keysoundsDeleteNote, keysoundsGenerate,
    keysoundsHumanise, keysoundsLoadVst3, keysoundsPreview, keysoundsSetVelocity, keysoundsStrum,
    keysoundsSyncToObjects, keysoundsVelocityRamp, keysoundsView, scalePitchMask,
    sidecarStatus, visibleRollPitches,
  } from "../lib/midi.js";
  import {
    createMusicalTypingState, handleMusicalTypingKeyDown, handleMusicalTypingKeyUp,
  } from "../lib/musicalTyping.js";
  import {
    readDrumVst3, sceneDrumHitsToNotes, syncDrumVst3Track, writeDrumVst3,
  } from "../lib/drumVst3.js";
  import ChordResolverPopover from "./ChordResolverPopover.svelte";
  import KeysoundBounceModal from "./KeysoundBounceModal.svelte";
  import KeysoundSetupBar from "./KeysoundSetupBar.svelte";
  import PianoKeyboard from "./PianoKeyboard.svelte";
  import KeysoundTrackBar from "./keysounds/KeysoundTrackBar.svelte";
  import KeysoundRollSurface from "./keysounds/KeysoundRollSurface.svelte";

  // Keysounds mode: the keysound tracks Rust keeps for this map as a piano roll, each optionally
  // played by a scanned VST3 instrument. It covers the hitsound lanes inside #scroll, under the
  // editor's own ruler; the roll and the velocity lane are painted by the editor's paint().
  let open = $state(false);
  let loading = $state(false);
  let error = $state("");
  // Replaced whole by every answer from Rust and never written in place, so raw: a track's
  // thousands of notes stay plain objects for the roll's paint to read.
  let summary = $state.raw(null);
  let name = $state("");
  let pluginId = $state("");
  let trackId = $state(null);
  let selected = $state(null);
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
  let showGhosts = $state(true);
  let showObjectLines = $state(true);
  let scaleRoot = $state(0);
  let scaleKind = $state("chromatic");
  let foldToScale = $state(false);
  let typingState = $state(createMusicalTypingState());
  const heldTypingKeys = new Set();

  const scaleMask = $derived(scalePitchMask(scaleRoot, scaleKind));
  const pitches = $derived(visibleRollPitches(scaleRoot, scaleKind, foldToScale));
  const visiblePitchSet = $derived(new Set(pitches));
  const tracks = $derived(summary?.tracks ?? []);
  const instruments = $derived(instrumentChoices(summary?.vst3Plugins));
  const track = $derived(tracks.find(t => t.id === trackId) ?? null);
  const notes = $derived(track?.notes ?? []);
  const anySolo = $derived(tracks.some(t => t.solo));
  const ghostNotes = $derived(
    showGhosts
      ? tracks
          .filter(t => t.id !== trackId && !t.muted && (!anySolo || t.solo))
          .flatMap(t =>
            (t.notes ?? [])
              .filter(n => !foldToScale || visiblePitchSet.has(n.pitch))
              .map(n => ({ ...n, trackId: t.id }))
          )
      : []
  );
  const objectTimes = $derived(
    showObjectLines
      ? (bridge.scene?.moments ?? [])
          .filter(m => (Array.isArray(m) ? Boolean(m[2]) : Boolean(m?.hasObject ?? m?.place)))
          .map(m => (Array.isArray(m) ? m[0] : m?.time))
          .filter(t => Number.isFinite(t))
      : []
  );
  const sustainSpans = $derived(track?.sustainSpans ?? []);
  const chordConflicts = $derived(summary?.chordConflicts ?? []);

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
        bridge.showToast({ tone: "success", title: "Opened Keysound project", body: res.loadedPath ?? "" });
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
    if (bridge.mode !== "keysounds") {
      bridge.mode = "keysounds";
      if (typeof document !== "undefined") {
        const arrange = document.getElementById("editor-arrange") ?? document.querySelector(".arrange");
        arrange?.classList.add("keysounds-active");
        document.getElementById("keysounds-view")?.setAttribute("aria-pressed", "true");
        document.getElementById("samples-view")?.setAttribute("aria-pressed", "false");
        document.getElementById("drums-view")?.setAttribute("aria-pressed", "false");
      }
    }
    name = "";
    selected = null;
    drumVst3 = readDrumVst3(storage());
    if (drumVst3.pluginId) pluginId = drumVst3.pluginId;
    await send(keysoundsView());
  }

  async function syncToObjects() {
    if (loading || trackId == null) return;
    if (await send(keysoundsSyncToObjects(trackId))) {
      const res = summary?.syncResult;
      if (res) {
        const synced = res.syncedCount ?? 0;
        const unsynced = res.unsyncedCount ?? 0;
        bridge.showToast({
          tone: "success",
          title: "Synced to beatmap objects",
          body: `${synced} ${synced === 1 ? "note" : "notes"} synced` + (unsynced ? ` (${unsynced} not near an object).` : "."),
        });
      }
    }
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
        bridge.showToast({ tone: "success", title: "SoundFont loaded", body: res.loadedPath ?? "" });
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
      if (toast) bridge.showToast(toast);
      bridge.callAction("act", null, true);
    } catch (err) {
      error = String(err);
      bridge.showToast({ tone: "error", title: "Generate [Keysounds]", body: String(err) });
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
      if (toast) bridge.showToast(toast);
      bridge.callAction("act", null, true);
    } catch (err) {
      error = String(err);
      bridge.showToast({ tone: "error", title: "Generate [Keysounds]", body: String(err) });
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

  function closeKeysounds() {
    open = false;
    typingState = { ...typingState, enabled: false };
    heldTypingKeys.clear();
  }

  // Leaves Keysounds mode for the mode it came from; the bridge then closes this part.
  function closeModal() {
    if (loading) return;
    if (bridge.mode === "keysounds") bridge.setMode(bridge.view?.drumView?.enabled ? "drums" : "hitsounds");
    else closeKeysounds();
  }

  function toggleMusicalTyping() {
    const nextEnabled = !typingState.enabled;
    if (!nextEnabled) heldTypingKeys.clear();
    typingState = { ...typingState, enabled: nextEnabled };
  }

  function onKeydown(event) {
    if (!open) return;
    const typingInput = /^(INPUT|SELECT|TEXTAREA)$/.test(event.target?.tagName ?? "");
    if (event.altKey && !event.ctrlKey && !event.metaKey && (event.code === "KeyK" || event.key?.toLowerCase() === "k")) {
      event.preventDefault();
      event.stopPropagation();
      toggleMusicalTyping();
      return;
    }
    if (!typingInput && typingState.enabled) {
      const result = handleMusicalTypingKeyDown(typingState, heldTypingKeys, event);
      if (result.action) {
        event.preventDefault();
        event.stopPropagation();
        typingState = result.state;
        if (result.action === "note") {
          preview(result.pitch, result.velocity);
          if ((bridge.view?.tapRecordingArmed ?? bridge.latestView?.tapRecordingArmed) && trackId != null) {
            const playheadMs = bridge.currentPositionMs();
            send(keysoundsAddNote(trackId, playheadMs, result.pitch, result.velocity));
          }
        }
        return;
      }
    }
    if (event.key === "Escape") {
      event.stopPropagation();
      if (bounceOpen) {
        bounceOpen = false;
      } else if (chordPopoverOpen) {
        chordPopoverOpen = false;
      }
      return;
    }
    if (!typingInput && (event.ctrlKey || event.metaKey) && !event.altKey) {
      const k = event.key?.toLowerCase();
      if (k === "z" && !event.shiftKey) {
        event.preventDefault();
        event.stopPropagation();
        send({ kind: "undo" });
        return;
      }
      if (k === "y" || (k === "z" && event.shiftKey)) {
        event.preventDefault();
        event.stopPropagation();
        send({ kind: "redo" });
        return;
      }
    }
    if (!typingInput && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (event.code === "Space") {
        event.preventDefault();
        event.stopPropagation();
        bridge.callAction("command", "play");
        return;
      }
      if (event.code === "Numpad0") {
        event.preventDefault();
        event.stopPropagation();
        bridge.callAction("command", "stop");
        return;
      }
      if (event.code === "Home") {
        event.preventDefault();
        event.stopPropagation();
        bridge.callAction("command", "start");
        return;
      }
      if (event.code === "KeyL" && !typingState.enabled) {
        event.preventDefault();
        event.stopPropagation();
        bridge.callAction("command", "cycle");
        return;
      }
    }
    if (!typingInput && (event.key === "Delete" || event.key === "Backspace") && selected != null) {
      event.preventDefault();
      event.stopPropagation();
      deleteNote(selected);
    }
  }

  function onKeyup(event) {
    if (!open) return;
    handleMusicalTypingKeyUp(heldTypingKeys, event.code);
  }

  function onBlur() {
    heldTypingKeys.clear();
  }

  // Whatever switched to Keysounds mode, the roll opens with it: with the mode on and this part
  // closed, the hitsound lanes would show through and take edits.
  $effect(() => {
    if (bridge.mode === "keysounds" && bridge.scene && !open) untrack(() => openKeysounds());
  });

  // The editor's timeline shortcuts stay off while this is open (boot.js reads the flag).
  $effect(() => {
    bridge.setModalOpen("keysounds", open);
    return () => { bridge.setModalOpen("keysounds", false); };
  });

  onMount(() => {
    const disposeModal = bridge.registerModal("openKeysounds", openKeysounds);
    const disposeClose = bridge.registerAction("closeKeysounds", closeKeysounds);
    const disposeAction = bridge.registerAction("keysoundsMicroAction", (action, value) => {
      if (action === "chordPolicy") {
        chordPolicy = value === "top_only" ? "top_note_only" : "flatten_polyphonic";
        send({ kind: "resolve_all_chords", policy: chordPolicy });
      } else if (action === "previewNote") {
        const note = selected != null ? notes[selected] : notes[0];
        if (note) preview(note.pitch, note.velocity);
      }
    });
    // The velocity lane sits in the bottom panel, outside this part: it reports here.
    const disposeRoll = bridge.registerAction("keysoundsRoll", (action, first, second) => {
      if (loading || trackId == null) return;
      if (action === "select") {
        selected = first;
      } else if (action === "velocity") {
        const note = notes.find(n => n.index === first);
        if (note && note.velocity !== second) setVelocity(first, second);
      } else if (action === "ramp") {
        send(keysoundsVelocityRamp(trackId, first.startMs, first.endMs, first.startVel, first.endVel, first.curve));
      }
    });
    return () => {
      disposeModal();
      disposeClose();
      disposeAction();
      disposeRoll();
    };
  });
</script>

<svelte:window onkeydown={onKeydown} onkeyup={onKeyup} onblur={onBlur} />

{#if open}
  <section class="ks-workspace" aria-label="Keysounds and VST3">
      <KeysoundSetupBar
        open={true}
        {tracks}
        activeTrackId={trackId}
        chordCount={chordConflicts.length}
        canUndo={Boolean(summary?.canUndo)}
        canRedo={Boolean(summary?.canRedo)}
        dirty={Boolean(summary?.dirty)}
        onselecttrack={selectTrack}
        onimportmidi={() => { closeModal(); bridge.openModal("openMidiImport"); }}
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
        bind:scaleRoot
        bind:scaleKind
        {foldToScale}
        {instruments}
        sidecarAvailable={Boolean(summary?.sidecarAvailable)}
        {drumVst3}
        notesCount={notes.length}
        {showGhosts}
        {showObjectLines}
        {typingState}
        ondeletetrack={() => send(keysoundsDelete(trackId))}
        onaddtrack={addTrack}
        onpluginchange={onPluginChange}
        onloadvst3={loadVst3}
        onloadsoundfont={loadSoundfont}
        onselectpreset={selectSoundfontPreset}
        onsend={send}
        ontoggledrumvst3={toggleDrumVst3}
        ontrackaction={trackAction}
        onsynctoobjects={syncToObjects}
        ontoggleghosts={() => { showGhosts = !showGhosts; }}
        ontoggleobjectlines={() => { showObjectLines = !showObjectLines; }}
        ontoggletyping={toggleMusicalTyping}
        ontogglefold={() => { if (scaleKind !== "chromatic") foldToScale = !foldToScale; }}
        onscalechange={nextScale => { if (nextScale === "chromatic") foldToScale = false; }}
        onstrum={() => trackAction(id => keysoundsStrum(id))}
        onhumanise={() => trackAction(id => keysoundsHumanise(id))}
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
        <div class="ks-fill"><p class="ks-caption">Loading…</p></div>
      {:else if !track}
        <div class="ks-fill">
          <p class="ks-caption">No keysound tracks yet. Add one above; MIDI Record fills the first track.</p>
          <div class="ks-empty-keys">
            <PianoKeyboard lowPitch={60} highPitch={72} rowHeight={18} onpreview={pitch => preview(pitch)} />
          </div>
        </div>
      {:else}
        <KeysoundRollSurface
          {pitches}
          {scaleMask}
          {foldToScale}
          {sustainSpans}
          {notes}
          {ghostNotes}
          {objectTimes}
          bind:selected
          {loading}
          {trackId}
          {preview}
          onaddnote={addNote}
          ondragnote={dragNote}
          {deleteNote}
        />
      {/if}

      <footer class="ks-foot">
        <span class="ks-hint">Click to place · drag to move · drag right edge to resize · right-click or Delete to remove · velocity in the bottom panel · Alt+K typing</span>
        <button type="button" class="ks-gold" disabled={loading || !tracks.some(t => t.noteCount)} onclick={() => generate()}>Generate [Keysounds]</button>
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
  </section>
{/if}

<style>
  .ks-workspace {
    display: flex;
    flex-direction: column;
    flex: 1;
    width: 100%;
    height: 100%;
    min-height: 0;
    color: var(--editor-text);
    overflow: hidden;
  }
  .ks-fill {
    flex: 1;
    min-height: 0;
    background: var(--editor-panel);
  }
  .ks-chord-layer {
    position: fixed;
    inset: 0;
    z-index: 66;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
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
    background: var(--editor-panel);
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
