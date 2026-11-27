import {
  actions, hasOpenModal, modals, onlyModalOpen, openModal, registerAction, registerTimelineTheme, showToast,
} from "../lib/registry.js";
// Main editor runtime entry point: composes the modular editor subsystems, wires top-level
// controls, keyboard and Tauri listeners, and registers runtime actions on the bridge registry.
import { attachBrowser } from "./browser.js";
import { attachCanvas } from "./canvas.js";
import { attachCopyDialogs } from "./dialogs-copy.js";
import { attachEditDialogs } from "./dialogs-edit.js";
import { attachSettingsDialogs } from "./dialogs-settings.js";
import { createDom } from "./dom.js";
import {
  defaultWaveformHeight, keyInput, laneGrey, levelColours, limit, minZoom, nameOf,
  objectReach, objectSize, playbackStartChoices, rulerBandHeight, sampleFolderNames,
  shortcutName, slateTimeline, visibleWarningMarkers, volumeColour, volumePalette,
} from "./helpers.js";
import { attachInspector } from "./inspector.js";
import { attachIpc } from "./ipc.js";
import { attachLanes } from "./lanes.js";
import { attachModal } from "./modal.js";
import { attachPanels } from "./panels.js";
import { createState } from "./state.js";
import { attachTimeline } from "./timeline.js";
import { createQuasimodeController, toolKeyFor } from "../lib/quasimode.js";
import { PREVIEW_SKIN_KEY, readPreviewSkin, timelineSkinSource } from "../lib/previewSkin.js";
import { resolveVisibleTapRow, tapLaneSlotForKey } from "../lib/tapRecord.js";
import { attachGlobalWheelGuard } from "../lib/wheelGuard.js";

const mutableStateFields = [
  "view", "scene", "drag", "rulerDrag", "overviewDrag", "overviewTarget", "overviewNavigation",
  "waveformHeight", "visibleWaveformHeight", "waveformShown", "soundFirst", "waveformDrag", "markedRange",
  "browserMap", "browserFolder", "browserSample", "browserSource", "browserLibrary",
  "browserLibraryFolders", "browserScene", "browserState", "browserFilter", "browserPreview",
  "browserModel", "browserTree", "usedLaneKeys", "laneFilterText", "showLaneInfo", "laneRows", "lanePositions",
  "laneVisualPositions", "drumRowMembers", "drumRowByMember", "selectionLaneAnchor", "selectionLanesOnly", "familyParents",
  "familyLayers", "laneRemoving", "previewParents", "orderedLaneKeys", "sampleFolderOrder",
  "layoutWrites", "mixerStrips", "mixerKey", "laneHeaders", "bookmarkPick", "selectedHitsScene",
  "selectedHits", "inspectorState", "problemStripKept", "sectionBarCache", "sliderEndCache",
  "overviewBinCache", "gameplayIndexCache", "staticLayers", "layerCopies", "lastLayerKey", "skinTextures", "cursorSurface", "cursorAltDraw", "hoveredHit", "lastHover", "rulerPointer",
  "modalKind", "modalClose", "outputProblemIndex", "outputProblemList", "rulerWarningMarkers", "rulerChordMarks",
  "pending", "requests", "refreshing", "lastRefresh", "frame",
  "playbackFrame", "playheadAnchor", "centreSentStart", "followQueued", "quitAfterSave",
];

const readonlyStateFields = [
  "browserExpanded", "selectionLaneKeys", "collapsedFamilies", "laneColours",
  "renamedLanes", "collapsedFolders", "inspectorLists",
];

const exportedMethods = [
  "invoke", "setStatusNotice", "run", "update", "act", "command", "toggle",
  "missingUiParts", "renderFrame", "show",
  "dockState", "setDockOverride", "panelPrefs", "effectivePanelState",
  "panelOn", "togglePanel", "openPanelAt", "renderPanelLayout",
  "hydrateLaneLayout", "saveLaneLayout", "reorderSampleFolder", "folderHeaderModel",
  "laneDisplayName", "laneSound", "laneTitle", "savedLaneRows", "orderedLaneRows",
  "familyMains", "familyOrder", "adjacentLaneForMove", "moveLane", "renameLane",
  "refreshLanePresentation", "groupedLaneRows", "renderLanes", "currentMixer",
  "mixerModel", "currentLaneHeaders", "laneHeaderModel", "laneHeaderAction",
  "mixSelectedLanes", "volumeScopeHint", "updateVolumeScope", "selectVolumeLane",
  "requestDrumMap", "setDrumView", "drumMapAction", "openDrumMapSetup", "deletableLane", "openAddLane", "deleteActiveLane",
  "laneTint", "setLaneHeight",
  "followBookmarkPick", "selectedBookmark", "renderActiveLane", "selectedHitDetails",
  "renderSelection", "handOverInspector", "inspectorLaneNames", "currentInspector",
  "keptList", "problemStripData", "inspectorModel", "inspectorAction", "stepCoverageIssue",
  "sampleTarget", "pickSample", "mapsetSamples", "requestLibraryFolder", "resetBrowserFolder",
  "mapsetTree", "renderSampleBrowser", "currentBrowser", "samplePreviewAction",
  "requestSamplePreview", "sampleBrowserAction",
  "prepare", "line", "sectionBars", "drawTimingGrid", "waveform", "paintSoon",
  "anchorPlayhead", "playheadPosition", "animatePlayback",
  "fillHitDiamond", "strokeHitDiamond", "comboOf", "roundedTrack", "nodeRadii",
  "drawGameplay", "overviewBins", "diamond", "drawOverview", "drawLocatorFlags",
  "drawBookmarks", "paint", "paintFrame",
  "renderScene", "nearestHitTime", "pointer", "revealLayerPreview", "scrollGrab", "refreshToolCursor",
  "updateCursorModifiers", "handOverHitCard", "hitObjectName", "refreshHitCard",
  "hitCardAt", "refreshWaveformSwitch", "setWaveformHeight", "finishWaveformDrag",
  "refreshRulerCursor", "trackRulerPointer", "refreshRulerWarningTooltip",
  "waveformTarget", "bookmarkNearFlag", "locatorFlags", "rulerGestureAt",
  "locatorPreview", "locatorDragTo", "endRulerDrag", "drainOverviewNavigation",
  "panOverview", "timelineStart", "followPlayhead", "queueFollow", "resizeLaneHeaders",
  "refreshLaneToolbar", "changeView", "stepWheelSnap", "fitTimeline",
  "modal", "closeModal", "confirm", "showOutputProblem", "renderDialogs",
  "openMergeSettings", "openGenerate", "openNewDifficulty", "openCopy",
  "openLaneColour", "openReplaceSample", "openMap", "openMapFolder", "openMapFolderPath",
  "createAndOpenHitsoundDiff", "pickAndCreateHitsoundDiff", "openFromOsu", "pickDrumStem", "goHome", "confirmRevertToOriginal", "undoMapsetRevert",
  "openSiblingDifficulty", "chooseSiblingDifficulty", "showTimelineEdit",
  "editBookmark", "openTimeSignatures",
  "openFolders", "openAudioOutput", "openViewSettings", "openPreferences", "showHelp", "showShortcutEditor",
  "clampSnapInput", "popoverAction", "startScreenAction", "statusBarAction",
  "setQuasimodeShortcuts", "notifyStrokeStart", "notifyStrokeEnd", "toolKey", "loadTimelineSkin",
  "startTelemetry", "parseTelemetryFrame",
];

export function createEditor(env = globalThis, { ui = null, autoStart = true, exposeGlobals = true } = {}) {
  const dom = createDom(env);
  const { $, makeButton, paragraph, input, labeled, setValue, number } = dom;
  const state = createState();
  const elements = {
    grid: $("grid"),
    controller: $("controller"),
    ruler: $("ruler"),
    overview: $("overview"),
    dialog: $("dialog"),
    speedButtons: [...env.document.querySelectorAll("[data-playback-speed]")],
  };

  const dock = env.window.editorDocking?.attach($("editor-dock"));
  const ctx = {
    env,
    state,
    dom,
    elements,
    dock,
    labeled,
    playbackStartChoices,
    ui: () => ui ?? env.window?.dawsuUi,
  };
  // The window crossing 1180 px changes what the panel switches show.
  if (dock) dock.onChange = () => ctx.renderPanelLayout();

  attachIpc(ctx);
  attachPanels(ctx);
  attachLanes(ctx);
  attachInspector(ctx);
  attachBrowser(ctx);
  attachCanvas(ctx);
  attachTimeline(ctx);
  attachModal(ctx);
  attachCopyDialogs(ctx);
  attachEditDialogs(ctx);
  attachSettingsDialogs(ctx);

  const { invoke } = env.window.__TAURI__.core;

  // Bind all module-specific DOM listeners.
  ctx.bindPanels();
  ctx.bindLanes();
  ctx.bindBrowser();
  ctx.bindTimeline();
  ctx.bindModal();
  attachGlobalWheelGuard(env.document);

  const openMoreToolsPicker = () => {
    const trigger = env.document.querySelector?.("[data-more-tools]");
    const openMenu = env !== globalThis ? env.window?.openEditorMenu : actions.openEditorMenu;
    if (trigger && typeof openMenu === "function") {
      const box = trigger.getBoundingClientRect();
      openMenu("tool", false, { x: box.left, y: box.bottom, trigger, label: "Timeline tools" });
    }
  };
  const quasimode = createQuasimodeController({
    getTool: () => state.view?.tool ?? "Select",
    setTool: (name) => {
      if (state.scene && !state.view?.busy && state.view?.tool !== name) {
        ctx.act({ kind: "tool", name }, false);
      }
    },
    isDragging: () => Boolean(state.drag || state.rulerDrag),
    openMoreTools: openMoreToolsPicker,
  });
  // The key that picks `tool` in the current preset and shortcuts, for hints that name it.
  ctx.toolKey = tool => toolKeyFor(quasimode.getKeyMap(), tool);
  // Decodes a skin payload from preview_skin_load into the images the objects row draws, or null
  // when its hitcircle or overlay will not load.
  async function decodeSkinTextures(payload, ImageCtor) {
    if (!payload?.hitcircle || !payload?.overlay) return null;
    const loadImg = (src) => new Promise((resolve) => {
      if (!src) { resolve(null); return; }
      const img = new ImageCtor();
      img.onload = () => resolve(img);
      img.onerror = () => resolve(null);
      img.src = src;
    });
    const [hitcircle, overlay, reverseArrow, digits] = await Promise.all([
      loadImg(payload.hitcircle),
      loadImg(payload.overlay),
      loadImg(payload.reverseArrow),
      Promise.all((payload.digits ?? []).map(loadImg)),
    ]);
    if (!hitcircle || !overlay) return null;
    let circleFill = typeof payload.circleFill === "number" && payload.circleFill > 0 ? payload.circleFill : 0.92;
    try {
      const probe = env.document.createElement("canvas");
      const w = hitcircle.naturalWidth || hitcircle.width;
      const h = hitcircle.naturalHeight || hitcircle.height;
      if (w > 0 && h > 0) {
        probe.width = w;
        probe.height = h;
        const pctx = probe.getContext("2d");
        if (pctx?.getImageData) {
          pctx.drawImage(hitcircle, 0, 0);
          const row = pctx.getImageData(0, Math.floor(h / 2), w, 1).data;
          let first = -1, last = -1;
          for (let x = 0; x < w; x++) {
            if (row[x * 4 + 3] > 128) {
              if (first < 0) first = x;
              last = x;
            }
          }
          if (first >= 0) circleFill = (last - first + 1) / w;
        }
      }
    } catch {
      // Keep default circleFill when canvas pixel readback is unavailable.
    }
    return { ready: true, hitcircle, overlay, reverseArrow, digits, circleFill, tinted: new Map() };
  }

  // Skins the objects row with the map preview's pick (lib/previewSkin.js): a skin folder as
  // picked, lazer Classic for anything else or when the folder will not load, plain circles when
  // neither does. A load that a newer pick overtook is dropped.
  let skinLoads = 0;
  ctx.loadTimelineSkin = async (skinId) => {
    const ImageCtor = env.Image ?? env.window?.Image;
    if (typeof ImageCtor !== "function") return null;
    const load = ++skinLoads;
    const fetchSkin = (source) => invoke("preview_skin_load", { skinId: source })
      .then(payload => decodeSkinTextures(payload, ImageCtor))
      .catch(() => null);
    const source = timelineSkinSource(skinId);
    let skin = await fetchSkin(source);
    if (!skin && source !== "classic" && load === skinLoads) skin = await fetchSkin("classic");
    if (load !== skinLoads) return null;
    state.skinTextures = skin;
    ctx.paintSoon();
    return skin;
  };
  ctx.notifyStrokeStart = () => quasimode.notifyStrokeStart();
  ctx.notifyStrokeEnd = () => quasimode.notifyStrokeEnd();
  ctx.setQuasimodeShortcuts = (shortcuts) => quasimode.setShortcuts(shortcuts);
  ctx.setQuasimodePreset = (preset) => quasimode.setPreset(preset);

  function clampSnapInput(value, fallback) {
    const numeric = typeof value === "string" ? (value.trim() === "" ? NaN : Number(value)) : Number(value);
    return Number.isFinite(numeric) ? limit(Math.trunc(numeric), 1, 64) : fallback;
  }

  function popoverAction(kind, ...args) {
    switch (kind) {
      case "objectLines":
        return ctx.act({ kind: "object_lines", on: !!args[0] });
      case "snap": {
        if (!state.scene) return;
        const patch = args[0] ?? {};
        return ctx.act({
          kind: "snap",
          on: patch.on ?? state.scene.snap,
          mode: patch.mode ?? state.scene.snapMode,
          beats: clampSnapInput(patch.beats, state.scene.beats),
          divisor: clampSnapInput(patch.divisor, state.scene.divisor),
        });
      }
      case "tripletSnap":
        if (state.scene) return ctx.act({ kind: "snap", on: state.scene.snap, mode: state.scene.snapMode, beats: 1, divisor: args[0] ? 6 : 4 });
        return;
      case "timeSignatures":
        return ctx.openTimeSignatures();
      case "laneHeightStep":
        if (state.scene) return ctx.setLaneHeight(Math.round(state.scene.rowHeight * args[0]));
        return;
      case "laneHeight": {
        const [height, live] = args;
        if (!live || state.requests < 2) return ctx.setLaneHeight(height);
        return;
      }
      case "collapseLayers":
        if (!state.scene || state.view.busy || state.drag) return;
        for (const family of state.familyParents.values()) state.collapsedFamilies.add(family);
        ctx.renderLanes();
        ctx.paintSoon();
        ctx.saveLaneLayout();
        return;
      case "laneFilter":
        state.laneFilterText = String(args[0] ?? "");
        ctx.ui()?.laneOptions?.(state.usedLaneKeys !== null, state.laneFilterText);
        if (!state.scene || state.drag) return;
        ctx.renderLanes();
        ctx.paintSoon();
        return;
      case "usedOnly": {
        const on = !!args[0];
        if (!state.scene || state.view.busy || state.drag) {
          ctx.ui()?.laneOptions?.(state.usedLaneKeys !== null, state.laneFilterText);
          return;
        }
        if (on) {
          // Snapshot used lanes so a row does not disappear when its last hit is moved or erased.
          state.usedLaneKeys = new Set();
          for (const [, , , hits] of state.scene.moments) {
            for (const [row] of hits) state.usedLaneKeys.add(state.scene.lanes[row].key);
          }
        } else {
          state.usedLaneKeys = null;
        }
        ctx.ui()?.laneOptions?.(state.usedLaneKeys !== null, state.laneFilterText, state.groupBySampleSet !== false);
        ctx.renderLanes();
        ctx.paintSoon();
        return;
      }
      case "groupBySampleSet": {
        state.groupBySampleSet = !!args[0];
        ctx.ui()?.laneOptions?.(state.usedLaneKeys !== null, state.laneFilterText, state.groupBySampleSet);
        if (!state.scene || state.drag) return;
        ctx.renderLanes();
        ctx.paintSoon();
        ctx.saveLaneLayout?.(true);
        return;
      }
    }
  }

  function startScreenAction(kind, arg, extra) {
    switch (kind) {
      case "resume":
        if (!state.view.busy) return ctx.act({ kind: "resume" });
        return;
      case "open":
        return ctx.openMap();
      case "openFolder":
        return ctx.openMapFolder();
      case "createHsDiff":
        return arg
          ? ctx.createAndOpenHitsoundDiff(arg, extra ? false : true, extra || null)
          : ctx.pickAndCreateHitsoundDiff();
      case "openMapsetIntake":
        return ctx.ui()?.openMapsetIntake?.(arg);
      case "openFromOsu":
        return ctx.openFromOsu();
      case "importOsz":
        return ctx.run(async () => ctx.show(await invoke("pick_map", { archiveOnly: true })));
      case "folders":
        return ctx.openFolders();
      case "audio":
        return ctx.openPreferences("audio");
      case "shortcuts":
        return ctx.openPreferences("shortcuts");
      case "tryPractice":
        return ctx.act({ kind: "open_practice_mapset" });
      case "openRecent":
      case "openPath":
        return ctx.act({ kind: "open", path: arg });
      case "restoreRecovery":
        return ctx.act({ kind: "restore_recovery", id: arg });
    }
  }

  function statusBarAction(kind, ...args) {
    switch (kind) {
      case "speed":
        return ctx.act({ kind: "speed", value: args[0] });
      case "keepPitch":
        return ctx.act({ kind: "keep_pitch", on: Boolean(args[0]) });
      case "stepPadsMode":
        return ctx.act({ kind: "step_pads_mode", on: Boolean(args[0]) });
      case "nextOutputProblem":
        return ctx.showOutputProblem(state.outputProblemIndex + 1);
      case "warnings":
        ctx.modal("warnings", "Audio warnings", state.view.warnings.map(paragraph), [["Close", () => ctx.closeModal(), true]]);
        return;
      case "cancelJob":
        return ctx.act({ kind: "cancel_job" });
      case "reloadFromDisk":
        return ctx.act({ kind: "reload_from_disk" });
      case "dismissExternalOsuChange":
        return ctx.act({ kind: "dismiss_external_osu_change" });
      case "fit":
        return ctx.fitTimeline();
      case "zoom": {
        const [logZoom, live] = args;
        if (!state.scene) return;
        if (!live || state.requests < 2) return ctx.changeView(state.scene.viewStart, 10 ** logZoom);
        return;
      }
      case "audio":
        return ctx.openAudioOutput();
    }
  }

  Object.assign(ctx, { clampSnapInput, popoverAction, startScreenAction, statusBarAction });

  const notifyToast = toast => (env !== globalThis ? env.window?.showToast?.(toast) : showToast(toast));
  const otherModalOpen = () =>
    Boolean(env.window?.dawsuPreferencesOpen || env.window?.dawsuModalOpen || env.window?.dawsuCommandPaletteOpen);
  // Keysounds mode owns the keyboard like a modal, except for its plain tool keys (keysoundsToolKey).
  const keysoundsMode = () => ctx.ui()?.mode?.() === "keysounds";
  const modalOpen = () => (env === globalThis && hasOpenModal()) || otherModalOpen() || keysoundsMode();
  // Plain 1-4 in Keysounds mode with nothing else open: the tool quasimode (plan 3.5, task 5.2).
  // Every other key stays with the Keysounds host, so no hit is tapped, deleted or edited.
  // Numpad digits are tap-lane keys, never tool keys, so they stay blocked here too.
  const keysoundsToolKey = (event, key) => keysoundsMode() && /^[1-4]$/.test(key) &&
    !String(event.code ?? "").startsWith("Numpad") && !otherModalOpen() &&
    (env !== globalThis || onlyModalOpen("keysounds")) && !event.ctrlKey && !event.metaKey && !event.shiftKey &&
    !event.altKey && $("editor-menu")?.hidden !== false;

  // Bind remaining top-level controls.
  $("ramp-shape").addEventListener("change", () => {
    ctx.act({ kind: "ramp_shape", value: $("ramp-shape").value }, false);
  });
  for (const button of env.document.querySelectorAll("[data-action]")) {
    button.addEventListener("click", () => ctx.act({ kind: button.dataset.action }));
  }
  $("difficulty").addEventListener("change", async () => {
    if (!state.scene) return;
    const value = $("difficulty").value;
    $("difficulty").value = state.scene.path;
    if (value === "__toggle_hs__") {
      await ctx.act({ kind: "hs_mode", on: !state.scene.hs });
      return;
    }
    if (value === "__create_hs__") {
      await ctx.createAndOpenHitsoundDiff(state.scene.path, false);
      return;
    }
    if (value === "__create_hs_from_current__") {
      try {
        const created = await invoke("create_hitsound_diff_from_current");
        if (created) await ctx.act({ kind: "open", path: created });
      } catch (error) {
        notifyToast({ tone: "error", title: "Could not create [Hitsounds] diff", body: String(error) });
      }
      return;
    }
    if (value === "__capture_shared_base__") {
      try {
        await invoke("shared_base_action", { action: { kind: "capture" } });
        notifyToast({ tone: "success", title: "Shared Base captured", body: "Captured hitsounds from this difficulty as the mapset's Shared Base." });
        if (env !== globalThis) env.window?.openSharedBaseModal?.();
        else openModal("openSharedBase");
      } catch (error) {
        notifyToast({ tone: "error", title: "Could not capture Shared Base", body: String(error) });
      }
      return;
    }
    ctx.openSiblingDifficulty(value);
  });
  // Save writes at once through the editor's save command, as Ctrl+S does, instead of review_save's
  // file-by-file review first. Moments Save can't write still stop it with the Save hitsounds summary.
  $("save").addEventListener("click", () => {
    if (state.scene && !state.view.busy) ctx.command("save");
  });
  // Export opens inside the editor as a modal dialog (falling back to the standalone window if not mounted).
  $("export-entry").addEventListener("click", () => {
    const opener = env !== globalThis ? env.window?.openExportModal : modals.openExportModal;
    if (typeof opener === "function") opener();
    else invoke("open_export_window").catch((error) => { ctx.setStatusNotice(String(error)); });
  });
  $("song-mute").addEventListener("click", () => ctx.act({ kind: "song_muted", on: !state.view.songMuted }, false));
  $("song-solo").addEventListener("click", () => ctx.act({ kind: "song_solo", on: !state.view.songSolo }, false));
  ctx.pickDrumStem = () => ctx.run(async () => ctx.show(await invoke("pick_drum_stem")));
  const songStem = $("song-stem");
  if (songStem) {
    songStem.addEventListener("click", () => {
      if (!state.scene || state.view.busy || state.view.stemLoading) return;
      if (state.view.stemLoaded) ctx.act({ kind: "toggle_stem_monitor" }, false);
      else ctx.pickDrumStem();
    });
    songStem.addEventListener("contextmenu", (event) => {
      if (!state.scene || state.view.busy || !state.view.stemLoaded) return;
      event.preventDefault();
      ctx.act({ kind: "clear_drum_stem" }, false);
    });
  }

  // A select, checkbox or slider keeps focus after a click, and the keyboard handler below ignores
  // keys aimed at it: let go of it once used, so Space and Ctrl+Z reach the timeline again.
  // Dialogs and Preferences keep their focus for keyboard users.
  const releasable = "select, input[type=checkbox], input[type=range]";
  const releaseControl = (event) => {
    const control = event.target?.closest?.(releasable);
    if (!control || control.closest("dialog, [role=dialog]")) return;
    // A select lets go once picked (its pointerup opens its list), a slider once let go of (a change
    // also comes from each arrow key), a checkbox either way (a click on its label sends no pointerup to it).
    const kind = control.tagName === "SELECT" ? "select" : control.type;
    if (event.type === "change" ? kind === "range" : kind === "select") return;
    control.blur?.();
  };
  env.document.addEventListener("change", releaseControl);
  env.document.addEventListener("pointerup", releaseControl);

  // Global keyboard handler.
  env.document.addEventListener("keydown", (event) => {
    if (event.defaultPrevented || event.isComposing || elements.dialog.open ||
        event.target.closest("input, select, textarea, [contenteditable]")) return;
    if (keysoundsToolKey(event, event.key)) {
      if (state.scene && !state.drag && !state.rulerDrag &&
          quasimode.handleKeyDown({ key: event.key, repeat: Boolean(event.repeat) })) {
        event.preventDefault();
      }
      return;
    }
    // An open menu, Preferences or a Svelte modal (MIDI import, Keysounds, Command Palette) owns the keyboard:
    // no timeline shortcut runs behind it.
    if ($("editor-menu")?.hidden === false || modalOpen()) {
      quasimode.cancel(true);
      return;
    }
    const { key, code, ctrl, shift, alt } = keyInput(event);
    if (event.repeat) {
      if (state.scene && !state.drag && !state.rulerDrag && quasimode.handleKeyDown({ key, ctrl, shift, alt, repeat: true })) {
        event.preventDefault();
      }
      return;
    }
    if (state.rulerDrag) {
      if (key === "Escape" && !ctrl && !shift && !alt) {
        event.preventDefault();
        ctx.endRulerDrag(event);
        quasimode.cancel(true);
      }
      return;
    }
    if (key === "Escape" && state.drag && !ctrl && !shift && !alt) {
      event.preventDefault();
      const { surface, firstPoint, lastPoint, id } = state.drag;
      state.drag = null;
      if (state.drawStrokePreview) {
        state.drawStrokePreview = null;
        ctx.paintSoon();
      }
      if (surface.hasPointerCapture(id)) surface.releasePointerCapture(id);
      ctx.act({ ...(lastPoint ?? firstPoint), phase: "cancel" });
      ctx.notifyStrokeEnd?.();
      quasimode.cancel(true);
      return;
    }
    if (key === "Escape" && state.view.showHome && state.view.hasDocument && !state.view.busy && !ctrl && !shift && !alt) {
      event.preventDefault();
      ctx.act({ kind: "resume" });
      return;
    }
    const activatesButton = !ctrl && !shift && !alt &&
      ((key === " " && !state.scene) || (key === "Enter" && code !== "NumpadEnter"));
    if (state.view.busy || (activatesButton && event.target.closest("button"))) return;
    // Spacebar while a map is open must ONLY trigger Play/Pause, never activate a focused toolbar button.
    if (key === " " && !ctrl && !shift && !alt && state.scene && event.target.closest("button")) {
      event.target.closest("button").blur?.();
    }
    // Live keyboard lane tapping (F028/F029): Numpad 1–9 anytime, or A–L while tap recording is armed.
    if (state.scene && !state.view.busy) {
      const slot = tapLaneSlotForKey({ code, ctrl, shift, alt }, Boolean(state.view.tapRecordingArmed));
      if (slot !== null) {
        const row = resolveVisibleTapRow(state.laneRows, slot);
        if (row !== null) {
          event.preventDefault();
          const rowEl = env.document.querySelector?.(`[data-lane-row="${row}"]`);
          if (rowEl?.classList) {
            rowEl.classList.add("tap-flash");
            setTimeout(() => rowEl.classList.remove("tap-flash"), 140);
          }
          ctx.act({ kind: "tap_lane", row }, false);
          return;
        }
      }
    }
    // Quasimodes: tap (< 180 ms) switches tools or opens More Tools on "4"; hold reverts on release.
    if (state.scene && !state.drag && !state.rulerDrag &&
        quasimode.handleKeyDown({ key, ctrl, shift, alt, repeat: false })) {
      event.preventDefault();
      return;
    }
    // N / Shift+N steps to the next / previous coverage issue in the active Inspector filter.
    if ((key === "n" || key === "N") && !ctrl && !alt && state.scene) {
      event.preventDefault();
      ctx.stepCoverageIssue(shift ? -1 : 1);
      return;
    }
    if (key === "Tab") {
      if (!ctrl && !alt && event.target.closest("[data-dock-control]")) return;
      const bound = state.view.tabRoutes?.some(([control, shifted, alternate, global]) =>
        control === ctrl && shifted === shift && alternate === alt && (state.scene || global));
      if (!bound) return;
    } else if (key.length !== 1 &&
        !/^(Arrow(Left|Right|Up|Down)|Delete|Backspace|Escape|Home|End|PageUp|PageDown|Insert|Enter|F([1-9]|1[0-2]))$/.test(key) &&
        !code.startsWith("Numpad")) return;

    event.preventDefault();
    const action = { kind: "key", key, code, ctrl, shift, alt };
    if (state.scene?.path && elements.grid.clientWidth > 0 && state.scene.zoom > 0) {
      action.lane_view = {
        path: state.scene.path,
        rows: [...new Set(state.laneRows.filter(Number.isInteger))],
        span: elements.grid.clientWidth / state.scene.zoom,
      };
    }
    ctx.act(action);
  });

  env.window.addEventListener?.("keyup", (event) => {
    ctx.updateCursorModifiers?.(event);
    const { key } = keyInput(event);
    quasimode.handleKeyUp({ key });
  });
  env.window.addEventListener?.("blur", () => {
    state.cursorAltDraw = false;
    ctx.refreshToolCursor?.();
    quasimode.cancel(true);
  });
  env.document.addEventListener?.("visibilitychange", () => {
    if (env.document.hidden) quasimode.cancel(true);
  });

  // Native Tauri window events.
  env.window.__TAURI__.event.listen("tauri://drag-drop", async ({ payload }) => {
    const map = payload.paths.find((path) => /\.(osu|osz)$/i.test(path));
    if (map) {
      ctx.act({ kind: "open", path: map });
      return;
    }
    const audioPaths = payload.paths.filter((path) => /\.(wav|ogg|mp3)$/i.test(path));
    if (audioPaths.length && state.scene?.path) {
      const zeroLanes = (state.scene.laneOrder ?? state.scene.lanes ?? []).length === 0;
      const target = ctx.sampleTarget();
      if (!zeroLanes && audioPaths.length === 1 && target) {
        ctx.act({ kind: "replace_sample", target, path: audioPaths[0], mapset: false });
        return;
      }
      const mapPath = state.scene.path;
      for (const audioPath of audioPaths) {
        await ctx.act({ kind: "add_lane_from_file", map: mapPath, path: audioPath });
      }
      return;
    }
    if (payload.paths.length && !audioPaths.length) {
      ctx.openMapFolderPath?.(payload.paths[0]);
    }
  });

  env.window.__TAURI__.event.listen("node-graph-viewer", ({ payload }) => {
    ctx.ui()?.setNodeGraphViewer?.(payload ?? null);
    ctx.paintSoon();
  });

  // Space in the map preview window plays and stops here, through the same key route as the editor's own.
  env.window.__TAURI__.event.listen("preview-space", () => {
    if (!state.scene || state.view.busy || elements.dialog.open || $("editor-menu")?.hidden === false || modalOpen()) return;
    ctx.act({ kind: "key", key: " ", code: "Space", ctrl: false, shift: false, alt: false });
  });

  env.window.__TAURI__.event.listen("node-graph-applied", () => {
    ctx.update(null, true);
  });

  env.window.__TAURI__.event.listen("editor-close-requested", ({ payload }) => {
    if (state.view.fault) return;
    const cancelClose = () => {
      ctx.closeModal();
      invoke("cancel_staged_update").catch(() => {});
    };
    if (state.view.busy) {
      ctx.modal("close-busy", "Job in progress", [paragraph(payload)], [["Keep editor open", cancelClose, true]]);
      return;
    }
    ctx.modal("close-editor", "Quit dawsu!?", [paragraph(payload)], [
      ["Cancel", cancelClose],
      ["Discard changes", () => ctx.run(() => invoke("close_editor", { discard: true }))],
      ["Save and quit", async () => {
        ctx.closeModal();
        await ctx.act({ kind: "save" });
        if (state.view.busy) state.quitAfterSave = true;
        else if (!state.view.dirty) ctx.run(() => invoke("close_editor", { discard: false }));
        else invoke("cancel_staged_update").catch(() => {});
      }, true],
    ]);
  });

  // Periodic background refresh for playback & job progress.
  const perf = () => env.performance ?? performance;
  const startInterval = env.setInterval ?? setInterval;
  startInterval(async () => {
    const now = perf().now();
    // Skip 85 ms fallback polling while binary push telemetry channel is actively delivering frames.
    if (state.telemetryConnected && now - state.lastTelemetryAt < 2000 && !state.quitAfterSave) return;
    // While playing or busy, every tick asks; a gate at the timer's own 85 ms skipped every other tick.
    if (state.view.fault || state.requests || state.refreshing || (!state.view.playing && !state.view.busy && !state.view.songLoading && !state.view.stemLoading && now - state.lastRefresh < 1000)) return;
    state.refreshing = true;
    state.lastRefresh = now;
    await ctx.run(() => ctx.update(null, false));
    state.refreshing = false;
    // Save and quit waits while the Save hitsounds summary is open; its Save then quits.
    if (state.quitAfterSave && !state.view.busy && !state.view.saveProblemReview) {
      state.quitAfterSave = false;
      if (!state.view.dirty && !state.view.error) ctx.run(() => invoke("close_editor", { discard: false }));
    }
  }, 85);

  if (env === globalThis) {
    registerTimelineTheme(slateTimeline);
    for (const name of exportedMethods) {
      registerAction(name, (...args) => ctx[name]?.(...args));
    }
  }

  if (exposeGlobals) {
    const descriptors = {
      $: { value: $, writable: true, configurable: true },
      makeButton: { value: makeButton, writable: true, configurable: true },
      paragraph: { value: paragraph, writable: true, configurable: true },
      input: { value: input, writable: true, configurable: true },
      labeled: { get: () => ctx.labeled, set: (fn) => { ctx.labeled = fn; }, configurable: true },
      setValue: { value: setValue, writable: true, configurable: true },
      number: { value: number, writable: true, configurable: true },
      grid: { value: elements.grid, writable: true, configurable: true },
      controller: { value: elements.controller, writable: true, configurable: true },
      ruler: { value: elements.ruler, writable: true, configurable: true },
      overview: { value: elements.overview, writable: true, configurable: true },
      dialog: { value: elements.dialog, writable: true, configurable: true },
      speedButtons: { value: elements.speedButtons, writable: true, configurable: true },
      dock: { value: dock, writable: true, configurable: true },
      playbackStartChoices: { value: playbackStartChoices, writable: true, configurable: true },
      sampleFolderNames: { value: sampleFolderNames, writable: true, configurable: true },
      defaultWaveformHeight: { value: defaultWaveformHeight, writable: true, configurable: true },
      rulerBandHeight: { value: rulerBandHeight, writable: true, configurable: true },
      minZoom: { value: minZoom, writable: true, configurable: true },
      laneGrey: { value: laneGrey, writable: true, configurable: true },
      levelColours: { value: levelColours, writable: true, configurable: true },
      volumePalette: { value: volumePalette, writable: true, configurable: true },
      volumeColour: { value: volumeColour, writable: true, configurable: true },
      slateTimeline: { value: slateTimeline, writable: true, configurable: true },
      objectSize: { value: objectSize, writable: true, configurable: true },
      objectReach: { value: objectReach, writable: true, configurable: true },
      nameOf: { value: nameOf, writable: true, configurable: true },
      limit: { value: limit, writable: true, configurable: true },
      shortcutName: { value: shortcutName, writable: true, configurable: true },
      keyInput: { value: keyInput, writable: true, configurable: true },
      visibleWarningMarkers: { value: visibleWarningMarkers, writable: true, configurable: true },
    };
    for (const field of mutableStateFields) {
      descriptors[field] = {
        get: () => state[field],
        set: (value) => { state[field] = value; },
        configurable: true,
      };
    }
    for (const field of readonlyStateFields) {
      descriptors[field] = {
        get: () => state[field],
        configurable: true,
      };
    }
    for (const name of exportedMethods) {
      descriptors[name] = {
        get: () => ctx[name],
        set: (fn) => { ctx[name] = fn; },
        configurable: true,
      };
    }
    Object.defineProperties(env, descriptors);
  } else if (env === globalThis) {
    Object.defineProperties(env, {
      scene: { get: () => state.scene, set: (value) => { state.scene = value; }, configurable: true },
      view: { get: () => state.view, set: (value) => { state.view = value; }, configurable: true },
      pending: { get: () => state.pending, set: (value) => { state.pending = value; }, configurable: true },
      act: { get: () => ctx.act, set: (fn) => { ctx.act = fn; }, configurable: true },
    });
  }

  if (autoStart) {
    ctx.act(null).then(() => {
      ctx.startTelemetry?.();
    });
    // The objects row's skin: the one picked for the map preview, then any pick made in the
    // preview window (a storage event; this window's own picks call loadTimelineSkin directly).
    ctx.loadTimelineSkin(readPreviewSkin(env.window?.localStorage));
    env.window?.addEventListener?.("storage", (event) => {
      if (event.key === PREVIEW_SKIN_KEY) ctx.loadTimelineSkin(event.newValue || "argon");
    });
  }
  return ctx;
}
