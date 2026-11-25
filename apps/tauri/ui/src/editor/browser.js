// Sample browser tree, library folder loading, preview requests, and Browser actions.
import { browserRowLimit, byFolderThenName, nameOf } from "./helpers.js";
import { sampleDragType, sampleDropAction } from "../lib/browser.js";

// How many library folders a search may list on its own before it stops opening new ones.
const libraryCrawlLimit = 200;

export function attachBrowser(ctx) {
  const { env, state, dom } = ctx;
  const { $ } = dom;
  const { invoke } = env.window.__TAURI__.core;

  function sampleTarget() {
    const lane = state.scene?.lanes?.[state.view.activeLane];
    return !state.drag && !state.view.busy && lane?.canReplace ? { map: state.scene.path, lane: lane.sourceKey } : null;
  }

  function pickSample(target) {
    ctx.run(async () => ctx.show(await invoke("pick_sample", { target })));
  }

  function mapsetSamples() {
    const samples = new Map();
    for (const [row, lane] of state.scene.lanes.entries()) {
      if (lane.sample && !samples.has(lane.sample)) samples.set(lane.sample, row);
    }
    return samples;
  }

  function requestLibraryFolder(folder) {
    if (!state.browserLibrary || state.browserLibraryFolders.has(folder)) return;
    const cache = state.browserLibraryFolders;
    const location = { root: state.browserLibrary, path: folder };
    cache.set(folder, null);
    ctx.run(async () => {
      try {
        const children = await invoke("list_library", { location });
        cache.set(folder, children.map(([name, directory]) => {
          const path = folder ? folder + "/" + name : name;
          return [path, directory ? null : path];
        }).sort(byFolderThenName));
      } catch (error) {
        cache.set(folder, String(error));
      }
      if (cache === state.browserLibraryFolders && state.scene && state.browserSource === "library") {
        state.browserState = "";
        ctx.renderSampleBrowser();
      }
    });
  }

  function resetBrowserFolder() {
    state.browserFolder = "";
    state.browserSample = null;
    state.browserPreview = null;
    state.browserExpanded.clear();
  }

  // The map folder samples (path to lane row) and their folder tree, built and sorted once per scene.
  function mapsetTree() {
    if (state.browserTree?.scene === state.scene) return state.browserTree;
    const folders = new Map([["", []]]);
    const samples = mapsetSamples();
    for (const [path, row] of samples) {
      const parts = path.split("/");
      let folder = "";
      for (const part of parts.slice(0, -1)) {
        const child = folder ? folder + "/" + part : part;
        if (!folders.has(child)) {
          folders.set(child, []);
          folders.get(folder).push([child, null]);
        }
        folder = child;
      }
      folders.get(folder).push([path, row]);
    }
    for (const entries of folders.values()) entries.sort(byFolderThenName);
    state.browserTree = { scene: state.scene, folders, samples };
    return state.browserTree;
  }

  function renderSampleBrowser() {
    if (state.browserMap !== state.scene.path) {
      state.browserMap = state.scene.path;
      resetBrowserFolder();
      state.browserLibraryFolders = new Map();
    }
    if (state.browserLibrary !== (state.view.hitsoundLibrary ?? null)) {
      state.browserLibrary = state.view.hitsoundLibrary ?? null;
      state.browserLibraryFolders = new Map();
      if (state.browserSource === "library") resetBrowserFolder();
    }
    const library = state.browserSource === "library";
    const filter = state.browserFilter.trim().toLowerCase();
    const classic = !!state.view.classicBrowser;
    const { folders: tree, samples: mapset } = library ? {} : mapsetTree();
    if (!library && !tree.has(state.browserFolder)) state.browserFolder = "";
    // A picked sample whose row is gone (its lane was removed, or the library file left its folder).
    const listing = library && state.browserSample
      ? state.browserLibraryFolders.get(state.browserSample.split("/").slice(0, -1).join("/")) : null;
    if (state.browserSample && (library ? Array.isArray(listing) && !listing.some(([path]) => path === state.browserSample)
      : !mapset.has(state.browserSample))) {
      state.browserSample = null;
      state.browserPreview = null;
    }
    const key = JSON.stringify([classic, state.browserSource, state.browserLibrary, state.browserFolder,
      state.browserSample, state.browserFilter, [...state.browserExpanded], state.view.busy, state.view.activeLane, state.browserPreview]);
    if (state.browserScene === state.scene && state.browserState === key) return;
    state.browserScene = state.scene;
    state.browserState = key;
    // While searching, a folder shows (open) only when a sample under it matches. A library folder
    // not listed yet is asked for and counts as a match until its answer says otherwise; past
    // libraryCrawlLimit listed folders the search stops opening new ones.
    const crawlable = folder => state.browserLibraryFolders.has(folder) || state.browserLibraryFolders.size < libraryCrawlLimit;
    const matching = new Map();
    const hasMatch = folder => {
      if (!matching.has(folder)) {
        let entries = tree?.get(folder);
        if (library) {
          if (crawlable(folder)) requestLibraryFolder(folder);
          entries = state.browserLibraryFolders.get(folder);
        }
        matching.set(folder, !Array.isArray(entries) || entries.some(([path, row]) =>
          row === null ? hasMatch(path) : nameOf(path).toLowerCase().includes(filter)));
      }
      return matching.get(folder);
    };
    const rows = [];
    const visibleSamples = new Set();
    let full = false;
    let loading = false;
    const note = (label, depth) => rows.push({ kind: "note", key: "note:" + rows.length, label, depth });
    const addRows = (folder, depth) => {
      let entries;
      if (library) {
        if (!state.browserLibrary) {
          note("No library folder chosen yet.", depth);
          return;
        }
        requestLibraryFolder(folder);
        entries = state.browserLibraryFolders.get(folder);
        if (!Array.isArray(entries)) {
          loading ||= !entries;
          note(entries || "Loading samples…", depth);
          return;
        }
      } else {
        entries = tree.get(folder);
      }
      for (const [path, row] of entries) {
        if (rows.length >= browserRowLimit) {
          full = true;
          return;
        }
        const directory = row === null;
        const label = nameOf(path);
        if (!directory && !label.toLowerCase().includes(filter)) continue;
        // Classic's library folders open one at a time, so its search keeps to the folder shown.
        if (directory && filter && !(library && classic) && !hasMatch(path)) continue;
        if (!directory) visibleSamples.add(path);
        const open = !classic && (state.browserExpanded.has(path) || !!filter && (!library || crawlable(path)));
        rows.push(directory
          ? { kind: "folder", key: path, label, title: path, depth, open, selected: false }
          : { kind: "sample", key: path, label, title: path, depth, selected: state.browserSample === path });
        if (directory && open) addRows(path, depth + 1);
      }
    };
    addRows(classic ? state.browserFolder : "", 0);
    if (full) note("Search to see more samples.", 0);
    const unopened = rows.some(row => row.kind === "folder" && !row.open);
    if (!rows.length || filter && !visibleSamples.size && !unopened && !loading) {
      note(filter ? "No matching samples." : "No samples in this folder.", 0);
    }
    const preview = state.browserSample && state.browserPreview?.path === state.browserSample && state.browserPreview.source === state.browserSource
      ? { name: nameOf(state.browserSample), title: state.browserSample, ms: state.browserPreview.ms, peaks: state.browserPreview.peaks } : null;
    state.browserModel = {
      source: state.browserSource,
      classic,
      filter: state.browserFilter,
      busy: !!state.view.busy,
      // The tree's top row: the map folder, or the library folder by its name (full path in the tooltip).
      root: classic ? null : !library ? "Mapset samples" : state.browserLibrary ? nameOf(state.browserLibrary.replace(/[\\/]+$/, "")) : null,
      rootTitle: library ? state.browserLibrary : null,
      // The Library tab without a folder offers to choose one there.
      chooseLibrary: library && !state.browserLibrary,
      folder: classic ? {
        label: state.browserFolder ? nameOf(state.browserFolder) : library ? "Library" : "Mapset samples",
        title: library ? [state.browserLibrary, state.browserFolder].filter(Boolean).join("/") : state.browserFolder,
        canUp: !!state.browserFolder,
      } : null,
      rows,
      preview,
      canAssign: !!sampleTarget() && visibleSamples.has(state.browserSample),
      canPick: !!sampleTarget(),
      canAddAsLane: !state.drag && !state.view.busy && Boolean(state.scene?.path) && visibleSamples.has(state.browserSample),
      canBrowseAdd: !state.drag && !state.view.busy && Boolean(state.scene?.path),
      // The lane Use replaces, by the name its header shows.
      target: sampleTarget() ? ctx.laneTitle(state.scene.lanes[state.view.activeLane]) : null,
    };
    try { ctx.ui()?.browser?.(state.browserModel); } catch (error) { (env.console ?? console).error(error); }
  }

  function currentBrowser() {
    return state.browserModel;
  }

  // What a sample row plays: a map folder sample through the lane that uses it, or a library file.
  function samplePreviewAction(path) {
    return state.browserSource === "library"
      ? { kind: "preview_library_sample", location: { root: state.browserLibrary, path } }
      : { kind: "lane", row: mapsetSamples().get(path), operation: "preview_sample" };
  }

  // Asks the backend for a sample's length and outline (source and lane taken at pick time); an answer
  // for an older pick is dropped. Each pick and Play asks again, so a file changed on disk shows anew.
  async function requestSamplePreview(path, source, action, request = state.previewRequest) {
    let answer = null;
    try {
      answer = await invoke("sample_preview", {
        source: source === "library"
          ? { kind: "library", location: action.location } : { kind: "lane", row: action.row },
      });
    } catch {
      // Without an answer the preview shows the file name only.
    }
    if (!answer || request !== state.previewRequest || state.browserPreview?.path !== path || state.browserPreview.source !== source) return;
    state.browserPreview = { path, source, ms: answer.ms, peaks: answer.peaks };
    if (state.scene) ctx.renderSampleBrowser();
  }

  // One sample plays and is measured at a time. Picks made meanwhile (ArrowDown held through a
  // folder) replace each other, so only the last one waits: each ask carries a request id, and an
  // answer for an older one is dropped.
  function audition(next) {
    state.previewRequest = (state.previewRequest ?? 0) + 1;
    const queued = !!state.auditionNext;
    state.auditionNext = { ...next, request: state.previewRequest };
    if (queued) return;
    ctx.run(async () => {
      const { path, source, action, request } = state.auditionNext;
      state.auditionNext = null;
      if (request !== state.previewRequest) return;
      await ctx.update(action, false);
      await requestSamplePreview(path, source, action, request);
    });
  }

  // What the Browser parts do: switch tab, search, fold or open a folder, pick, play, Up, Use, Browse.
  function sampleBrowserAction(kind, value) {
    if (!state.scene) return;
    let focusFirst = false;
    if (kind === "source") {
      if (value === state.browserSource) return;
      state.browserSource = value === "library" ? "library" : "mapset";
      resetBrowserFolder();
      if (state.browserSource === "library") state.browserLibraryFolders = new Map();
    } else if (kind === "filter") {
      state.browserFilter = String(value ?? "");
    } else if (kind === "toggle") {
      if (state.view.classicBrowser) {
        state.browserFolder = value;
        focusFirst = true;
      } else if (!state.browserExpanded.delete(value)) state.browserExpanded.add(value);
    } else if (kind === "up") {
      state.browserFolder = state.browserFolder.split("/").slice(0, -1).join("/");
      focusFirst = true;
    } else if (kind === "pick" || kind === "play") {
      const path = kind === "pick" ? value : state.browserSample;
      if (state.view.busy || !path) return;
      const source = state.browserSource;
      if (path !== state.browserSample || state.browserPreview?.path !== path || state.browserPreview.source !== source) {
        state.browserSample = path;
        state.browserPreview = { path, source, ms: null, peaks: null };
      }
      audition({ path, source, action: samplePreviewAction(path) });
    } else if (kind === "addLane") {
      if (!state.browserSample || !state.browserModel?.canAddAsLane) return;
      if (state.browserSource === "library") {
        ctx.act({ kind: "add_lane_from_library", map: state.scene.path, location: { root: state.browserLibrary, path: state.browserSample } });
      } else {
        const row = mapsetSamples().get(state.browserSample);
        const lane = row != null ? state.scene.lanes[row] : null;
        if (lane) {
          ctx.act({ kind: "show_lane", target: { map: state.scene.path, lane: lane.sourceKey } });
        }
      }
      return;
    } else if (kind === "assign") {
      const zeroLanes = (state.scene.laneOrder ?? state.scene.lanes ?? []).length === 0;
      if (zeroLanes && state.browserSample && state.browserModel?.canAddAsLane) {
        sampleBrowserAction("addLane");
        return;
      }
      const target = sampleTarget();
      if (!target || !state.browserSample || !state.browserModel?.canAssign) return;
      if (state.browserSource === "library") {
        ctx.act({ kind: "replace_library_sample", target, location: { root: state.browserLibrary, path: state.browserSample } });
      } else {
        ctx.act({ kind: "replace_sample", target, path: state.browserSample, mapset: true });
      }
      return;
    } else if (kind === "browse") {
      const target = sampleTarget();
      if (target) {
        pickSample(target);
      } else if (state.scene?.path && !state.view.busy) {
        ctx.run(async () => ctx.show(await invoke("pick_lane_sample", { map: state.scene.path })));
      }
      return;
    } else if (kind === "drag") {
      // Where the dragged sample comes from, kept until it drops (sampleDrop) or the drag ends.
      state.sampleDrag = state.view.busy ? null : { path: value, source: state.browserSource, root: state.browserLibrary };
      return;
    } else if (kind === "dragEnd") {
      state.sampleDrag = null;
      clearSampleDropTarget();
      return;
    } else if (kind === "chooseLibrary") {
      if (state.view.busy) return;
      ctx.run(async () => {
        // A cancelled picker answers nothing.
        const next = await invoke("pick_hitsound_library");
        if (next) ctx.show(next);
      });
      return;
    }
    ctx.renderSampleBrowser();
    // Classic replaces the rows on entering a folder, so focus moves to the first new row.
    if (focusFirst) $("sample-browser-entries").querySelector?.("button")?.focus?.();
  }

  // The lane row under a Browser sample dragged over the lane column or the timeline, or -1.
  function sampleDropRow(surface, event) {
    if (!state.scene || state.drag || state.view.busy) return -1;
    let row;
    if (surface === "lanes") {
      const header = event.target?.closest?.(".lane[data-lane-row]");
      row = header ? Number(header.getAttribute("data-lane-row")) : undefined;
    } else {
      const { y } = ctx.pointerIn($("grid"), event);
      row = state.laneRows[Math.floor((y + $("scroll").scrollTop) / state.scene.rowHeight)];
    }
    return typeof row === "number" && state.lanePositions.has(row) ? row : -1;
  }

  function clearSampleDropTarget() {
    for (const node of [...(env.document.querySelectorAll?.(".sample-drop-target") ?? [])]) {
      node.classList.remove("sample-drop-target");
    }
    $("grid")?.classList?.remove("sample-drop");
  }

  // Lights the lane header (and, over the timeline, the timeline) the sample would land on.
  function showSampleDropTarget(surface, row) {
    clearSampleDropTarget();
    if (row === -1) return;
    $("lanes").querySelector?.(`.lane[data-lane-row="${row}"]`)?.classList.add("sample-drop-target");
    if (surface === "grid") $("grid").classList.add("sample-drop");
  }

  function bindSampleDrop(surface) {
    const node = $(surface);
    const carriesSample = event => [...(event.dataTransfer?.types ?? [])].includes(sampleDragType);
    const droppable = (event) => {
      const row = carriesSample(event) && state.sampleDrag ? sampleDropRow(surface, event) : -1;
      const lane = row === -1 ? null : state.scene.lanes[row];
      return sampleDropAction(state.sampleDrag, state.scene?.path, lane) ? row : -1;
    };
    const canAddOnDrop = () => Boolean(state.sampleDrag?.path && state.scene?.path && !state.drag && !state.view.busy && (state.scene.laneOrder ?? state.scene.lanes ?? []).length === 0);
    node.addEventListener("dragover", (event) => {
      if (!carriesSample(event)) return;
      const row = droppable(event);
      showSampleDropTarget(surface, row);
      if (row === -1 && !canAddOnDrop()) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    });
    node.addEventListener("dragleave", (event) => {
      if (!node.contains?.(event.relatedTarget)) clearSampleDropTarget();
    });
    node.addEventListener("drop", (event) => {
      if (!carriesSample(event)) return;
      const row = droppable(event);
      const drag = state.sampleDrag;
      const addEmpty = row === -1 && canAddOnDrop();
      clearSampleDropTarget();
      state.sampleDrag = null;
      if ((!addEmpty && row === -1) || event.dataTransfer.getData(sampleDragType) !== drag?.path) return;
      event.preventDefault();
      const map = state.scene.path;
      if (addEmpty) {
        if (drag.source === "library" && drag.root) {
          ctx.act({ kind: "add_lane_from_library", map, location: { root: drag.root, path: drag.path } });
        } else {
          const mapRow = mapsetSamples().get(drag.path);
          const lane = mapRow != null ? state.scene.lanes[mapRow] : null;
          if (lane) ctx.act({ kind: "show_lane", target: { map, lane: lane.sourceKey } });
        }
        return;
      }
      const key = state.scene.lanes[row].sourceKey;
      // Rust replaces the active lane's sample only, so the lane under the drop is made active first
      // (without audition, as the lane menu does); a lane that moved or left meanwhile takes nothing.
      ctx.run(async () => {
        await ctx.update({ kind: "lane", row, operation: "activate" }, false);
        const lane = state.scene?.path === map ? state.scene.lanes?.[state.view.activeLane] : null;
        if (state.view.activeLane !== row || lane?.sourceKey !== key) return;
        const action = sampleDropAction(drag, map, lane);
        if (action) await ctx.update(action);
      });
    });
  }

  function bindBrowser() {
    $("sample-browser-close").addEventListener("click", () => ctx.togglePanel("browser"));
    bindSampleDrop("lanes");
    bindSampleDrop("grid");
  }

  Object.assign(ctx, {
    sampleTarget, pickSample, mapsetSamples, requestLibraryFolder, resetBrowserFolder,
    mapsetTree, renderSampleBrowser, currentBrowser, samplePreviewAction,
    requestSamplePreview, audition, sampleBrowserAction, bindBrowser,
  });
}
