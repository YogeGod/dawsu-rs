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
    }).finally(() => { state.requests--; });
    state.pending = result;
    return result;
  }

  async function update(action, full = true) {
    let next = await invoke("editor_view", { scene: full, action: action ?? null });
    if (!next.fault && !full && (state.view.busy !== next.busy || state.view.hasDocument !== next.hasDocument ||
        state.view.laneCount !== next.laneCount || state.view.showHome !== next.showHome)) {
      // The editor names a Previous or Next bookmark step once, in the first answer.
      const step = next.bookmarkStep;
      next = await invoke("editor_view", { scene: true, action: null });
      if (!next.fault) next.bookmarkStep ??= step;
    }
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

  function act(action, full = true) {
    return ctx.run(() => ctx.update(action, full));
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
      // Centre scrolls the view on every frame; the editor's scene carries an older start.
      const centring = !loaded && state.scene && state.view.playing && state.view.follow && ctx.ui()?.centre?.();
      if (centring) state.view.scene.viewStart = state.scene.viewStart;
      state.scene = state.view.scene;
      if (unfolded.length) ctx.saveLaneLayout();
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
    try { env.window.refreshEditorMenu?.(); } catch (error) { (env.console ?? console).error(error); }
    ctx.paintSoon();
    if (state.scene && state.view.playing) ctx.animatePlayback();
    try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
    setStatusNotice(missingUiParts());
  }

  Object.assign(ctx, {
    invoke, setStatusNotice, run, update, act, command, toggle,
    missingUiParts, renderFrame, show,
  });
}
