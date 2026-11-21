// Main editor runtime entry point: composes the modular editor subsystems, wires top-level
// controls, keyboard and Tauri listeners, and exposes the globals that menus.js and Svelte parts call.
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

const mutableStateFields = [
  "view", "scene", "drag", "rulerDrag", "overviewDrag", "overviewTarget", "overviewNavigation",
  "waveformHeight", "visibleWaveformHeight", "waveformShown", "waveformDrag", "markedRange",
  "browserMap", "browserFolder", "browserSample", "browserSource", "browserLibrary",
  "browserLibraryFolders", "browserScene", "browserState", "browserFilter", "browserPreview",
  "browserModel", "browserTree", "usedLaneKeys", "showLaneInfo", "laneRows", "lanePositions",
  "laneVisualPositions", "selectionLaneAnchor", "selectionLanesOnly", "familyParents",
  "familyLayers", "laneRemoving", "previewParents", "orderedLaneKeys", "sampleFolderOrder",
  "layoutWrites", "mixerStrips", "mixerKey", "laneHeaders", "bookmarkPick", "selectedHitsScene",
  "selectedHits", "inspectorState", "problemStripKept", "sectionBarCache", "sliderEndCache",
  "overviewBinCache", "cursorSurface", "cursorAltDraw", "hoveredHit", "lastHover", "rulerPointer",
  "modalKind", "modalClose", "outputProblemIndex", "outputProblemList", "rulerWarningMarkers",
  "pending", "requests", "refreshing", "lastRefresh", "frame", "quitAfterSave",
];

const readonlyStateFields = [
  "browserExpanded", "selectionLaneKeys", "collapsedFamilies", "laneColours",
  "renamedLanes", "collapsedFolders", "inspectorLists",
];

const exportedMethods = [
  "invoke", "run", "update", "act", "command", "toggle",
  "missingUiParts", "renderFrame", "renderRecent", "renderRecovery", "show",
  "dockState", "setDockOverride", "panelPrefs", "effectivePanelState",
  "panelOn", "togglePanel", "openPanelAt", "renderPanelLayout",
  "hydrateLaneLayout", "saveLaneLayout", "reorderSampleFolder", "folderHeaderModel",
  "laneDisplayName", "laneSound", "laneTitle", "savedLaneRows", "orderedLaneRows",
  "familyMains", "familyOrder", "adjacentLaneForMove", "moveLane", "renameLane",
  "refreshLanePresentation", "groupedLaneRows", "renderLanes", "currentMixer",
  "mixerModel", "currentLaneHeaders", "laneHeaderModel", "laneHeaderAction",
  "mixSelectedLanes", "volumeScopeHint", "updateVolumeScope", "selectVolumeLane",
  "requestDrumMap", "setDrumView", "deletableLane", "openAddLane", "deleteActiveLane",
  "laneTint", "setLaneHeight",
  "followBookmarkPick", "selectedBookmark", "renderActiveLane", "selectedHitDetails",
  "renderSelection", "handOverInspector", "inspectorLaneNames", "currentInspector",
  "keptList", "problemStripData", "inspectorModel", "inspectorAction",
  "sampleTarget", "pickSample", "mapsetSamples", "requestLibraryFolder", "resetBrowserFolder",
  "mapsetTree", "renderSampleBrowser", "currentBrowser", "samplePreviewAction",
  "requestSamplePreview", "sampleBrowserAction",
  "prepare", "line", "sectionBars", "drawTimingGrid", "waveform", "paintSoon",
  "fillHitDiamond", "strokeHitDiamond", "comboOf", "roundedTrack", "nodeRadii",
  "drawGameplay", "overviewBins", "diamond", "drawOverview", "drawLocatorFlags",
  "drawBookmarks", "paint",
  "renderScene", "pointer", "revealLayerPreview", "scrollGrab", "refreshToolCursor",
  "updateCursorModifiers", "handOverHitCard", "hitObjectName", "refreshHitCard",
  "hitCardAt", "refreshWaveformSwitch", "setWaveformHeight", "finishWaveformDrag",
  "refreshRulerCursor", "trackRulerPointer", "refreshRulerWarningTooltip",
  "waveformTarget", "bookmarkNearFlag", "locatorFlags", "rulerGestureAt",
  "locatorPreview", "locatorDragTo", "endRulerDrag", "drainOverviewNavigation",
  "panOverview", "timelineStart", "followPlayhead", "resizeLaneHeaders",
  "refreshLaneToolbar", "changeView", "stepWheelSnap", "fitTimeline",
  "modal", "closeModal", "showOutputProblem", "renderDialogs",
  "openMergeSettings", "openGenerate", "openNewDifficulty", "openCopy",
  "openLaneColour", "openReplaceSample", "openMap", "goHome",
  "openSiblingDifficulty", "chooseSiblingDifficulty", "showTimelineEdit",
  "editBookmark", "openTimeSignatures",
  "openFolders", "openAudioOutput", "openViewSettings", "showHelp", "showShortcutEditor",
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
    ui: () => env.window?.dawsuUi ?? ui,
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

  // Bind remaining top-level controls.
  $("home-open").addEventListener("click", () => ctx.openMap());
  $("home-resume").addEventListener("click", () => { if (!state.view.busy) ctx.act({ kind: "resume" }); });
  $("home-import").addEventListener("click", () => ctx.run(async () => ctx.show(await invoke("pick_map", { archiveOnly: true }))));
  $("home-folders").addEventListener("click", () => ctx.openFolders());
  $("ramp-shape").addEventListener("change", () => {
    ctx.act({ kind: "ramp_shape", value: $("ramp-shape").value }, false);
  });
  for (const button of env.document.querySelectorAll("[data-action]")) {
    button.addEventListener("click", () => ctx.act({ kind: button.dataset.action }));
  }
  for (const id of ["snap", "snap-mode", "beats", "divisor"]) {
    $(id).addEventListener("change", () => {
      ctx.act({
        kind: "snap",
        on: $("snap").checked,
        mode: $("snap-mode").value,
        beats: limit(number("beats", state.scene.beats), 1, 64),
        divisor: limit(number("divisor", state.scene.divisor), 1, 64),
      });
    });
  }
  $("time-signatures").addEventListener("click", () => ctx.openTimeSignatures());
  ctx.toggle("object-lines", "object_lines");
  $("difficulty").addEventListener("change", () => {
    if (!state.scene) return;
    const path = $("difficulty").value;
    $("difficulty").value = state.scene.path;
    ctx.openSiblingDifficulty(path);
  });
  for (const button of elements.speedButtons) {
    button.addEventListener("click", () => ctx.act({ kind: "speed", value: Number(button.dataset.playbackSpeed) }));
  }
  // The Export window opens beside the editor. Generate, Copy and Merge settings stay until it replaces them.
  $("export-entry").addEventListener("click", () => {
    invoke("open_export_window").catch((error) => { $("status").textContent = String(error); });
  });
  $("warnings").addEventListener("click", () => {
    ctx.modal("warnings", "Audio warnings", state.view.warnings.map(paragraph), [["Close", () => ctx.closeModal(), true]]);
  });
  $("audio").addEventListener("click", () => ctx.openAudioOutput());
  $("song-mute").addEventListener("click", () => ctx.act({ kind: "song_muted", on: !state.view.songMuted }, false));
  $("song-solo").addEventListener("click", () => ctx.act({ kind: "song_solo", on: !state.view.songSolo }, false));

  // Global keyboard handler.
  env.document.addEventListener("keydown", (event) => {
    if (event.defaultPrevented || event.repeat || event.isComposing || elements.dialog.open ||
        event.target.closest("input, select, textarea, [contenteditable]")) return;
    const { key, code, ctrl, shift, alt } = keyInput(event);
    if (state.rulerDrag) {
      if (key === "Escape" && !ctrl && !shift && !alt) {
        event.preventDefault();
        ctx.endRulerDrag(event);
      }
      return;
    }
    if (key === "Escape" && state.drag && !ctrl && !shift && !alt) {
      event.preventDefault();
      const { surface, firstPoint, lastPoint, id } = state.drag;
      state.drag = null;
      if (surface.hasPointerCapture(id)) surface.releasePointerCapture(id);
      ctx.act({ ...(lastPoint ?? firstPoint), phase: "cancel" });
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

  // Native Tauri window events.
  env.window.__TAURI__.event.listen("tauri://drag-drop", ({ payload }) => {
    const map = payload.paths.find((path) => /\.(osu|osz)$/i.test(path));
    if (map) {
      ctx.act({ kind: "open", path: map });
      return;
    }
    const target = ctx.sampleTarget();
    const path = payload.paths.find((path) => /\.(wav|ogg|mp3)$/i.test(path));
    if (target && path) ctx.act({ kind: "replace_sample", target, path, mapset: false });
  });

  env.window.__TAURI__.event.listen("node-graph-viewer", ({ payload }) => {
    ctx.ui()?.setNodeGraphViewer?.(payload ?? null);
    ctx.paintSoon();
  });

  env.window.__TAURI__.event.listen("node-graph-applied", () => {
    ctx.update(null, true);
  });

  env.window.__TAURI__.event.listen("editor-close-requested", ({ payload }) => {
    if (state.view.fault) return;
    if (state.view.busy) {
      ctx.modal("close-busy", "Job in progress", [paragraph(payload)], [["Keep editor open", () => ctx.closeModal(), true]]);
      return;
    }
    ctx.modal("close-editor", "Quit dawsu!?", [paragraph(payload)], [
      ["Cancel", () => ctx.closeModal()],
      ["Discard changes", () => ctx.run(() => invoke("close_editor", { discard: true }))],
      ["Save and quit", async () => {
        ctx.closeModal();
        await ctx.act({ kind: "save" });
        if (state.view.busy) state.quitAfterSave = true;
        else if (!state.view.dirty) ctx.run(() => invoke("close_editor", { discard: false }));
      }, true],
    ]);
  });

  // Periodic background refresh for playback & job progress.
  const perf = () => env.performance ?? performance;
  const startInterval = env.setInterval ?? setInterval;
  startInterval(async () => {
    const now = perf().now();
    if (state.view.fault || state.requests || state.refreshing || now - state.lastRefresh < (state.view.playing || state.view.busy ? 85 : 1000)) return;
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
  }

  if (autoStart) ctx.act(null);
  return ctx;
}
