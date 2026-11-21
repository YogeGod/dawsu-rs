// Lane ordering, folders, numbered families, drum view, lane headers, and Mixer strip models.
import { laneSounds, sampleFolderNames } from "./helpers.js";

export function attachLanes(ctx) {
  const { env, state, dom } = ctx;
  const { $, input, labeled, paragraph } = dom;
  const { invoke } = env.window.__TAURI__.core;

  function hydrateLaneLayout(layout) {
    state.orderedLaneKeys = [...layout?.lane_order ?? []];
    state.renamedLanes.clear();
    for (const [key, name] of Object.entries(layout?.lane_names ?? {})) state.renamedLanes.set(key, name);
    state.sampleFolderOrder = layout?.folder_order?.slice() ?? Object.keys(sampleFolderNames);
    state.collapsedFolders.clear();
    for (const group of layout?.collapsed_folders ?? []) state.collapsedFolders.add(group);
    state.laneColours.clear();
    for (const [key, colour] of Object.entries(layout?.lane_colours ?? {})) {
      if (/^#[\da-f]{6}$/i.test(colour)) state.laneColours.set(key, colour);
    }
    state.collapsedFamilies.clear();
    for (const family of layout?.collapsed_families ?? []) state.collapsedFamilies.add(family);
  }

  function saveLaneLayout(persist = false) {
    const path = state.scene.path;
    const value = {
      folder_order: [...state.sampleFolderOrder],
      collapsed_folders: [...state.collapsedFolders],
      collapsed_families: [...state.collapsedFamilies],
      lane_colours: Object.fromEntries(state.laneColours),
      ...(state.renamedLanes.size ? { lane_names: Object.fromEntries(state.renamedLanes) } : {}),
      ...(state.orderedLaneKeys.length ? { lane_order: [...state.orderedLaneKeys] } : {}),
    };
    state.layoutWrites++;
    ctx.run(async () => {
      try {
        if (path === state.scene?.path) await ctx.update({ kind: "lane_layout", path, persist, value }, false);
      } finally {
        state.layoutWrites--;
      }
    });
  }

  function reorderSampleFolder(from, to, after) {
    if (state.view.busy || state.drag || from === to ||
        !state.sampleFolderOrder.includes(from) || !state.sampleFolderOrder.includes(to)) return;
    state.sampleFolderOrder = state.sampleFolderOrder.filter(group => group !== from);
    state.sampleFolderOrder.splice(state.sampleFolderOrder.indexOf(to) + (after ? 1 : 0), 0, from);
    ctx.renderLanes();
    ctx.paintSoon();
    saveLaneLayout();
  }

  // A folder header row for the Svelte lane headers: count and used lanes, and the group's mix.
  function folderHeaderModel(group) {
    const rows = orderedLaneRows().filter(row => state.scene.lanes[row]?.group === group);
    const members = state.scene.lanes.filter(lane => lane.group === group);
    return {
      type: "folder", key: "folder:" + group, set: group, name: sampleFolderNames[group],
      count: rows.length, used: rows.filter(row => state.scene.lanes[row].hitCount > 0).length,
      collapsed: state.collapsedFolders.has(group),
      muted: members.every(lane => lane.muted), solo: members.every(lane => lane.solo),
    };
  }

  function laneDisplayName(lane) {
    return state.renamedLanes.get(lane?.colourKey) ?? lane?.name ?? "";
  }

  // A set lane reads as its sound ("Clap", or "Clap 2" for a numbered layer) with the set as a tag;
  // the skin's default sample adds "skin" to the tag. A map file lane keeps its file name and the tag "File".
  function laneSound(lane) {
    if (!lane?.group) return { name: lane?.name ?? "", set: "File", icon: "file" };
    const set = lane.group[0].toUpperCase() + lane.group.slice(1) + (lane.index === 0 ? " · skin" : "");
    const [sound, icon] = laneSounds[lane.family?.slice(lane.family.indexOf("-") + 1)] ?? [];
    return sound ? { name: sound + (lane.index > 1 ? " " + lane.index : ""), set, icon }
      : { name: lane.name, set, icon: "file" };
  }

  // The lane's name in the Inspector and the lane filter: "Soft Clap", or the mapper's own name.
  function laneTitle(lane) {
    if (!lane || state.renamedLanes.has(lane.colourKey)) return laneDisplayName(lane);
    const { name } = laneSound(lane);
    return lane.group ? lane.group[0].toUpperCase() + lane.group.slice(1) + " " + name : name;
  }

  // The saved order: the editor's lane order, sorted by the mapper's lane moves. A move edits this
  // order only; the family order below is for display and is never saved.
  function savedLaneRows() {
    const rows = state.scene.laneOrder ?? state.scene.lanes.map((_, row) => row);
    if (!state.orderedLaneKeys.length) return rows;
    const rank = new Map(state.orderedLaneKeys.map((key, index) => [key, index]));
    const position = row => rank.get(state.scene.lanes[row].colourKey) ?? rank.size;
    return [...rows].sort((left, right) => position(left) - position(right));
  }

  function orderedLaneRows() {
    return familyOrder(savedLaneRows());
  }

  // A family's main lane: the lane at custom index 1, else its skin sample (index 0), among rows.
  function familyMains(rows) {
    const mains = new Map();
    for (const row of rows) {
      const { family, index } = state.scene.lanes[row] ?? {};
      if (family != null && (index === 1 || (index === 0 && !mains.has(family)))) mains.set(family, row);
    }
    return mains;
  }

  // Numbered layers sit directly under their main lane, by number; without a main lane they stay
  // ordinary lanes where they are. main.rs's family_order does the same.
  function familyOrder(rows) {
    const mains = familyMains(rows);
    const layers = new Map();
    for (const row of rows) {
      const main = mains.get(state.scene.lanes[row]?.family);
      if (main === undefined || main === row) continue;
      if (!layers.has(main)) layers.set(main, []);
      layers.get(main).push(row);
    }
    const children = new Set([...layers.values()].flat());
    const byNumber = (left, right) => state.scene.lanes[left].index - state.scene.lanes[right].index || left - right;
    return rows.flatMap(row => children.has(row) ? [] : [row, ...(layers.get(row)?.sort(byNumber) ?? [])]);
  }

  function adjacentLaneForMove(row, direction) {
    if (state.view.drumView?.enabled || !state.scene?.lanes[row]?.colourKey || state.familyLayers.has(row)) return null;
    // A main lane moves with its layers, past the next lane or family; a layer keeps its place by number.
    const units = state.laneRows.filter(candidate => typeof candidate !== "number" || !state.familyLayers.has(candidate));
    const position = units.indexOf(row);
    if (position < 0) return null;
    const adjacent = units[position + direction];
    return typeof adjacent === "number" && state.scene.lanes[adjacent].colourKey &&
      state.scene.lanes[adjacent].group === state.scene.lanes[row].group ? adjacent : null;
  }

  function moveLane(row, direction) {
    const adjacent = adjacentLaneForMove(row, direction);
    if (adjacent === null || state.view.busy || state.drag) return;
    const rows = savedLaneRows().filter(candidate => candidate !== row);
    rows.splice(rows.indexOf(adjacent) + (direction > 0 ? 1 : 0), 0, row);
    const present = new Set(rows.map(candidate => state.scene.lanes[candidate].colourKey));
    state.orderedLaneKeys = [...rows.map(candidate => state.scene.lanes[candidate].colourKey),
      ...state.orderedLaneKeys.filter(key => !present.has(key))];
    refreshLanePresentation();
  }

  function renameLane(row) {
    const lane = state.scene?.lanes[row];
    if (!lane?.colourKey || state.view.busy || state.drag || state.view.drumView?.enabled) return;
    const path = state.scene.path;
    const field = input("text", state.renamedLanes.get(lane.colourKey) ?? "", "", "");
    field.placeholder = lane.name;
    field.maxLength = 120;
    const apply = () => {
      if (state.view.busy || state.scene?.path !== path ||
          !state.scene.lanes.some(item => item.colourKey === lane.colourKey)) return;
      const name = field.value.trim();
      if (name && name !== lane.name) state.renamedLanes.set(lane.colourKey, name);
      else state.renamedLanes.delete(lane.colourKey);
      ctx.closeModal();
      refreshLanePresentation();
    };
    field.addEventListener("keydown", event => {
      if (event.key === "Enter" && !event.isComposing) {
        event.preventDefault();
        apply();
      }
    });
    ctx.modal("rename-lane", "Rename lane", [labeled("Display name", field),
      paragraph("Leave blank to restore the sample name.")],
      [["Cancel", () => ctx.closeModal()], ["Rename", apply, true]]);
    field.focus();
  }

  function refreshLanePresentation() {
    ctx.renderLanes();
    ctx.renderActiveLane();
    ctx.paintSoon();
    saveLaneLayout(true);
  }

  function groupedLaneRows(eligible) {
    const families = new Map();
    for (const row of eligible) {
      const family = state.scene.lanes[row].family;
      if (family == null) continue;
      if (!families.has(family)) families.set(family, []);
      families.get(family).push(row);
    }
    // A family folds only under a shown main lane, so a fold never hides a layer whose main lane is
    // removed, filtered out or unused.
    const mains = familyMains(eligible);
    for (const [family, rows] of families) {
      const parent = mains.get(family);
      if (rows.length < 2 || parent === undefined) continue;
      state.familyParents.set(parent, family);
      for (const row of rows) {
        if (row === parent) continue;
        state.familyLayers.set(row, parent);
        if (state.collapsedFamilies.has(family)) state.previewParents.set(row, parent);
      }
    }

    const rowsByFolder = new Map(state.sampleFolderOrder.map(group => [group, []]));
    const looseRows = [];
    for (const row of eligible) {
      const entries = rowsByFolder.get(state.scene.lanes[row].group) ?? looseRows;
      entries.push(row);
    }
    state.laneRows = [];
    for (const [group, rows] of rowsByFolder) {
      if (!rows.length) continue;
      state.laneRows.push(group);
      if (state.collapsedFolders.has(group)) {
        for (const row of rows) state.previewParents.set(row, group);
      } else {
        state.laneRows.push(...rows.filter(row => !state.previewParents.has(row)));
      }
    }
    state.laneRows.push(...looseRows.filter(row => !state.previewParents.has(row)));
  }

  function renderLanes() {
    const filter = ($("lane-filter").value ?? "").trim().toLowerCase();
    const eligible = (row, alias) => {
      const lane = state.scene.lanes[row];
      if (state.usedLaneKeys !== null && (!lane || !state.usedLaneKeys.has(lane.key))) return false;
      return !filter || [alias, laneDisplayName(lane), laneTitle(lane), lane?.name, lane?.sample]
        .some(name => name?.toLowerCase().includes(filter));
    };
    state.familyParents = new Map();
    state.familyLayers = new Map();
    state.previewParents = new Map();
    let drumNames = [];
    if (state.view.drumView?.enabled && state.scene.drumRows) {
      const rows = state.scene.drumRows.filter(({ row, name }) => eligible(row, name));
      state.laneRows = rows.map(({ row }) => row);
      drumNames = rows.map(({ name }) => name);
    } else {
      groupedLaneRows(orderedLaneRows().filter(row => eligible(row)));
    }
    state.lanePositions = new Map();
    state.laneVisualPositions = new Map();
    for (const [position, row] of state.laneRows.entries()) {
      if (!state.lanePositions.has(row)) state.lanePositions.set(row, position);
      if (typeof row !== "number") continue;
      if (!state.laneVisualPositions.has(row)) state.laneVisualPositions.set(row, []);
      state.laneVisualPositions.get(row).push(position);
    }
    const lanePanel = $("lanes");
    lanePanel.style.setProperty("--lane-row-height", state.scene.rowHeight + "px");
    const laneKey = JSON.stringify([state.laneRows, drumNames, state.scene.lanes, state.view.activeLane, [...state.selectionLaneKeys],
      [...state.familyParents], state.view.panels?.laneNames !== false, state.showLaneInfo, [...state.laneColours], [...state.renamedLanes], !!state.view.busy,
      !!state.view.drumView?.enabled]);
    if (lanePanel.dataset.state !== laneKey) {
      lanePanel.dataset.state = laneKey;
      state.laneHeaders = laneHeaderModel(drumNames);
      try { ctx.ui()?.lanes?.(state.laneHeaders); } catch (error) { (env.console ?? console).error(error); }
    }
    const strips = mixerModel();
    const key = JSON.stringify(strips);
    if (key !== state.mixerKey) {
      state.mixerKey = key;
      state.mixerStrips = strips;
      try { ctx.ui()?.mixer?.(state.mixerStrips); } catch (error) { (env.console ?? console).error(error); }
    }
  }

  function currentMixer() {
    return state.mixerStrips;
  }

  function mixerModel() {
    const numbers = new Map(orderedLaneRows().map((row, index) => [row, index + 1]));
    const used = [...numbers.keys()].filter(row => state.scene.lanes[row]?.hitCount > 0);
    const inFolder = new Set(state.sampleFolderOrder);
    const rows = [...state.sampleFolderOrder.flatMap(group => used.filter(row => state.scene.lanes[row].group === group)),
      ...used.filter(row => !inFolder.has(state.scene.lanes[row].group))];
    const anySolo = state.scene.lanes.some(lane => lane?.solo);
    // The drum view numbers its rows by drum: a strip takes its drum row's number and name, or no
    // number when no drum row shows the lane.
    const drumRows = state.view.drumView?.enabled ? new Map() : null;
    for (const header of drumRows ? state.laneHeaders.rows : []) {
      if (header.type === "lane" && !drumRows.has(header.row)) drumRows.set(header.row, header);
    }
    return rows.map(row => {
      const lane = state.scene.lanes[row];
      const sound = laneSound(lane);
      const drum = drumRows?.get(row);
      return {
        key: "lane:" + row, row, number: drumRows ? drum?.number ?? "" : numbers.get(row),
        name: drum?.name ?? (state.renamedLanes.has(lane.colourKey) ? laneDisplayName(lane) : sound.name),
        set: sound.set, icon: sound.icon,
        // Yoge, 2026-11-20 (ADR 0068): a lane is grey unless the mapper picked a colour.
        colour: state.laneColours.get(lane.colourKey) ?? null,
        selected: state.selectionLaneKeys.has(lane.key), muted: !!lane.muted, solo: !!lane.solo,
        quiet: !!lane.muted || (anySolo && !lane.solo),
      };
    });
  }

  function currentLaneHeaders() {
    return state.laneHeaders;
  }

  function laneHeaderModel(drumNames) {
    const drum = !!state.view.drumView?.enabled;
    const numbers = new Map(orderedLaneRows().map((row, index) => [row, index + 1]));
    const anySolo = state.scene.lanes.some(lane => lane?.solo);
    const rows = state.laneRows.map((row, position) => {
      if (typeof row === "string") return folderHeaderModel(row);
      if (row === null) {
        return { type: "drum", key: "drum:" + position, name: drumNames[position], title: drumNames[position] + " · Unassigned" };
      }
      const lane = state.scene.lanes[row];
      const displayName = drumNames[position] ?? laneDisplayName(lane);
      const label = state.showLaneInfo && lane.index != null
        ? displayName + (lane.index === 0 ? " · skin" : " · idx " + lane.index)
        : displayName;
      const sound = laneSound(lane);
      const family = state.familyParents.get(row);
      const collapsed = state.collapsedFamilies.has(family);
      return {
        type: "lane", key: drum ? "drum:" + position : "lane:" + row, row,
        number: drum ? position + 1 : numbers.get(row),
        name: !state.showLaneInfo && !drum && !state.renamedLanes.has(lane.colourKey) ? sound.name : label, label, display: displayName,
        title: (lane.sample ?? label) + " · Select and audition · Ctrl/Cmd toggles · Shift selects a range",
        tag: drum ? null : sound.set, icon: sound.icon,
        // Yoge, 2026-11-20 (ADR 0068): a lane is grey unless the mapper picked a colour.
        colour: state.laneColours.get(lane.colourKey) ?? null,
        folder: !drum && lane.group ? lane.group : null,
        active: state.view.activeLane === row, selected: state.selectionLaneKeys.has(lane.key),
        muted: !!lane.muted, solo: !!lane.solo, quiet: !!lane.muted || (anySolo && !lane.solo),
        layer: !drum && state.familyLayers.has(row),
        family: family == null ? null : { collapsed, label: (collapsed ? "Expand " : "Collapse ") + lane.name + " layers" },
        // The drum view adds no lanes (vault rule) and removes none.
        addLayer: !drum && lane.canAddLayer ? "Add numbered " + lane.name + " layer" : null,
        remove: drum ? null : { label: "Remove " + (state.renamedLanes.has(lane.colourKey) ? displayName : sound.name), disabled: !lane.canDelete },
      };
    });
    return { names: state.view.panels?.laneNames !== false, busy: !!state.view.busy, rows };
  }

  // Every click on a lane or folder header comes back here from the Svelte part.
  function laneHeaderAction(kind, ...args) {
    if (!state.scene || state.view.busy || state.drag) return;
    switch (kind) {
      case "select": {
        const [row, event = {}] = args;
        selectVolumeLane(row, event);
        if (!event.ctrlKey && !event.metaKey && !event.shiftKey) ctx.act({ kind: "lane", row, operation: "audition" });
        break;
      }
      case "mix":
        mixSelectedLanes(args[0], args[1]);
        break;
      case "family": {
        const family = state.familyParents.get(args[0]);
        if (family == null) return;
        if (!state.collapsedFamilies.delete(family)) state.collapsedFamilies.add(family);
        ctx.renderLanes();
        ctx.paintSoon();
        saveLaneLayout();
        break;
      }
      case "folder": {
        const [group] = args;
        if (!state.collapsedFolders.delete(group)) state.collapsedFolders.add(group);
        ctx.renderLanes();
        ctx.paintSoon();
        saveLaneLayout();
        break;
      }
      case "folderMix":
        ctx.act({ kind: "group_mix", set: args[0], operation: args[1] });
        break;
      case "folderMove":
        reorderSampleFolder(...args);
        break;
      case "folderStep": {
        const [group, after] = args;
        const headers = state.laneRows.filter(row => typeof row === "string");
        const next = headers[headers.indexOf(group) + (after ? 1 : -1)];
        if (next) reorderSampleFolder(group, next, after);
        break;
      }
      case "addLayer":
        // The editor takes the next free number. A removed layer keeps its file in the mapset, so +
        // never reuses its number (safe: nothing is overwritten).
        if (!state.view.drumView?.enabled) ctx.act({ kind: "lane", row: args[0], operation: "add_layer" });
        break;
      case "remove": {
        // One undo step takes the lane and its hits; Undo brings both back, so there is no question
        // first. Presses while a removal is on its way are ignored.
        const lane = state.scene.lanes[args[0]];
        if (!lane?.canDelete || state.view.drumView?.enabled || state.laneRemoving) return;
        state.laneRemoving = true;
        ctx.act({ kind: "delete_lane", target: { map: state.scene.path, lane: lane.sourceKey }, revision: state.scene.laneRevision })
          .finally(() => { state.laneRemoving = false; });
        break;
      }
    }
  }

  function mixSelectedLanes(row, operation) {
    const lane = state.scene?.lanes[row];
    if (!lane || state.view.busy || state.drag) return;
    if (!state.selectionLaneKeys.has(lane.key)) selectVolumeLane(row, {});
    const lanes = state.scene.lanes.filter(item => state.selectionLaneKeys.has(item.key)).map(item => item.sourceKey);
    ctx.act({ kind: "lane_selection_mix", target: { map: state.scene.path, lane: lane.sourceKey }, operation, lanes });
  }

  // The Inspector's line under Change volume.
  function volumeScopeHint() {
    return state.selectionLanesOnly
      ? (state.selectionLaneKeys.size ? "Changes selected lanes at the selected times." : "Select lanes to change their volume.")
      : "Changes all sounds at the selected times.";
  }

  function updateVolumeScope() {
    ctx.renderSelection();
    if (state.scene) {
      const lanes = state.selectionLanesOnly ? [...state.selectionLaneKeys] : null;
      ctx.act({ kind: "volume_scope", path: state.scene.path, lanes }, false);
    }
  }

  function selectVolumeLane(row, event) {
    const key = state.scene.lanes[row].key;
    const toggle = !!(event.ctrlKey || event.metaKey);
    if (!toggle) state.selectionLaneKeys.clear();
    const visible = event.shiftKey
      ? [...new Set(state.laneRows.filter(item => typeof item === "number").map(item => state.scene.lanes[item].key))]
      : [];
    const start = visible.indexOf(state.selectionLaneAnchor);
    const end = visible.indexOf(key);
    if (start >= 0 && end >= 0) {
      for (const item of visible.slice(Math.min(start, end), Math.max(start, end) + 1)) {
        state.selectionLaneKeys.add(item);
      }
    } else {
      if (!toggle || !state.selectionLaneKeys.delete(key)) state.selectionLaneKeys.add(key);
      state.selectionLaneAnchor = key;
    }
    if (state.selectionLanesOnly) updateVolumeScope();
    ctx.renderLanes();
  }

  function requestDrumMap() {
    if (!state.scene || state.view.busy || state.drag) return;
    const map = state.scene.path;
    ctx.run(async () => {
      if (state.scene?.path !== map || state.view.busy || state.drag) return;
      const next = await invoke("pick_drum_map", { map });
      if (next) ctx.show(next);
    });
  }

  function setDrumView(on) {
    const map = state.scene.path;
    ctx.run(async () => {
      if (state.scene?.path === map && !state.view.busy && !state.drag) ctx.show(await invoke("set_drum_view", { map, on }));
    });
  }

  function deletableLane() {
    if (!state.scene || state.view.busy || state.drag || !state.lanePositions.has(state.view.activeLane)) return null;
    const lane = state.scene.lanes?.[state.view.activeLane];
    return lane?.canDelete ? lane : null;
  }

  // The Add lane row at the end of the lane column (LaneHeaders part) and the menus' Add lane….
  function openAddLane() {
    if (!state.scene || state.view.busy || state.drag) return;
    const map = state.scene.path;
    ctx.run(async () => {
      if (state.scene?.path !== map || state.view.busy || state.drag) return;
      const candidates = await invoke("lane_add_candidates", { map });
      if (state.scene?.path !== map || state.view.busy || state.drag) return;
      if (!candidates.length) {
        ctx.modal("add-lane", "Add lane", [paragraph("No new samples or hidden lanes. Put a WAV, OGG, or MP3 file in the map folder to add a lane.")],
          [["Done", () => ctx.closeModal(), true]]);
        return;
      }
      const OptionCtor = env.Option ?? Option;
      const lane = env.document.createElement("select");
      lane.append(...candidates.map(candidate => new OptionCtor(candidate.name, candidate.key)));
      lane.value = candidates[0].key;
      ctx.modal("add-lane", "Add lane", [labeled("Sample", lane)], [
        ["Cancel", () => ctx.closeModal()],
        ["Add", () => {
          const target = { map, lane: lane.value };
          ctx.closeModal();
          ctx.act({ kind: "show_lane", target });
        }, true],
      ]);
    });
  }

  // Edit › Delete active lane… and the lane menu's Delete lane… (menus.js); v3 has no toolbar button.
  function deleteActiveLane() {
    const lane = deletableLane();
    if (!lane) return;
    const target = { map: state.scene.path, lane: lane.sourceKey };
    const revision = state.scene.laneRevision;
    const remove = () => ctx.act({ kind: "delete_lane", target, revision });
    if (!lane.hitCount) {
      remove();
      return;
    }
    ctx.modal("delete-lane", "Delete lane", [
      paragraph("Delete " + lane.name + " and its " + lane.hitCount + (lane.hitCount === 1 ? " hit?" : " hits?")),
      paragraph("Undo restores the lane and its hits. The sample file stays in the map folder."),
    ], [
      ["Cancel", () => ctx.closeModal(), true],
      ["Delete", () => { ctx.closeModal(); remove(); }],
    ]);
  }

  function laneTint(lane, active = false) {
    if (!lane) return null;
    // Picked colour or none: an uncoloured lane's row stays grey (Yoge, ADR 0068). v3: 8 %, 16 % when active.
    const colour = state.laneColours.get(lane.colourKey);
    return colour ? colour + (active ? "29" : "14") : null;
  }

  function setLaneHeight(height) {
    if (state.scene && !state.view.busy && !state.drag) ctx.changeView(state.scene.viewStart, state.scene.zoom, height);
  }

  function bindLanes() {
    $("samples-view").addEventListener("click", () => {
      if (state.scene && !state.view.busy && !state.drag && state.view.drumView?.enabled) {
        setDrumView(false);
      }
    });
    $("drums-view").addEventListener("click", () => {
      if (!state.scene || state.view.busy || state.drag || state.view.drumView?.enabled) return;
      if (state.view.drumView?.name) setDrumView(true);
      else requestDrumMap();
    });
    $("lane-height-in").addEventListener("click", () => {
      if (state.scene) setLaneHeight(Math.round(state.scene.rowHeight * 1.15));
    });
    $("lane-height-out").addEventListener("click", () => {
      if (state.scene) setLaneHeight(Math.round(state.scene.rowHeight / 1.15));
    });
    $("lane-height").addEventListener("input", () => {
      if (state.requests < 2) setLaneHeight(+$("lane-height").value);
    });
    $("lane-height").addEventListener("change", () => setLaneHeight(+$("lane-height").value));
    $("collapse-layers").addEventListener("click", () => {
      if (!state.scene || state.view.busy || state.drag) return;
      for (const family of state.familyParents.values()) state.collapsedFamilies.add(family);
      ctx.renderLanes();
      ctx.paintSoon();
      saveLaneLayout();
    });
    $("lane-filter").addEventListener("input", () => {
      if (!state.scene || state.drag) return;
      ctx.renderLanes();
      ctx.paintSoon();
    });
    $("used-only").addEventListener("change", () => {
      if (!state.scene || state.view.busy || state.drag) {
        $("used-only").checked = state.usedLaneKeys !== null;
        return;
      }
      if ($("used-only").checked) {
        // Snapshot used lanes so a row does not disappear when its last hit is moved or erased.
        state.usedLaneKeys = new Set();
        for (const [, , , hits] of state.scene.moments) {
          for (const [row] of hits) state.usedLaneKeys.add(state.scene.lanes[row].key);
        }
      } else {
        state.usedLaneKeys = null;
      }
      ctx.renderLanes();
      ctx.paintSoon();
    });
  }

  Object.assign(ctx, {
    hydrateLaneLayout, saveLaneLayout, reorderSampleFolder, folderHeaderModel,
    laneDisplayName, laneSound, laneTitle, savedLaneRows, orderedLaneRows,
    familyMains, familyOrder, adjacentLaneForMove, moveLane, renameLane,
    refreshLanePresentation, groupedLaneRows, renderLanes, currentMixer,
    mixerModel, currentLaneHeaders, laneHeaderModel, laneHeaderAction,
    mixSelectedLanes, volumeScopeHint, updateVolumeScope, selectVolumeLane,
    requestDrumMap, setDrumView, deletableLane, openAddLane, deleteActiveLane,
    laneTint, setLaneHeight, bindLanes,
  });
}
