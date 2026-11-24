// Core backend RPC loop, top-level view synchronization, frame chrome, and Home lists.
import { nameOf, recentMapName } from "./helpers.js";

export function attachIpc(ctx) {
  const { env, state, dom, elements } = ctx;
  const { $, makeButton, paragraph } = dom;
  const { invoke } = env.window.__TAURI__.core;

  function setStatusNotice(text) {
    if (ctx.ui()?.statusNotice) {
      ctx.ui().statusNotice(text);
      return;
    }
    const node = $("status");
    if (!node) return;
    const shown = text || state.view?.fault || state.view?.status || (state.view?.busy ? "Working…" : "");
    node.textContent = shown;
    node.title = shown;
  }

  function run(task) {
    state.requests++;
    const result = state.pending.then(task).catch((error) => {
      setStatusNotice(String(error));
      (env.console ?? console).error(error);
    }).finally(() => {
      state.requests--;
      // A wheel zoom or pan held back while the editor answered goes now.
      ctx.flushWheel?.();
    });
    state.pending = result;
    return result;
  }

  let lastActionKind = null;

  async function update(action, full = true) {
    let next = await invoke("editor_view", { scene: full, action: action ?? null });
    if (!next.fault && !full && (state.view.busy !== next.busy || state.view.hasDocument !== next.hasDocument ||
        state.view.laneCount !== next.laneCount || state.view.showHome !== next.showHome)) {
      // The editor names a Previous or Next bookmark step once, in the first answer.
      const step = next.bookmarkStep;
      next = await invoke("editor_view", { scene: true, action: null });
      if (!next.fault) next.bookmarkStep ??= step;
    }
    // The editor sends a map's static data (timing sections, waveform, objects, combos) once. A page
    // that never got it (reloaded, or another caller took that frame) asks for it again.
    const kept = state.scene?.sections && state.scene.path === next.scene?.path &&
      state.scene.previewKey === next.scene?.previewKey;
    if (!next.fault && next.scene && !next.scene.sections && !kept) {
      const step = next.bookmarkStep;
      next = await invoke("editor_view", { scene: true, action: null, fresh: true });
      if (!next.fault) next.bookmarkStep ??= step;
    }
    lastActionKind = action?.kind ?? null;
    ctx.show(next);
    if (next.shortcutHelp) await ctx.showHelp();
    if (next.shortcutOpen && !next.busy && !ctx.chooseSiblingDifficulty()) ctx.show(await invoke("pick_map"));
    if (!next.busy) {
      switch (next.shortcutAction) {
        case "filter": {
          const filter = $("lane-filter");
          const panel = filter.closest("[popover]");
          if (panel && !panel.matches(":popover-open")) panel.showPopover();
          filter.focus();
          break;
        }
        case "newDiff":
          $("new")?.click();
          break;
        case "openFolder":
          await invoke("open_editor_folder", { folder: "map" });
          break;
        case "importOsz":
          ctx.show(await invoke("pick_map", { archiveOnly: true }));
          break;
        case "openHitsoundFolder":
          if (next.hitsoundLibrary) await invoke("open_editor_folder", { folder: "library" });
          else ctx.openFolders();
          break;
        case "reviewOutput":
          await ctx.openMergeSettings();
          break;
      }
    }
  }

  const TOOL_COMMAND_NAMES = {
    toolSelect: "Select",
    toolDraw: "Draw",
    toolErase: "Erase",
    toolMute: "Mute",
    toolGrab: "Grab",
    toolRange: "Range",
    toolRamp: "Ramp",
  };

  function optimisticToolFor(action) {
    if (!action) return null;
    if (action.kind === "tool" && typeof action.name === "string") return action.name;
    if (action.kind === "command" && Object.hasOwn(TOOL_COMMAND_NAMES, action.id)) return TOOL_COMMAND_NAMES[action.id];
    if (action.kind === "key" && !action.ctrl && !action.shift && !action.alt) {
      const osu = state.view?.keyPreset === "osu!";
      if (action.key === "1") return "Select";
      if (action.key === "2") return osu ? "Draw" : "Grab";
      if (action.key === "3" && !osu) return "Draw";
      if (action.key === "5") return "Mute";
      if (action.key === "6") return "Ramp";
    }
    return null;
  }

  function tryLaneColourHistory(action) {
    if (!state.scene || state.view.busy || state.drag) return false;
    const isUndo = (action?.kind === "command" && action.id === "undo")
      || (action?.kind === "key" && action.ctrl && !action.shift && !action.alt && String(action.key).toLowerCase() === "z");
    const isRedo = (action?.kind === "command" && action.id === "redo")
      || (action?.kind === "key" && action.ctrl && !action.alt && (
        (String(action.key).toLowerCase() === "y" && !action.shift)
        || (String(action.key).toLowerCase() === "z" && action.shift)
      ));
    const currentPos = state.scene?.history?.position ?? 0;
    if (isUndo && state.laneColourUndo?.length) {
      const top = state.laneColourUndo[state.laneColourUndo.length - 1];
      if (top.historyPos === currentPos) {
        state.laneColourUndo.pop();
        state.laneColourRedo.push(top);
        if (top.before) state.laneColours.set(top.key, top.before);
        else state.laneColours.delete(top.key);
        ctx.renderActiveLane();
        ctx.renderLanes();
        ctx.paintSoon();
        ctx.saveLaneLayout(true);
        return true;
      }
    }
    if (isRedo && state.laneColourRedo?.length) {
      const top = state.laneColourRedo[state.laneColourRedo.length - 1];
      if (top.historyPos === currentPos) {
        state.laneColourRedo.pop();
        state.laneColourUndo.push(top);
        if (top.after) state.laneColours.set(top.key, top.after);
        else state.laneColours.delete(top.key);
        ctx.renderActiveLane();
        ctx.renderLanes();
        ctx.paintSoon();
        ctx.saveLaneLayout(true);
        return true;
      }
    }
    return false;
  }

  function act(action, full = true) {
    if (tryLaneColourHistory(action)) return Promise.resolve();
    const optimisticTool = optimisticToolFor(action);
    if (optimisticTool && state.scene && !state.view.busy && !state.view.fault && state.view.tool !== optimisticTool) {
      state.view = { ...state.view, tool: optimisticTool };
      ctx.refreshToolCursor();
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
    }
    const requestFull = action?.kind === "tool" ? false : full;
    return ctx.run(() => ctx.update(action, requestFull));
  }

  const command = (id) => ctx.act({ kind: "command", id });
  const toggle = (id, kind) => $(id).addEventListener("change", () => ctx.act({ kind, on: $(id).checked }));

  // The Svelte parts mount during bundle load, so a part counts as missing only after the page has loaded.
  function missingUiParts() {
    if (env.document.readyState !== "complete") return "";
    return env.document.querySelectorAll("[data-ui-mount]:not([data-ui-ready])").length
      ? "Part of the editor did not load" : "";
  }

  // The map chip in the ribbon. Map metadata comes from the .osu file, so it is set as text only.
  function renderFrame(path) {
    $("map-title").textContent = state.scene?.mapTitle || (path ? nameOf(path).replace(/\.osu$/i, "") : "");
    $("map-version").textContent = state.scene?.version ?? "";
    $("map-version").hidden = !state.scene?.version;
    $("difficulty").title = path;
  }

  // Save and the Export window's write run in the background and finish here, in a later view:
  // a toast that goes by itself says so. A failed write keeps its error and gets no toast. The
  // last message told is kept, so a view still busy after the write (a sweep) can't swallow it.
  function announceWrite(before, after, hadMap) {
    const status = after.status ?? "";
    const title = /^Saved\b/.test(status) ? "Saved" : /^Wrote \d+ reviewed files?\b/.test(status) ? "Files written" : null;
    if (!title) {
      state.announcedWrite = null;
      return;
    }
    if (!hadMap || after.error) return;
    // The same message again is news only when the map became saved with it (a second Save).
    if (status === state.announcedWrite && !(before.dirty && !after.dirty)) return;
    state.announcedWrite = status;
    try { env.window.showToast?.({ tone: "success", title, body: status }); } catch (error) { (env.console ?? console).error(error); }
  }

  function show(next) {
    if (next.fault) {
      // A new object, so the Svelte parts see the fault and disable.
      state.view = { ...state.view, fault: next.fault };
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
      setStatusNotice(next.fault);
      const close = () => ctx.run(() => invoke("close_editor", { discard: false }));
      ctx.modal("fault", "Editor stopped", [paragraph(next.fault)], [["Close dawsu!", close, true]], close);
      return;
    }
    const previousTool = state.view.tool;
    const wasBusy = state.view.busy;
    const before = state.view;
    const hadMap = !!state.scene;
    // Where the line is now, from the view before this one.
    const drawn = ctx.playheadPosition();
    state.view = next;
    ctx.anchorPlayhead(drawn);
    if (state.view.tool !== previousTool) ctx.refreshToolCursor();
    if (!state.view.hasDocument || state.view.showHome) state.scene = null;
    if (state.view.scene) {
      const loaded = state.view.scene.path !== state.scene?.path || (wasBusy && state.view.status === "Difficulty loaded");
      if (loaded) {
        state.overviewDrag = null;
        state.overviewTarget = null;
        state.rulerSeekTarget = null;
        state.usedLaneKeys = null;
        state.markedRange = null;
        state.selectionLaneKeys.clear();
        state.selectionLaneAnchor = null;
        if (state.selectionLanesOnly) ctx.act({ kind: "volume_scope", path: state.view.scene.path, lanes: null }, false);
        state.selectionLanesOnly = false;
        state.laneFilterText = "";
        ctx.ui()?.laneOptions?.(false, "");
      }
      if (loaded || !state.layoutWrites) ctx.hydrateLaneLayout(state.view.scene.laneLayout);
      // A lane that starts showing (a new layer from +, Ctrl+L or the lane menu, one Add lane shows
      // again, an Undo of −) shows at once: its family unfolds.
      const shownRows = incoming => incoming.laneOrder ?? incoming.lanes?.map((_, row) => row) ?? [];
      const before = !loaded && state.scene ? new Set(shownRows(state.scene)) : null;
      const unfolded = before ? shownRows(state.view.scene).filter(row => !before.has(row))
        .filter(row => state.collapsedFamilies.delete(state.view.scene.lanes[row]?.family)) : [];
      // Centre keeps the playhead locked in the middle while playing, paused, seeking, or zooming.
      const centring = !loaded && state.scene && state.view.follow && ctx.ui()?.centre?.() && elements.grid.clientWidth > 0;
      if (centring) state.view.scene.viewStart = ctx.playheadPosition() - elements.grid.clientWidth / state.view.scene.zoom / 2;
      // Static map data (waveform, gameplay objects, combos, timing sections) is omitted on
      // incremental edits for the same loaded document so clicks and drags stay fast. It is kept on
      // the "Difficulty loaded" frame too: the editor sent it once, in the frame before.
      if (state.scene && state.view.scene.path === state.scene.path &&
          state.view.scene.previewKey === state.scene.previewKey) {
        state.view.scene.waveform ??= state.scene.waveform;
        state.view.scene.gameplay ??= state.scene.gameplay;
        state.view.scene.combos ??= state.scene.combos;
        state.view.scene.sections ??= state.scene.sections;
      }
      state.scene = state.view.scene;
      if (unfolded.length) ctx.saveLaneLayout();
    }
    // Fast path for pure 12-Hz playback polls: skip DOM/menu/dialog churn when only playhead/peaks moved.
    const purePlaybackPoll = Boolean(
      state.scene && !next.scene && before.playing && next.playing && lastActionKind === null &&
      !next.busy && !before.busy && next.hasDocument && !next.showHome &&
      before.dirty === next.dirty && before.fault === next.fault &&
      before.left === next.left && before.right === next.right && before.cycle === next.cycle &&
      before.activeLane === next.activeLane && before.tool === next.tool &&
      before.follow === next.follow && before.offset === next.offset &&
      before.songMuted === next.songMuted && before.songSolo === next.songSolo &&
      before.hitsoundsMuted === next.hitsoundsMuted && before.hitsoundsSolo === next.hitsoundsSolo &&
      before.song === next.song && before.main === next.main && before.hitsounds === next.hitsounds &&
      (before.outputProblems?.length ?? 0) === (next.outputProblems?.length ?? 0) &&
      !next.revert && !next.review && !next.editDialog && !next.saveProblemReview
    );
    if (purePlaybackPoll) {
      ctx.followBookmarkPick();
      ctx.followPlayhead();
      ctx.paintSoon(true);
      ctx.animatePlayback();
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
      return;
    }

    ctx.followBookmarkPick();
    if (state.rulerPointer || state.rulerDrag) ctx.refreshRulerCursor();
    $("home").hidden = !!state.scene;
    $("workspace").hidden = !state.scene;
    const path = state.scene?.path ?? "";
    ctx.renderFrame(path);
    $("editor-header").dataset.empty = String(!state.scene);
    const problems = state.view.outputProblems ?? [];
    const listKey = JSON.stringify(problems);
    if (state.outputProblemList !== listKey) {
      state.outputProblemList = listKey;
      state.outputProblemIndex = -1;
      state.rulerWarningMarkers = [];
      elements.ruler.title = "";
      if (state.modalKind === "output-problem") ctx.closeModal();
    }
    $("save-problem-badge").hidden = problems.length === 0;
    $("save-problem-badge").textContent = problems.length ? String(problems.length) : "";
    // One wording for the count: the badge, Save's name and the status bar.
    const cannotSave = problems.length + (problems.length === 1 ? " moment" : " moments") + " can't be saved";
    $("save-problem-badge").title = cannotSave;
    // Save's name says what it holds: unsaved changes and the moments Save can't write. Save is
    // icon-only below 1440 px, so its tooltip says the same.
    const save = $("save");
    const saveName = "Save" + (state.scene && state.view.dirty ? " (unsaved changes)" : "") + (problems.length ? ", " + cannotSave : "");
    save.dataset.dirty = String(!!state.scene && !!state.view.dirty);
    save.setAttribute("aria-label", saveName);
    save.title = saveName;
    for (const button of env.document.querySelectorAll("[data-doc]")) button.disabled = !state.scene || !!state.view.busy;
    $("samples-view").ariaPressed = String(!state.view.drumView?.enabled);
    $("drums-view").ariaPressed = String(!!state.view.drumView?.enabled);
    $("drums-view").title = state.view.drumView?.name ?? "Open a drum map";
    ctx.syncDrumVst3Button?.();
    $("ramp-shape").value = state.view.rampShape ?? "Linear";
    $("song-mute").ariaPressed = String(!!state.view.songMuted);
    $("song-solo").ariaPressed = String(!!state.view.songSolo);

    ctx.renderActiveLane();
    ctx.renderPanelLayout();
    if (state.view.scene) ctx.renderScene();
    else ctx.renderSelection();
    if (state.scene) ctx.followPlayhead();
    ctx.refreshLaneToolbar();
    ctx.refreshHitCard();
    ctx.renderDialogs();
    const toolOnlyUpdate = !next.scene && lastActionKind === "tool";
    const playheadOnlyUpdate = !next.scene &&
      (lastActionKind === "seek" || lastActionKind === "ruler_locators") &&
      before.left === state.view.left &&
      before.right === state.view.right &&
      before.cycle === state.view.cycle &&
      before.offset === state.view.offset;
    lastActionKind = null;
    if (!toolOnlyUpdate && !playheadOnlyUpdate) {
      try { env.window.refreshEditorMenu?.(); } catch (error) { (env.console ?? console).error(error); }
    }
    // A playback poll or playhead-only seek without a scene moved only the playhead: the cached canvas layers still hold.
    ctx.paintSoon(Boolean((state.view.playing && !next.scene) || playheadOnlyUpdate));
    if (state.scene && state.view.playing) ctx.animatePlayback();
    try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
    setStatusNotice(missingUiParts());
    announceWrite(before, state.view, hadMap);
  }

  Object.assign(ctx, {
    invoke, setStatusNotice, run, update, act, command, toggle,
    missingUiParts, renderFrame, show,
  });
}
