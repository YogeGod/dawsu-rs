// Sample browser tree, library folder loading, preview requests, and Browser actions.
import { browserRowLimit, byFolderThenName, nameOf } from "./helpers.js";

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
    // While searching the map folder, a folder shows (open) only when a sample under it matches.
    const matching = new Map();
    const hasMatch = folder => {
      if (!matching.has(folder)) {
        matching.set(folder, tree.get(folder).some(([path, row]) =>
          row === null ? hasMatch(path) : nameOf(path).toLowerCase().includes(filter)));
      }
      return matching.get(folder);
    };
    const rows = [];
    const visibleSamples = new Set();
    let full = false;
    const note = (label, depth) => rows.push({ kind: "note", key: "note:" + rows.length, label, depth });
    const addRows = (folder, depth) => {
      let entries;
      if (library) {
        if (!state.browserLibrary) {
          note("Choose a hitsound library folder in Folders.", depth);
          return;
        }
        requestLibraryFolder(folder);
        entries = state.browserLibraryFolders.get(folder);
        if (!Array.isArray(entries)) {
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
        if (directory && filter && !library && !hasMatch(path)) continue;
        if (!directory) visibleSamples.add(path);
        const open = !classic && (state.browserExpanded.has(path) || !!filter && !library);
        rows.push(directory
          ? { kind: "folder", key: path, label, title: path, depth, open, selected: false }
          : { kind: "sample", key: path, label, title: path, depth, selected: state.browserSample === path });
        if (directory && open) addRows(path, depth + 1);
      }
    };
    addRows(classic ? state.browserFolder : "", 0);
    if (full) note("Search to see more samples.", 0);
    const unopened = rows.some(row => row.kind === "folder" && !row.open);
    if (!rows.length || filter && !visibleSamples.size && !unopened) {
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
      // osu! skin and Sample folders: shown greyed out in the tree, nothing lists them yet.
      defaults: !library && !classic,
      folder: classic ? {
        label: state.browserFolder ? nameOf(state.browserFolder) : library ? "Library" : "Mapset samples",
        title: library ? [state.browserLibrary, state.browserFolder].filter(Boolean).join("/") : state.browserFolder,
        canUp: !!state.browserFolder,
      } : null,
      rows,
      preview,
      canAssign: !!sampleTarget() && visibleSamples.has(state.browserSample),
      canPick: !!sampleTarget(),
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
  async function requestSamplePreview(path, source, action) {
    let answer = null;
    try {
      answer = await invoke("sample_preview", {
        source: source === "library"
          ? { kind: "library", location: action.location } : { kind: "lane", row: action.row },
      });
    } catch {
      // Without an answer the preview shows the file name only.
    }
    if (!answer || state.browserPreview?.path !== path || state.browserPreview.source !== source) return;
    state.browserPreview = { path, source, ms: answer.ms, peaks: answer.peaks };
    if (state.scene) ctx.renderSampleBrowser();
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
      const action = samplePreviewAction(path);
      ctx.act(action, false);
      if (path !== state.browserSample || state.browserPreview?.path !== path || state.browserPreview.source !== source) {
        state.browserSample = path;
        state.browserPreview = { path, source, ms: null, peaks: null };
      }
      ctx.run(() => requestSamplePreview(path, source, action));
    } else if (kind === "assign") {
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
      if (target) pickSample(target);
      return;
    }
    ctx.renderSampleBrowser();
    // Classic replaces the rows on entering a folder, so focus moves to the first new row.
    if (focusFirst) $("sample-browser-entries").querySelector?.("button")?.focus?.();
  }

  function bindBrowser() {
    $("sample-browser-close").addEventListener("click", () => ctx.togglePanel("browser"));
  }

  Object.assign(ctx, {
    sampleTarget, pickSample, mapsetSamples, requestLibraryFolder, resetBrowserFolder,
    mapsetTree, renderSampleBrowser, currentBrowser, samplePreviewAction,
    requestSamplePreview, sampleBrowserAction, bindBrowser,
  });
}
