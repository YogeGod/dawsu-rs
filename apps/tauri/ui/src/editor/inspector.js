// Inspector model builder, problem strip cache, bookmark tracking, and Inspector action router.
import { analyzeCoverage, filterCoverageIssues, stepCoverageIssue as findCoverageStep } from "../lib/coverageReview.js";
import { bookmarkJumpWait, clock, limit, parseClock } from "./helpers.js";

export function attachInspector(ctx) {
  const { env, state, elements } = ctx;

  // The pick follows Previous and Next, and goes when the playhead moves off its bookmark or the
  // difficulty changes, so Go to, Rename and Remove never act on a row away from the playhead.
  function followBookmarkPick() {
    const now = (env.Date ?? Date).now();
    if (state.bookmarkPick?.path !== state.scene?.path) state.bookmarkPick = null;
    if (state.scene && state.view.bookmarkStep != null) {
      state.bookmarkPick = { path: state.scene.path, time: state.view.bookmarkStep, since: now };
    }
    if (!state.bookmarkPick) return;
    // A jump stops at the song's end.
    const at = Math.min(state.bookmarkPick.time, state.view.duration ?? Infinity);
    if (!state.scene.bookmarks?.some(mark => mark.time === state.bookmarkPick.time)) state.bookmarkPick = null;
    else if (state.view.position === at) state.bookmarkPick.since = null;
    else if (state.bookmarkPick.since === null || now - state.bookmarkPick.since > bookmarkJumpWait) state.bookmarkPick = null;
  }

  function selectedBookmark() {
    return state.bookmarkPick?.path === state.scene?.path
      ? state.scene?.bookmarks?.find(mark => mark.time === state.bookmarkPick.time)
      : undefined;
  }

  // The Svelte Inspector parts show the active lane and the selection.
  function renderActiveLane() {
    handOverInspector();
  }

  // The lanes and volumes of the selected hits, worked out once per scene from its moments.
  function selectedHitDetails() {
    if (state.selectedHitsScene === state.scene) return state.selectedHits;
    const lanes = new Set();
    const volumes = new Set();
    for (const [, volume, , hits] of state.scene?.moments ?? []) {
      for (const [row, selected, , gain] of hits) {
        if (!selected) continue;
        lanes.add(row);
        volumes.add(gain ?? volume);
      }
    }
    state.selectedHitsScene = state.scene;
    state.selectedHits = { lanes: [...lanes], volumes: [...volumes].sort((a, b) => a - b) };
    return state.selectedHits;
  }

  function renderSelection() {
    handOverInspector();
  }

  function sameInspector(a, b) {
    if (Object.is(a, b)) return true;
    if (!a || !b || typeof a !== "object" || typeof b !== "object") return false;
    if (Array.isArray(a) || Array.isArray(b)) return false;
    const keys = Object.keys(a);
    if (keys.length !== Object.keys(b).length) return false;
    for (const key of keys) {
      if (!Object.prototype.hasOwnProperty.call(b, key) || !sameInspector(a[key], b[key])) return false;
    }
    return true;
  }

  function handOverInspector() {
    const next = inspectorModel();
    if (sameInspector(state.inspectorState, next)) return;
    state.inspectorState = next;
    try {
      ctx.ui()?.inspector?.(state.inspectorState);
    } catch (error) {
      (env.console ?? console).error(error);
    }
  }

  // v3's Inspector names a lane "Clap · Soft set" and a selection's lane "Clap (Soft)". The lane
  // headers and the lane filter keep laneTitle(); a lane the mapper renamed keeps its own name.
  function inspectorLaneNames(lane) {
    if (!lane?.group || state.renamedLanes.has(lane.colourKey)) {
      const name = ctx.laneTitle(lane);
      return { title: name, short: name };
    }
    const { name } = ctx.laneSound(lane);
    const set = lane.group[0].toUpperCase() + lane.group.slice(1);
    return { title: name + " · " + set + " set", short: name + " (" + set + ")" };
  }

  function currentInspector() {
    return state.inspectorState;
  }

  // The Inspector's bookmark and history lists stay the same arrays while nothing in them changes,
  // so the 85 ms updates during playback rebuild no rows.
  function keptList(name, list) {
    const key = JSON.stringify(list);
    const kept = state.inspectorLists.get(name);
    if (kept?.key === key) return kept.list;
    state.inspectorLists.set(name, { key, list });
    return list;
  }

  // Difficulty's problem strip data, kept until the scene, the problems, the song length or the view
  // change, so the strip doesn't rebuild on every update while playing.
  function problemStripData() {
    if (!state.scene) return null;
    const viewWidth = (elements.grid.clientWidth || 0) / state.scene.zoom;
    const key = [state.scene, JSON.stringify(state.view?.outputProblems ?? []), state.view?.duration, state.scene.viewStart, viewWidth];
    if (state.problemStripKept?.key.every((value, index) => Object.is(value, key[index]))) return state.problemStripKept.strip;
    const strip = {
      path: state.scene.path, name: state.scene.version ?? "", duration: state.view?.duration ?? 0,
      problems: state.view?.outputProblems ?? [],
      orphaned: state.scene.orphanedHits ?? [],
      objects: (state.scene.moments ?? []).filter(moment => moment[2]).map(moment => moment[0]),
      viewStart: state.scene.viewStart, viewWidth,
    };
    state.problemStripKept = { key, strip };
    return strip;
  }

  // Everything the Inspector's sections show; editor state lives here.
  function inspectorModel() {
    const selection = state.scene?.selection;
    const { lanes, volumes } = selection ? selectedHitDetails() : { lanes: [], volumes: [] };
    const body = selection?.sliderBody;
    const lane = state.scene?.lanes?.[state.view?.activeLane];
    // A fault stops the editor like a running job does.
    const busy = !!state.view?.busy || !!state.view?.fault;
    return {
      busy, doc: !!state.scene && !busy,
      selection: selection ? {
        summary: selection.count + (selection.count === 1 ? " hit" : " hits") + " selected" +
          (lanes.length > 1 ? " · " + lanes.length + " lanes"
            : lanes.length ? " · " + inspectorLaneNames(state.scene.lanes?.[lanes[0]]).short : ""),
        start: clock(selection.start),
        volume: !volumes.length ? ""
          : volumes.length === 1 ? volumes[0] + "%" : volumes[0] + "–" + volumes[volumes.length - 1] + "%",
        object: selection.object,
        // The fields re-seed only when the difficulty or the slider body changes.
        sliderBody: body ? {
          key: JSON.stringify([state.scene.path, body]), set: body.set ?? "", whistle: !!body.whistle,
          whistleSet: body.whistleSet ?? "auto", volume: body.volume,
        } : null,
      } : null,
      scope: state.selectionLanesOnly ? "selected" : "all", hint: ctx.volumeScopeHint(),
      range: !!state.scene?.range, insertVolume: state.scene?.insertVolume ?? 100,
      protect: !!state.scene?.protect, audition: !!state.view?.audition, hs: !!state.scene?.hs,
      lane: lane ? {
        title: inspectorLaneNames(lane).title, sample: lane.sample && lane.sample !== lane.name ? lane.sample : "",
        colour: state.laneColours.get(lane.colourKey) ?? null, canColour: !!lane.colourKey, canReplace: !!lane.canReplace,
      } : null,
      bookmarks: keptList("bookmarks", (state.scene?.bookmarks ?? []).map(({ time, label }) => ({ time, label, clock: clock(time) }))),
      bookmark: selectedBookmark()?.time ?? null,
      history: keptList("history", state.scene ? ["Opened difficulty", ...state.scene.history ?? []] : []),
      historyPosition: state.scene?.historyPosition ?? 0,
      problemStrip: problemStripData(),
    };
  }

  // Every Inspector control comes back here from the Svelte part.
  function inspectorAction(kind, ...args) {
    switch (kind) {
      case "laneColour": return ctx.openLaneColour();
      case "replaceSample": return ctx.openReplaceSample();
      case "generate": return ctx.openGenerate();
      case "new": return ctx.openNewDifficulty();
      case "copy": return ctx.openCopy();
      case "mergeSettings": return ctx.run(() => ctx.openMergeSettings());
      case "command": return ctx.command(args[0]);
      case "insert": {
        // A blank or unreadable field sets 100 %, as the old Insert volume field's fallback did.
        const value = typeof args[0] === "number" && Number.isFinite(args[0]) ? args[0] : 100;
        return ctx.act({ kind: "insert_volume", value: limit(Math.round(value), 0, 100) });
      }
      case "toggle": return ctx.act({ kind: args[0], on: !!args[1] });
      case "history": return ctx.act({ kind: "history", position: args[0] });
    }
    if (!state.scene || state.view.busy) return false;
    switch (kind) {
      case "moveStart": {
        // The time already shown is the hits' start rounded to the millisecond; moving there would only round them.
        if (!state.scene.selection || args[0].trim() === clock(state.scene.selection.start)) return false;
        const position = parseClock(args[0]);
        if (!Number.isFinite(position)) return false;
        ctx.act({ kind: "timeline_edit", path: state.scene.path, edit: { kind: "selection_start", position } });
        return true;
      }
      case "volume": {
        const [value, relative] = args;
        if (!state.scene.selection || !Number.isFinite(value)) return false;
        ctx.act({ kind: "volume", value: limit(Math.round(value), relative ? -100 : 0, 100), relative: !!relative });
        return true;
      }
      case "nudge":
        ctx.act({ kind: "volume", relative: true, value: args[0] });
        return true;
      case "scope":
        if (state.drag) return false;
        state.selectionLanesOnly = args[0] === "selected";
        ctx.updateVolumeScope();
        return true;
      case "sliderBody": {
        const body = state.scene.selection?.sliderBody;
        const { set, whistleSet, whistle, volume } = args[0];
        if (!body || !Number.isInteger(volume) || volume < 0 || volume > 100) return false;
        ctx.act({
          kind: "slider_body", path: state.scene.path, index: body.index, set: set || null,
          whistle_set: whistleSet, whistle: !!whistle, volume,
        });
        return true;
      }
      case "jump":
        // A Difficulty problem mark: the playhead goes to its time, centred, as Next problem does.
        if (!Number.isFinite(args[0])) return false;
        ctx.act({ kind: "seek", ms: args[0] }, false);
        ctx.changeView(args[0] - elements.grid.clientWidth / state.scene.zoom / 2, state.scene.zoom);
        return true;
      case "pickBookmark": {
        const mark = state.scene.bookmarks?.find(item => item.time === args[0]);
        if (!mark) return false;
        // No show() follows a pick, so the Inspector gets it now.
        state.bookmarkPick = { path: state.scene.path, time: mark.time, since: (env.Date ?? Date).now() };
        handOverInspector();
        ctx.act({ kind: "bookmark_jump", time: mark.time });
        return true;
      }
      case "bookmark": {
        if (args[0] === "add") {
          ctx.editBookmark(null);
          return true;
        }
        const mark = selectedBookmark();
        if (!mark) return false;
        if (args[0] === "goTo") ctx.act({ kind: "bookmark_jump", time: mark.time });
        else if (args[0] === "rename") ctx.editBookmark(mark);
        else if (args[0] === "remove") ctx.act({ kind: "bookmark_edit", path: state.scene.path, time: mark.time, label: null });
        else return false;
        return true;
      }
    }
    return false;
  }

  // Step forward (+1) or backward (-1) through the open difficulty's filtered coverage issues.
  function stepCoverageIssue(direction = 1) {
    if (!state.scene || state.view?.busy) return null;
    const filter = ctx.ui()?.coverageFilter?.() ?? "all";
    const { issues } = analyzeCoverage(state.scene.moments);
    const filtered = filterCoverageIssues(issues, filter);
    const target = findCoverageStep(filtered, state.view?.position ?? 0, direction);
    if (!target) return null;
    ctx.ui()?.setCoverageFocus?.(target.time);
    inspectorAction("jump", target.time);
    return target;
  }

  Object.assign(ctx, {
    followBookmarkPick, selectedBookmark, renderActiveLane, selectedHitDetails,
    renderSelection, handOverInspector, inspectorLaneNames, currentInspector,
    keptList, problemStripData, inspectorModel, inspectorAction, stepCoverageIssue,
  });
}
