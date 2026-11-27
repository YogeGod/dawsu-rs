<script>
  import { onMount, untrack } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    NO_PLUGINS, findNoteIndex, generateToast, instrumentChoices,
    keysoundsAdd, keysoundsAddNote, keysoundsDelete, keysoundsDeleteNote, keysoundsGenerate,
    keysoundsHumanise, keysoundsLoadVst3, keysoundsPreview, keysoundsRenameTrack, keysoundsScan,
    keysoundsSetVelocity, keysoundsSnapToGrid, keysoundsStrum, keysoundsSyncToObjects, keysoundsToggleMute,
    keysoundsToggleSolo, keysoundsTranspose, keysoundsTrimOverlaps, keysoundsVelocityRamp, keysoundsView,
    scalePitchMask, sidecarStatus, visibleRollPitches,
  } from "../lib/midi.js";
  import { keysoundsStatus } from "../lib/keysounds.js";
  import {
    createMusicalTypingState, handleMusicalTypingKeyDown, handleMusicalTypingKeyUp,
  } from "../lib/musicalTyping.js";
  import { readDrumVst3, writeDrumVst3 } from "../lib/drumVst3.js";
  import ChordResolverPopover from "./ChordResolverPopover.svelte";
  import KeysoundBounceModal from "./KeysoundBounceModal.svelte";
  import KeysoundTrackList from "./keysounds/KeysoundTrackList.svelte";
  import KeysoundRollSurface from "./keysounds/KeysoundRollSurface.svelte";

  // Keysounds mode's host: the keysound tracks Rust keeps for this map as a piano roll, each
  // optionally played by a scanned VST3 instrument. It covers the hitsound lanes inside #scroll,
  // under the editor's own ruler; the roll and the velocity lane are painted by the editor's paint().
  // The track list sits in the lane column here. The Inspector section, the toolbar pills and the
  // File and Edit menus sit elsewhere: they read bridge.keysounds and act through "keysoundsUi".
  let open = $state(false);
  let loading = $state(false);
  // Replaced whole by every answer from Rust and never written in place, so raw: a track's
  // thousands of notes stay plain objects for the roll's paint to read.
  let summary = $state.raw(null);
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

  // With the status row gone, a failed action says so in a toast.
  function fail(err, title = "Keysounds") {
    bridge.showToast({ tone: "error", title, body: String(err) });
  }

  async function send(payload) {
    if (!payload) return false;
    loading = true;
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
      fail(err);
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
    try {
      const res = await invoke("keysounds_action", { payload: { kind: "browse_dawsuks" } });
      if (res?.picked) {
        summary = res;
        trackId = res.activeTrackId ?? res.tracks?.[0]?.id ?? null;
        selected = null;
        bridge.showToast({ tone: "success", title: "Opened Keysound project", body: res.loadedPath ?? "" });
      }
    } catch (err) {
      fail(err, "Open keysound project");
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

  // Auditions a pitch; it does not go through send() so the buttons don't flicker per key, and a
  // key that cannot sound is not worth a toast each.
  async function preview(pitch, velocity = 100) {
    await invoke("keysounds_action", { payload: keysoundsPreview(trackId, pitch, velocity) }).catch(() => {});
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

  // The instrument picked here is also the drum map's, while the drum map's VST3 switch is on
  // (Preferences › MIDI Input & VST3, Drum Map Setup).
  function setPlugin(id) {
    pluginId = id ?? "";
    drumVst3 = readDrumVst3(storage());
    if (!drumVst3.enabled) return;
    const picked = summary?.vst3Plugins?.find(p => p.id === pluginId) ?? null;
    drumVst3 = writeDrumVst3(storage(), {
      enabled: true,
      pluginId: picked?.id ?? "",
      pluginName: picked?.name ?? "",
      pluginPath: picked?.path ?? "",
    });
  }

  async function addTrack() {
    if (loading) return;
    const plugin = instruments.find(choice => choice.id === pluginId);
    const before = new Set(tracks.map(t => t.id));
    if (await send(keysoundsAdd("", plugin?.name ?? null))) {
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
    try {
      const res = await invoke("keysounds_action", { payload: { kind: "browse_soundfont", trackId } });
      if (res?.picked) {
        summary = res;
        bridge.showToast({ tone: "success", title: "SoundFont loaded", body: res.loadedPath ?? "" });
      }
    } catch (err) {
      fail(err, "Load SFZ / SF2");
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
    try {
      summary = await invoke("keysounds_action", { payload: keysoundsGenerate() });
      const toast = generateToast(summary?.generated);
      if (toast) bridge.showToast(toast);
      bridge.callAction("act", null, true);
    } catch (err) {
      fail(err, "Generate [Keysounds]");
    } finally {
      loading = false;
    }
  }

  async function generateWithBounceOpts(opts) {
    if (loading) return;
    loading = true;
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
      fail(err, "Generate [Keysounds]");
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

  // Leaves the mode first: Import MIDI… writes hitsound lanes, which this mode covers.
  function importMidi() {
    closeModal();
    bridge.openModal("openMidiImport");
  }

  // Every control of this mode that sits outside this part (Inspector section, toolbar pills, File
  // and Edit menus) calls one of these by name through the "keysoundsUi" action.
  const ui = {
    selectTrack,
    mute: id => { if (!loading) send(keysoundsToggleMute(id)); },
    solo: id => { if (!loading) send(keysoundsToggleSolo(id)); },
    rename: (id, name) => { if (!loading) send(keysoundsRenameTrack(id, name)); },
    deleteTrack: id => { if (!loading && id != null) send(keysoundsDelete(id)); },
    addTrack,
    importMidi,
    setPlugin,
    loadVst3,
    scan: () => { if (!loading) send(keysoundsScan()); },
    loadSoundfont,
    selectPreset: selectSoundfontPreset,
    snapToGrid: () => trackAction(id => keysoundsSnapToGrid(id)),
    trim: () => trackAction(id => keysoundsTrimOverlaps(id)),
    strum: () => trackAction(id => keysoundsStrum(id)),
    humanise: () => trackAction(id => keysoundsHumanise(id)),
    syncToObjects,
    transpose: semitones => trackAction(id => keysoundsTranspose(id, semitones)),
    setScaleRoot: root => { scaleRoot = root; },
    setScaleKind: kind => {
      scaleKind = kind;
      if (kind === "chromatic") foldToScale = false;
    },
    toggleFold: () => { if (scaleKind !== "chromatic") foldToScale = !foldToScale; },
    toggleGhosts: () => { showGhosts = !showGhosts; },
    toggleObjectLines: () => { showObjectLines = !showObjectLines; },
    toggleTyping: toggleMusicalTyping,
    openChords: () => { chordPopoverOpen = !chordPopoverOpen; },
    generate,
    openBounce: openBounceModal,
    undo: () => { if (!loading) send({ kind: "undo" }); },
    redo: () => { if (!loading) send({ kind: "redo" }); },
    openProject: openDawsuks,
    saveProject: () => { if (!loading) send({ kind: "save_dawsuks" }); },
  };

  $effect(() => {
    bridge.keysounds = summary ? {
      open,
      loading,
      tracks,
      trackId,
      track,
      notesCount: notes.length,
      anyNotes: tracks.some(t => t.noteCount),
      instruments,
      pluginId,
      sidecarAvailable: Boolean(summary.sidecarAvailable),
      status: keysoundsStatus(summary, track, instruments, NO_PLUGINS, sidecarStatus(summary)),
      chordCount: chordConflicts.length,
      canUndo: Boolean(summary.canUndo),
      canRedo: Boolean(summary.canRedo),
      dirty: Boolean(summary.dirty),
      scaleRoot,
      scaleKind,
      foldToScale,
      showGhosts,
      showObjectLines,
      typing: typingState,
    } : null;
  });

  // The unsaved mark: a dot on the Keysounds mode button, which shows in every mode.
  $effect(() => {
    const dirty = Boolean(summary?.dirty);
    const button = document.getElementById("keysounds-view");
    if (!button) return;
    button.toggleAttribute("data-unsaved", dirty);
    button.title = dirty
      ? "Melodic Keysounding (Piano Roll) · keysound project not saved (File › Save keysound project)"
      : "Melodic Keysounding (Piano Roll)";
  });

  // Another difficulty or map: what was read belongs to the one before.
  let shownPath = null;
  $effect(() => {
    const path = bridge.scene?.path ?? null;
    if (path === shownPath) return;
    const first = shownPath === null;
    shownPath = path;
    if (first) return;
    untrack(() => {
      if (!path) summary = null;
      else if (summary || open) send(keysoundsView());
    });
  });

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
        ui.undo();
        return;
      }
      if (k === "y" || (k === "z" && event.shiftKey)) {
        event.preventDefault();
        event.stopPropagation();
        ui.redo();
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
    const disposeUi = bridge.registerAction("keysoundsUi", (action, ...args) => ui[action]?.(...args));
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
      disposeUi();
      disposeAction();
      disposeRoll();
    };
  });
</script>

<svelte:window onkeydown={onKeydown} onkeyup={onKeyup} onblur={onBlur} />

{#if open}
  <section class="ks-workspace" aria-label="Keysounds and VST3">
      {#snippet trackList()}
        <KeysoundTrackList
          {tracks}
          activeTrackId={trackId}
          {loading}
          onselect={selectTrack}
          onmute={ui.mute}
          onsolo={ui.solo}
          onrename={ui.rename}
          ondelete={ui.deleteTrack}
          onadd={addTrack}
          onimport={importMidi}
        />
      {/snippet}

      {#if !summary}
        <div class="ks-fill"></div>
      {:else if !track}
        <!-- No track yet: the list says what to do next; there is no roll to paint. -->
        <div class="ks-none">
          <div class="ks-none-side">{@render trackList()}</div>
          <div class="ks-fill"></div>
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
          side={trackList}
        />
      {/if}

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
  .ks-none {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  /* As wide as the lane headers it covers, as the roll's own side column is. */
  .ks-none-side {
    flex: 0 0 var(--lanes);
    display: flex;
    min-width: 0;
    overflow: hidden;
    border-right: 1px solid var(--editor-border);
    box-sizing: border-box;
  }
</style>
