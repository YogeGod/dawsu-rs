// Core backend RPC loop, top-level view synchronization, frame chrome, and Home lists.
import { nameOf, recentMapName } from "./helpers.js";

export function attachIpc(ctx) {
  const { env, state, dom, elements } = ctx;
  const { $, makeButton, paragraph } = dom;
  const { invoke } = env.window.__TAURI__.core;

  function run(task) {
    state.requests++;
    const result = state.pending.then(task).catch((error) => {
      $("status").textContent = String(error);
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

  // The frame around the timeline: the map chip, the status bar's play state, selection, file name
  // and output device. Map metadata comes from the .osu file, so it is set as text only.
  function renderFrame(path) {
    $("map-title").textContent = state.scene?.mapTitle || (path ? nameOf(path).replace(/\.osu$/i, "") : "");
    $("map-version").textContent = state.scene?.version ?? "";
    $("map-version").hidden = !state.scene?.version;
    $("difficulty").title = path;
    const name = path ? nameOf(path) : "";
    // The name is cut in the middle: the head shrinks with an ellipsis, the tail ("[Hard].osu") stays.
    const tail = /\[[^\]]*\]\.osu$/i.exec(name)?.[0] ?? name.slice(-Math.min(name.length, 12));
    $("file-name-head").textContent = name.slice(0, name.length - tail.length);
    $("file-name-tail").textContent = tail;
    $("file-name").title = path;
    $("file-name").hidden = !path;
    $("play-state").textContent = state.view.playing ? "Playing" : "Stopped";
    $("play-state").dataset.playing = String(!!state.view.playing);
    $("play-state").hidden = !state.scene;
    const selected = state.scene?.selection?.count ?? 0;
    $("selection-count").textContent = selected + (selected === 1 ? " hit selected" : " hits selected");
    $("selection-count").hidden = !selected;
    const device = state.view.outputDevice || "";
    $("audio").textContent = device || "Audio output";
    $("audio").setAttribute("aria-label", device ? "Audio output: " + device : "Audio output");
    $("audio").title = device ? "Audio output: " + device : "Audio output";
  }

  function renderRecent(paths) {
    const panel = $("recent");
    $("home-recent").hidden = paths.length === 0;
    if (panel.dataset.paths === JSON.stringify(paths)) return;
    panel.dataset.paths = JSON.stringify(paths);
    panel.replaceChildren(...paths.map((path) => {
      const button = makeButton("", () => ctx.act({ kind: "open", path }));
      const [name, difficulty] = recentMapName(path);
      const title = env.document.createElement("strong");
      title.textContent = name;
      const detail = env.document.createElement("span");
      detail.textContent = difficulty;
      button.append(title, detail);
      button.className = "home-recent-map";
      button.title = path;
      return button;
    }));
  }

  function renderRecovery({ offers = [], error }) {
    const section = $("recovery");
    const entries = $("recovery-entries");
    const key = JSON.stringify([offers, error, state.view.busy]);
    if (entries.dataset.state === key) return;
    entries.dataset.state = key;
    section.hidden = !error && offers.length === 0;
    entries.replaceChildren();
    entries.style.maxHeight = "200px";
    entries.style.overflowY = "auto";
    if (error) entries.append(paragraph(error));
    for (const offer of offers) {
      const name = offer.mapset + " [" + offer.difficulty + "]";
      const date = new (env.Date ?? Date)(offer.created_ms);
      const label = paragraph(name);
      const time = env.document.createElement("time");
      time.textContent = date.toLocaleString();
      label.append(env.document.createElement("br"), time);
      const restore = makeButton("Restore", () => ctx.act({ kind: "restore_recovery", id: offer.id }));
      restore.setAttribute("aria-label", "Restore " + name);
      restore.disabled = state.view.busy;
      entries.append(label, restore);
    }
  }

  function show(next) {
    if (next.fault) {
      // A new object, so the Svelte parts see the fault and disable.
      state.view = { ...state.view, fault: next.fault };
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
      $("status").textContent = next.fault;
      const close = () => ctx.run(() => invoke("close_editor", { discard: false }));
      ctx.modal("fault", "Editor stopped", [paragraph(next.fault)], [["Close dawsu!", close, true]], close);
      return;
    }
    const previousTool = state.view.tool;
    const wasBusy = state.view.busy;
    state.view = next;
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
        $("used-only").checked = false;
        $("lane-filter").value = "";
      }
      if (loaded || !state.layoutWrites) ctx.hydrateLaneLayout(state.view.scene.laneLayout);
      // A lane that starts showing (a new layer from +, Ctrl+L or the lane menu, one Add lane shows
      // again, an Undo of −) shows at once: its family unfolds.
      const shownRows = incoming => incoming.laneOrder ?? incoming.lanes?.map((_, row) => row) ?? [];
      const before = !loaded && state.scene ? new Set(shownRows(state.scene)) : null;
      const unfolded = before ? shownRows(state.view.scene).filter(row => !before.has(row))
        .filter(row => state.collapsedFamilies.delete(state.view.scene.lanes[row]?.family)) : [];
      state.scene = state.view.scene;
      if (unfolded.length) ctx.saveLaneLayout();
    }
    ctx.followBookmarkPick();
    if (state.rulerPointer || state.rulerDrag) ctx.refreshRulerCursor();
    $("home").hidden = !!state.scene;
    $("workspace").hidden = !state.scene;
    $("home-resume").hidden = !state.view.showHome || !state.view.hasDocument;
    $("home-resume").disabled = !!state.view.busy;
    const path = state.scene?.path ?? "";
    ctx.renderFrame(path);
    $("status").textContent = ctx.missingUiParts() || state.view.status || (state.view.busy ? "Working…" : "");
    $("status").title = $("status").textContent;
    $("editor-header").dataset.empty = String(!state.scene);
    for (const control of env.document.querySelectorAll("[data-map]")) control.disabled = !state.scene;
    $("status-view").hidden = !state.scene;
    $("warnings").hidden = !(state.view.warnings?.length);
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
    $("problem-summary").hidden = problems.length === 0;
    const messages = problems.map(problem => problem.message).filter(Boolean);
    $("problem-summary").title = [...messages.slice(0, 10),
      ...(messages.length > 10 ? ["and " + (messages.length - 10) + " more"] : [])].join("\n");
    $("problem-summary-text").textContent = cannotSave;
    $("next-output-problem").hidden = problems.length === 0;
    $("next-output-problem").disabled = !!state.view.busy;
    $("playback-rates").hidden = !state.scene;
    $("hit-lane-counts").hidden = !state.scene;
    $("cancel-job").hidden = !state.view.busy;
    for (const button of env.document.querySelectorAll("[data-doc]")) button.disabled = !state.scene || !!state.view.busy;
    $("samples-view").ariaPressed = String(!state.view.drumView?.enabled);
    $("drums-view").ariaPressed = String(!!state.view.drumView?.enabled);
    $("drums-view").title = state.view.drumView?.name ?? "Open a drum map";
    $("ramp-shape").value = state.view.rampShape ?? "Linear";
    $("song-mute").ariaPressed = String(!!state.view.songMuted);
    $("song-solo").ariaPressed = String(!!state.view.songSolo);
    $("home-open").disabled = !!state.view.busy;
    $("home-import").disabled = $("home-folders").disabled = !!state.view.busy;

    ctx.renderActiveLane();
    ctx.renderPanelLayout();
    if (!state.scene && state.view.recent) ctx.renderRecent(state.view.recent);
    if (!state.scene && state.view.recovery) ctx.renderRecovery(state.view.recovery);
    else if (state.view.hasDocument && !$("recovery").hidden) {
      // Home with an open difficulty cannot restore, so hide the start-screen offers.
      $("recovery").hidden = true;
      $("recovery-entries").dataset.state = "";
    }
    if (state.view.scene) ctx.renderScene();
    else ctx.renderSelection();
    if (state.scene) {
      for (const button of elements.speedButtons) {
        button.ariaPressed = String(Number(button.dataset.playbackSpeed) === state.view.speed);
      }
      ctx.followPlayhead();
    }
    ctx.refreshLaneToolbar();
    ctx.refreshHitCard();
    ctx.renderDialogs();
    try { env.window.refreshEditorMenu?.(); } catch (error) { (env.console ?? console).error(error); }
    ctx.paintSoon();
    try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
    $("triplet-snap").checked = ctx.ui()?.snapGroup?.() === "triplets";
  }

  Object.assign(ctx, {
    invoke, run, update, act, command, toggle,
    missingUiParts, renderFrame, renderRecent, renderRecovery, show,
  });
}
