// Timeline viewport, pointer gestures, ruler/locator/bookmark/waveform/overview interactions,
// hit hover card, and scroll/zoom/wheel snap wiring.
import {
  bookmarkFlagHeight, bookmarkHalfWidth, clock, defaultWaveformHeight, describeOutputProblem,
  hitHalfWidth, limit, locatorFlagHeight, locatorFlagWidth, minZoom, nameOf, objectReach,
  objectSize, rulerBandHeight, sampleFolderNames,
} from "./helpers.js";

export function attachTimeline(ctx) {
  const { env, state, dom, elements } = ctx;
  const { $, setValue } = dom;
  const { grid, controller, ruler, overview } = elements;
  const waveformGrip = $("waveform-resize");

  function renderScene() {
    ctx.renderLanes();
    ctx.renderSelection();
    const difficulty = $("difficulty");
    const pathKey = JSON.stringify([state.scene.path, state.scene.difficulties]);
    if (difficulty.dataset.state !== pathKey) {
      difficulty.dataset.state = pathKey;
      const paths = [...new Set([state.scene.path, ...state.scene.difficulties])];
      const OptionCtor = env.Option ?? Option;
      difficulty.replaceChildren(...paths.map((path) => {
        const name = nameOf(path);
        return new OptionCtor(name.match(/\[([^\]]+)\]\.osu$/i)?.[1] || name, path);
      }));
      difficulty.value = state.scene.path;
    }
  }

  // Pick the nearest hit on `row` (or any moment when `row === null`, e.g. the volume lane)
  // within a zoom-independent pixel half-width (`hitHalfWidth`), so targets stay easy to click
  // and sweep at every zoom level.
  function nearestHitTime(ms, row, zoom) {
    let best = hitHalfWidth / zoom;
    let found = null;
    for (const [time, , , hits] of state.scene.moments) {
      const distance = Math.abs(time - ms);
      if (distance < best && (row === null ? hits.length > 0 : hits.some(([laneRow]) => row === laneRow))) {
        best = distance;
        found = time;
      }
    }
    return found;
  }

  const uiScale = () => (Number(env.document?.documentElement?.dataset?.uiScale) || 100) / 100;
  const perf = () => env.performance ?? performance;
  // How long a wheel scroll or zoom during playback keeps Follow from moving the view.
  const followYieldMs = 1200;

  function pointerIn(surface, event) {
    const rect = surface.getBoundingClientRect();
    const s = uiScale();
    const left = rect.left ?? rect.x ?? 0;
    const top = rect.top ?? rect.y ?? 0;
    return {
      x: (event.clientX - left) / s,
      y: (event.clientY - top) / s,
      width: (rect.width ?? 0) / s,
      height: (rect.height ?? 0) / s,
      rect,
    };
  }

  // A screen point on the ruler in its own CSS pixels, with the ruler's size in them too.
  function rulerPoint(point, bounds = ruler.getBoundingClientRect()) {
    const s = uiScale();
    const { x, y } = pointerIn({ getBoundingClientRect: () => bounds }, point);
    return { x, y, width: (bounds.width ?? 0) / s, height: (bounds.height ?? 0) / s };
  }

  function pointer(event, phase, gesture = state.drag) {
    const { x, y: localY, height } = pointerIn(gesture.surface, event);
    const valueLane = gesture.surface === controller;
    const y = localY + (valueLane || gesture.panning ? 0 : $("scroll").scrollTop);
    const rowUnderPointer = Math.floor(y / state.scene.rowHeight);
    const visibleRow = phase === "double_click" ? rowUnderPointer : limit(rowUnderPointer, 0, gesture.rows.length - 1);
    const lane = gesture.rows[visibleRow];
    if (typeof lane === "number") {
      gesture.lastRow = lane;
      gesture.lastVisualRow = visibleRow;
    }
    const row = valueLane || gesture.panning ? state.view.activeLane : gesture.lastRow;
    let ms = gesture.start + x / gesture.zoom;
    const tool = state.view.tool;
    const rightClick = gesture.button === "right";
    if (!valueLane && (
      ((phase === "down" || phase === "double_click") && (rightClick || ["Select", "Erase", "Mute", "Ramp"].includes(tool))) ||
      (phase === "move" && (rightClick || ["Erase", "Mute"].includes(tool)))
    )) {
      const hit = nearestHitTime(ms, row, gesture.zoom);
      if (hit != null) {
        if (phase !== "move") gesture.hit = hit;
        ms = hit;
      }
    } else if (valueLane && phase === "down" && !event.altKey) {
      const hit = nearestHitTime(ms, null, gesture.zoom);
      if (hit != null) {
        gesture.hit = hit;
        ms = hit;
      }
    }
    return {
      kind: "pointer", phase, ms, row, x, y,
      value: valueLane ? limit((height - y) / Math.max(1, height - 3) * 100, 1, 100) : null,
      shift: event.shiftKey, ctrl: event.ctrlKey || event.metaKey, alt: event.altKey,
      ...(rightClick ? { button: "right" } : {}),
    };
  }

  function revealLayerPreview(row, time, pointerY) {
    const family = state.familyParents.get(row);
    const folder = typeof row === "string" && state.collapsedFolders.has(row);
    // On a folded main lane, Select and Erase open the layer whose outline was clicked; Draw (and the
    // other tools) get the click, so Draw stacks a hit on the main lane. A folded folder always opens.
    if (!folder && (!state.collapsedFamilies.has(family) || !["Select", "Erase"].includes(state.view.tool))) return false;
    const tolerance = hitHalfWidth / state.scene.zoom;
    let nearest;
    let distance = tolerance;
    for (const [at, , , hits] of state.scene.moments) {
      const offset = Math.abs(at - time);
      if (offset > tolerance) continue;
      if (hits.some(([hitRow]) => hitRow === row)) return false;
      for (const [hitRow] of hits) {
        if (state.previewParents.get(hitRow) !== row || offset > distance) continue;
        nearest = hitRow;
        distance = offset;
      }
    }
    if (nearest === undefined) return false;
    if (folder) {
      state.collapsedFolders.delete(row);
      state.collapsedFamilies.delete(state.scene.lanes[nearest].family);
    } else {
      state.collapsedFamilies.delete(family);
    }
    ctx.renderLanes();
    $("scroll").scrollTop = state.lanePositions.get(nearest) * state.scene.rowHeight - pointerY;
    ctx.paintSoon();
    ctx.saveLaneLayout();
    return true;
  }

  function scrollGrab(point) {
    if (state.drag.panning) $("scroll").scrollTop = state.drag.scrollTop + state.drag.firstPoint.y - point.y;
  }

  function refreshToolCursor() {
    if (!state.cursorSurface) return;
    const tool = state.view.tool === "Grab" || !state.cursorAltDraw ? state.view.tool : "Draw";
    const cursor = {
      Select: "default",
      Grab: "pointer",
      Draw: "var(--cursor-draw)",
      Erase: "var(--cursor-erase)",
    }[tool] ?? "crosshair";
    if (state.cursorSurface.style.cursor !== cursor) state.cursorSurface.style.cursor = cursor;
  }

  function updateCursorModifiers(event) {
    const altDraw = event.altKey && !event.ctrlKey && !event.metaKey && !event.getModifierState?.("AltGraph");
    if (altDraw === state.cursorAltDraw) return;
    state.cursorAltDraw = altDraw;
    refreshToolCursor();
  }

  // The hit hover card (v3 f1, states.md): the lane and its set, the time, the volume and the object
  // under a hit. Handed to the bundle only when the hovered hit changes; it never repaints the timeline.
  function handOverHitCard(card) {
    if ((card?.key ?? null) === state.hoveredHit) return;
    state.hoveredHit = card?.key ?? null;
    try { ctx.ui()?.hitCard?.(card); } catch (error) { (env.console ?? console).error(error); }
  }

  // What a moment's object is on the Song row: "circle 3", "slider 2" (its head), "slider 2 end" or
  // "slider 2 repeat", "spinner end"; "no object" for a hit without one.
  function hitObjectName(time, onObject) {
    if (!onObject) return "no object";
    const points = state.scene.gameplay?.points ?? [];
    const point = points.find(([, at]) => at === time);
    if (!point) return "spinner end";
    const [kind, , object] = point;
    const number = state.scene.combos?.objects?.[object]?.[0];
    const name = kind + (number ? " " + number : "");
    if (kind !== "slider") return name;
    const nodes = points.filter(([nodeKind, , nodeObject]) => nodeKind === "slider" && nodeObject === object).map(([, at]) => at);
    if (time === Math.min(...nodes)) return name;
    return name + (time === Math.max(...nodes) ? " end" : " repeat");
  }

  // A new scene (a keyboard zoom, Delete, Undo) can move or remove the hovered hit.
  function refreshHitCard() {
    if (state.hoveredHit && state.lastHover) handOverHitCard(hitCardAt(state.lastHover));
  }

  function hitCardAt(event) {
    state.lastHover = { clientX: event.clientX, clientY: event.clientY };
    // No card while a menu is open over the timeline.
    if (!state.scene || state.drag || state.view.busy || $("editor-menu").hidden === false) return null;
    const { x: pointerX, y: pointerY, rect } = pointerIn(grid, event);
    const scrollTop = $("scroll").scrollTop;
    const visibleRow = Math.floor((pointerY + scrollTop) / state.scene.rowHeight);
    const row = state.laneRows[visibleRow];
    if (typeof row !== "number") return null;
    const ms = state.scene.viewStart + pointerX / state.scene.zoom;
    let best = hitHalfWidth / state.scene.zoom;
    let found = null;
    for (const moment of state.scene.moments) {
      const distance = Math.abs(moment[0] - ms);
      if (distance > best) continue;
      const hit = moment[3].find(([lane]) => lane === row);
      if (!hit) continue;
      best = distance;
      found = [moment, hit];
    }
    if (!found) return null;
    const [[time, volume, onObject], [, , , hitVolume]] = found;
    const lane = state.scene.lanes?.[row];
    const name = state.renamedLanes.has(lane?.colourKey) ? ctx.laneDisplayName(lane) : ctx.laneSound(lane).name;
    const set = sampleFolderNames[lane?.group];
    // The card is placed in CSS pixels, which the UI scale enlarges again: the grid's screen edge
    // is divided by it, the offsets inside the grid already are CSS pixels.
    const s = uiScale();
    const x = (rect.left ?? rect.x ?? 0) / s + (time - state.scene.viewStart) * state.scene.zoom;
    const y = (rect.top ?? rect.y ?? 0) / s + (visibleRow + 0.5) * state.scene.rowHeight - scrollTop;
    const title = set ? name + " · " + set : name;
    const detail = "Volume " + Math.round(hitVolume ?? volume) + "% · " + (onObject ? "on " : "") + hitObjectName(time, onObject);
    return {
      // A hit that moves on screen (a zoom) or changes (an edit, Undo) is a new card, so the card follows it.
      key: [visibleRow, time, Math.round(x), Math.round(y), title, detail].join(":"),
      title,
      time: clock(time),
      detail,
      x,
      y,
    };
  }

  // The waveform switch shows on only while a waveform shows: switched on and the Song row not dragged to nothing.
  function refreshWaveformSwitch() {
    $("ruler-waveform").setAttribute?.("aria-pressed", String(state.waveformShown && state.waveformHeight > 0));
  }

  function setWaveformHeight(requested) {
    state.waveformHeight = limit(Math.round(requested), 0, 200);
    if (state.waveformHeight) state.visibleWaveformHeight = state.waveformHeight;
    $("editor-arrange").style.setProperty("--wave-height", state.waveformHeight + "px");
    refreshWaveformSwitch();
    // Under 52 px the Song row has room for one line: its buttons stay, its name goes.
    $("editor-arrange").dataset.songRow = state.waveformHeight >= 52 ? "tall" : "short";
    waveformGrip.setAttribute("aria-valuenow", String(state.waveformHeight));
    waveformGrip.setAttribute("aria-valuetext", state.waveformHeight ? state.waveformHeight + " pixels" : "Waveform hidden");
    ctx.paintSoon();
  }

  function finishWaveformDrag(cancel) {
    const previous = state.waveformDrag;
    state.waveformDrag = null;
    delete waveformGrip.dataset.dragging;
    if (cancel) setWaveformHeight(previous.height);
    if (waveformGrip.hasPointerCapture(previous.id)) waveformGrip.releasePointerCapture(previous.id);
  }

  function refreshRulerCursor() {
    let gesture = state.rulerDrag?.mode;
    if (!gesture && state.scene && state.rulerPointer) {
      const { x, y, width } = rulerPoint(state.rulerPointer);
      if (x >= 0 && x < width && y >= 0 && y < rulerBandHeight) {
        const hit = rulerGestureAt(x, y);
        gesture = y < rulerBandHeight / 2 || hit === "left" || hit === "right" ? hit : "scrub";
      }
    }
    const cursor = gesture === "left" || gesture === "right" ? "ew-resize"
      : gesture === "move" ? "pointer" : "default";
    if (ruler.style.cursor !== cursor) ruler.style.cursor = cursor;
  }

  function optimisticSeek(ms) {
    const clampedMs = limit(ms, 0, state.view.duration);
    const prevStart = state.scene.viewStart;
    state.view = { ...state.view, position: clampedMs };
    state.playheadAnchor = { position: clampedMs, at: perf().now(), held: false };
    state.pendingSeek = { path: state.scene.path, ms: clampedMs, at: perf().now() };
    if (isCentred() && grid.clientWidth > 0) {
      state.scene.viewStart = centredStart(state.scene.zoom);
      state.centreSentStart = state.scene.viewStart;
    }
    ctx.paintSoon(state.scene.viewStart === prevStart);
    return clampedMs;
  }

  async function drainRulerSeek() {
    while (state.rulerSeekTarget) {
      const { path, ms } = state.rulerSeekTarget;
      state.rulerSeekTarget = null;
      if (state.scene?.path !== path || state.view.busy) continue;
      await ctx.act({ kind: "seek", ms }, false);
    }
  }

  function queueRulerSeek(path, ms) {
    state.rulerSeekTarget = { path, ms };
    state.rulerSeekDrain ??= drainRulerSeek().finally(() => { state.rulerSeekDrain = null; });
  }

  function trackRulerPointer(event) {
    const bounds = ruler.getBoundingClientRect();
    const { x } = rulerPoint(event, bounds);
    state.rulerPointer = { clientX: event.clientX, clientY: event.clientY };
    refreshRulerCursor();
    if (state.rulerDrag && state.rulerDrag.id === event.pointerId) {
      if (state.rulerDrag.mode === "scrub") {
        const ms = snapToActiveGrid(state.rulerDrag.start + x / state.rulerDrag.zoom);
        const clampedMs = optimisticSeek(ms);
        if (clampedMs !== state.rulerDrag.lastSeekMs) {
          state.rulerDrag.lastSeekMs = clampedMs;
          queueRulerSeek(state.rulerDrag.path, clampedMs);
        }
      } else {
        state.rulerDrag.preview = locatorPreview(state.rulerDrag, x);
        ctx.paintSoon();
      }
      return;
    }
    refreshRulerWarningTooltip(bounds);
  }

  function refreshRulerWarningTooltip(bounds) {
    let nearest = null;
    if (state.rulerPointer && !state.rulerDrag && state.rulerWarningMarkers.length) {
      const rect = rulerPoint(state.rulerPointer, bounds ?? ruler.getBoundingClientRect());
      const { x, y } = rect;
      if (x >= 0 && x < rect.width && y >= rect.height - 13 && y < rect.height) {
        let distance = Infinity;
        for (const marker of state.rulerWarningMarkers) {
          const gap = Math.abs(marker.x - x);
          if (gap <= 7 && gap < distance) {
            nearest = marker;
            distance = gap;
          }
        }
      }
    }
    const nearby = nearest?.problems ?? [];
    const lines = nearby.slice(0, 8).map(describeOutputProblem);
    if (nearby.length > lines.length) lines.push("…and " + (nearby.length - lines.length) + " more.");
    const title = lines.join("\n");
    if (ruler.title !== title) ruler.title = title;
  }

  function waveformTarget(ms, y, height) {
    const dy = Math.abs(y - (height + rulerBandHeight) / 2);
    if (!state.scene.gameplay || dy > objectReach) return null;
    // Hit the circles and slider tracks at the sizes drawGameplay() draws them.
    const diameter = Math.max(0, Math.min(objectSize, height - rulerBandHeight));
    const radiusOf = ctx.nodeRadii(state.scene.gameplay.points, diameter);
    let target = null;
    let nearest = Infinity;
    for (const [kind, time, object] of state.scene.gameplay.points) {
      const distance = Math.hypot((time - ms) * state.scene.zoom, dy);
      if (distance > objectReach || distance > radiusOf(kind, time, object).radius) continue;
      if (distance <= nearest) {
        target = { object, time, node: true };
        nearest = distance;
      }
    }
    if (target) return target;
    // The last matching span wins; a reverse loop stays compatible with Safari 15 / WebKitGTK.
    const spans = state.scene.gameplay.spans;
    for (let i = spans.length - 1; i >= 0; i--) {
      const [kind, start, end, object] = spans[i];
      if (start <= ms && ms <= end && dy <= (kind === "slider" ? diameter * 0.66 / 2 : 2.5)) {
        return { object, time: start, node: false };
      }
    }
    return null;
  }

  function bookmarkNearFlag(ms, y) {
    if (y >= rulerBandHeight + bookmarkFlagHeight) return null;
    let nearest = bookmarkHalfWidth / state.scene.zoom;
    let bookmark = null;
    for (const mark of state.scene.bookmarks ?? []) {
      const distance = Math.abs(mark.time - ms);
      if (distance <= nearest) {
        nearest = distance;
        bookmark = mark;
      }
    }
    return bookmark;
  }

  // The L and R flags at the top of the ruler: L hangs right of its line, R left of its line. Near an
  // edge a flag flips to stay on screen; on a range too short for both, they hang outward. A flag
  // never covers the other one.
  function locatorFlags(width, leftTime = state.view.left, rightTime = state.view.right) {
    const flag = (time, side) => {
      const x = (time - state.scene.viewStart) * state.scene.zoom;
      const from = side > 0 ? x : x - locatorFlagWidth;
      const flipped = from < 0 || from + locatorFlagWidth > width;
      return { x, from: flipped ? (side > 0 ? x - locatorFlagWidth : x) : from, flipped };
    };
    const left = flag(leftTime, 1);
    const right = flag(rightTime, -1);
    const overlap = () => Math.abs(left.from - right.from) < locatorFlagWidth;
    if (overlap()) {
      if (left.flipped) left.from = right.from - locatorFlagWidth;
      else if (right.flipped) right.from = left.from + locatorFlagWidth;
      else {
        if (left.x >= locatorFlagWidth) left.from = left.x - locatorFlagWidth;
        if (right.x + locatorFlagWidth <= width) right.from = right.x;
        if (overlap()) right.from = left.from + locatorFlagWidth;
      }
    }
    return { left, right };
  }

  function rulerGestureAt(x, y = Infinity) {
    if (y < locatorFlagHeight) {
      const flags = locatorFlags(ruler.clientWidth || ruler.getBoundingClientRect().width);
      for (const side of ["left", "right"]) {
        if (x >= flags[side].from && x <= flags[side].from + locatorFlagWidth) return side;
      }
    }
    const left = (state.view.left - state.scene.viewStart) * state.scene.zoom;
    const right = (state.view.right - state.scene.viewStart) * state.scene.zoom;
    if (left === right) return "draw";
    if (Math.abs(x - left) <= 5) return "left";
    if (Math.abs(x - right) <= 5) return "right";
    return x > Math.min(left, right) && x < Math.max(left, right) ? "move" : "draw";
  }

  function locatorPreview(gesture, x) {
    if (Math.abs(x - gesture.x) < 3) return null;
    const { left, right, cycling, duration } = gesture;
    const to = limit(gesture.start + x / gesture.zoom, 0, duration);
    if (gesture.mode === "left" || gesture.mode === "right") {
      const moved = locatorDragTo(gesture, x);
      return gesture.mode === "left" ? [moved, right, cycling] : [left, moved, cycling];
    }
    if (gesture.mode === "move") {
      const shift = limit((x - gesture.x) / gesture.zoom, -Math.min(left, right), Math.max(0, duration - Math.max(left, right)));
      return [left + shift, right + shift, cycling];
    }
    const from = limit(gesture.start + gesture.x / gesture.zoom, 0, duration);
    return [Math.min(from, to), Math.max(from, to), true];
  }

  // A locator follows the pointer's distance from where its flag or line was grabbed, and stops at
  // the other locator: L never passes R, R never passes L.
  function snapToActiveGrid(ms) {
    if (!state.scene?.snap || !state.scene.sections?.length) return ms;
    const sections = state.scene.sections;
    let active = sections[0];
    for (const s of sections) {
      if (s[0] <= ms) active = s;
      else break;
    }
    const [start, beatLen] = active;
    if (!(beatLen > 0)) return ms;
    const step = beatLen * (state.scene.beats || 1) / (state.scene.divisor || 4);
    if (!(step > 0)) return ms;
    return start + Math.round((ms - start) / step) * step;
  }

  function locatorDragTo(gesture, x) {
    const to = gesture[gesture.mode] + (x - gesture.x) / gesture.zoom;
    // Locators already reversed by the set shortcuts drag freely, as the backend lets them.
    // Snapping is performed authoritatively by Rust (`document.composer.snap`) on `ruler_locators`.
    const ordered = gesture.left <= gesture.right;
    if (gesture.mode === "left") return limit(to, 0, ordered ? gesture.right : gesture.duration);
    return limit(to, ordered ? gesture.left : 0, gesture.duration);
  }

  function endRulerDrag(event) {
    const current = state.rulerDrag;
    state.rulerDrag = null;
    if (ruler.hasPointerCapture?.(current.id)) ruler.releasePointerCapture?.(current.id);
    state.rulerPointer = { clientX: event.clientX, clientY: event.clientY };
    refreshRulerCursor();
    ctx.paintSoon(current.mode === "scrub");
    return current;
  }

  async function drainOverviewNavigation() {
    while (state.overviewTarget) {
      const { path, fraction } = state.overviewTarget;
      state.overviewTarget = null;
      if (state.scene?.path !== path || state.view.busy) continue;
      const zoom = state.scene.zoom;
      const start = timelineStart(fraction * state.view.duration - grid.clientWidth / zoom / 2, zoom);
      if (start !== state.scene.viewStart) await changeView(start, zoom);
    }
  }

  function panOverview(event) {
    if (!state.overviewDrag || state.overviewDrag.id !== event.pointerId ||
        state.overviewDrag.path !== state.scene?.path || !overview.clientWidth) return;
    const fraction = limit((event.clientX - overview.getBoundingClientRect().left) / (overview.clientWidth * uiScale()), 0, 1);
    state.overviewTarget = { path: state.overviewDrag.path, fraction };
    // Let go of, the strip's view stays a moment before Follow may move it again.
    state.manualScrollAt = perf().now();
    // Keep the latest position while the previous viewport update is pending.
    state.overviewNavigation ??= drainOverviewNavigation().finally(() => { state.overviewNavigation = null; });
  }

  function isCentred() {
    return Boolean(state.view?.follow && ctx.ui()?.centre?.());
  }

  function centredStart(zoom) {
    return ctx.playheadPosition() - grid.clientWidth / zoom / 2;
  }

  function timelineStart(start, zoom) {
    if (isCentred() && grid.clientWidth > 0) return centredStart(zoom);
    return limit(start, 0, state.view.duration - grid.clientWidth / zoom);
  }

  // Called with each view from the editor, and with `frame` on each playback frame.
  function followPlayhead(frame = false) {
    if (!state.view.follow || state.view.busy ||
        state.drag || state.rulerDrag || state.overviewDrag || state.overviewNavigation || !grid.clientWidth) return;
    // The Playhead group's Centre (window only): the view ALWAYS stays locked with the playhead
    // dead-center — while playing, paused, seeking, or zooming out/in (never clamped to 0).
    if (ctx.ui()?.centre?.()) {
      const centred = zoom => centredStart(zoom);
      const start = centred(state.scene.zoom);
      const prev = state.scene.viewStart;
      if (frame || !state.view.playing) state.scene.viewStart = start;
      // Not from the answer to Centre's own change: at high zoom every answer would differ by a
      // pixel and ask again at once. The next refresh tells the editor.
      if (!frame && !state.followQueued && Math.abs(start - (state.centreSentStart ?? prev)) * state.scene.zoom >= 0.5) {
        queueFollow(zoom => (state.centreSentStart = centred(zoom)));
      }
      return;
    }
    if (!state.view.playing) return;
    // A wheel scroll or zoom during playback holds Follow off for a moment, so the two don't fight.
    if (perf().now() - (state.manualScrollAt ?? -Infinity) < followYieldMs) return;
    const visible = grid.clientWidth / state.scene.zoom;
    const position = ctx.playheadPosition();
    state.centreSentStart = null;
    if (state.followQueued || (position >= state.scene.viewStart && position <= state.scene.viewStart + visible * 0.98)) return;
    const paged = zoom => timelineStart(ctx.playheadPosition() - grid.clientWidth / zoom * 0.02, zoom);
    if (paged(state.scene.zoom) !== state.scene.viewStart) queueFollow(paged);
  }

  // One Follow view change at a time, until its answer is shown; frames in between ask for no more.
  function queueFollow(start) {
    state.followQueued = true;
    changeView(start).finally(() => { state.followQueued = false; });
  }

  function resizeLaneHeaders(height) {
    $("lanes").style.setProperty("--lane-row-height", height + "px");
  }

  // A lane toolbar button at the editor's limit (zoom .0005 to 3 px per ms, lanes 20 to 65 px) shows disabled.
  function refreshLaneToolbar() {
    const off = !state.scene || !!state.view.busy;
    $("ruler-zoom-out").disabled = off || state.scene.zoom <= minZoom;
    $("ruler-zoom-in").disabled = off || state.scene.zoom >= 3;
    $("lane-shorter").disabled = off || state.scene.rowHeight <= 20;
    $("lane-taller").disabled = off || state.scene.rowHeight >= 65;
  }

  // `start` may be a function of the zoom: then the zoom and lane height are the ones in place when
  // this change's turn comes, and only the start changes (Follow).
  function changeView(start, zoom, rowHeight = state.scene.rowHeight, rowAnchorY = 0) {
    const follow = typeof start === "function";
    if (!follow) {
      zoom = limit(zoom, minZoom, 3);
      rowHeight = limit(rowHeight, 20, 65);
    }
    const path = state.scene.path;
    return ctx.run(async () => {
      if (!state.scene || state.scene.path !== path) return;
      const current = state.scene;
      if (follow) {
        zoom = current.zoom;
        rowHeight = current.rowHeight;
        start = start(zoom);
      }
      const previousStart = current.viewStart;
      const previousZoom = current.zoom;
      const previousHeight = current.rowHeight;
      const scroll = $("scroll");
      const previousScroll = scroll.scrollTop;
      current.viewStart = timelineStart(start, zoom);
      current.zoom = zoom;
      current.rowHeight = rowHeight;
      if (isCentred()) state.centreSentStart = current.viewStart;
      if (rowHeight !== previousHeight) {
        resizeLaneHeaders(rowHeight);
        scroll.scrollTop = (previousScroll + rowAnchorY) * rowHeight / previousHeight - rowAnchorY;
      }
      ctx.ui()?.viewport?.(zoom, rowHeight);
      refreshLaneToolbar();
      handOverHitCard(null);
      ctx.paintSoon();

      try {
        await ctx.update({ kind: "view", start: current.viewStart, zoom, row_height: rowHeight }, false);
      } catch (error) {
        current.viewStart = previousStart;
        current.zoom = previousZoom;
        current.rowHeight = previousHeight;
        if (rowHeight !== previousHeight) {
          resizeLaneHeaders(previousHeight);
          scroll.scrollTop = previousScroll;
        }
        ctx.ui()?.viewport?.(previousZoom, previousHeight);
        ctx.paintSoon();
        throw error;
      }
    });
  }

  // Like osu!lazer, the wheel steps through the snap group the slider shows (Common 1/1-1/16,
  // Triplets 1/1-1/12, or a custom one) and stops at either end (ui/src/lib/snap.js).
  function stepWheelSnap(finer) {
    const divisor = ctx.ui()?.wheelSnap?.(finer);
    if (divisor) ctx.act({ kind: "snap", on: state.scene.snap, mode: state.scene.snapMode, beats: 1, divisor });
  }

  // One waiting wheel move: a new one of the same kind adds to it, another kind replaces it.
  function queueWheel(move) {
    const queued = state.wheelQueued;
    if (queued?.kind !== move.kind) state.wheelQueued = move;
    else if (move.kind === "rows") state.wheelQueued = { ...move, steps: queued.steps + move.steps };
    else if (move.kind === "zoom") state.wheelQueued = { ...move, deltaY: queued.deltaY + move.deltaY };
    else state.wheelQueued = { ...move, delta: queued.delta + move.delta };
  }

  // Sends the waiting wheel move once at most one request is in flight; ipc.js's run() calls this
  // again as each request finishes.
  function flushWheel() {
    const move = state.wheelQueued;
    if (!move || state.requests > 1) return;
    state.wheelQueued = null;
    if (!state.scene || state.view.busy || state.drag) return;
    const { viewStart, zoom, rowHeight } = state.scene;
    if (move.kind === "rows") {
      changeView(viewStart, zoom, Math.round(rowHeight * 1.15 ** move.steps), move.anchor);
    } else if (move.kind === "zoom") {
      const next = limit(zoom * Math.exp(-move.deltaY * 0.003), minZoom, 3);
      const nextStart = isCentred() ? centredStart(next) : viewStart + Math.max(0, move.x) * (1 / zoom - 1 / next);
      changeView(nextStart, next);
    } else if (!isCentred()) {
      changeView(viewStart + move.delta / zoom, zoom);
    }
  }

  function fitTimeline() {
    if (state.scene && !state.view.busy && state.view.duration > 0 && grid.clientWidth > 0) {
      changeView(0, grid.clientWidth / state.view.duration);
    }
  }

  function bindTimeline() {
    env.window.addEventListener?.("keydown", updateCursorModifiers, true);
    env.window.addEventListener?.("keyup", updateCursorModifiers, true);
    env.window.addEventListener?.("blur", () => {
      state.cursorAltDraw = false;
      refreshToolCursor();
    });

    for (const surface of [grid, controller]) {
      const trackToolCursor = (event) => {
        state.cursorSurface = surface;
        updateCursorModifiers(event);
        refreshToolCursor();
      };
      surface.addEventListener("pointerenter", trackToolCursor);
      surface.addEventListener("pointermove", trackToolCursor);
      surface.addEventListener("pointerleave", () => {
        if (state.cursorSurface === surface) state.cursorSurface = null;
        if (surface === grid) handOverHitCard(null);
        if (surface === controller && state.volumeHoverTime !== null) {
          state.volumeHoverTime = null;
          ctx.paintSoon();
        }
      });
    }

    for (const surface of [grid, controller]) {
      surface.addEventListener("pointerdown", (event) => {
        if (surface === grid) handOverHitCard(null);
        const rightClick = surface === grid && event.button === 2;
        if (!state.scene || state.view.busy || state.drag || state.rulerDrag || (event.button !== 0 && !rightClick)) return;
        const panning = !rightClick && surface === grid && state.view.tool === "Grab" && !event.altKey;
        if (surface === grid && !panning) {
          const { x, y } = pointerIn(grid, event);
          const row = state.laneRows[Math.floor((y + $("scroll").scrollTop) / state.scene.rowHeight)];
          if (row === undefined) return;
          const time = state.scene.viewStart + x / state.scene.zoom;
          if (!rightClick && revealLayerPreview(row, time, y)) {
            event.preventDefault();
            return;
          }
          if (typeof row !== "number") return;
        }
        event.preventDefault();
        const HtmlElement = env.HTMLElement;
        if (HtmlElement && env.document.activeElement instanceof HtmlElement) env.document.activeElement.blur();
        surface.setPointerCapture(event.pointerId);
        state.drag = {
          surface, id: event.pointerId, button: rightClick ? "right" : "left",
          start: state.scene.viewStart, zoom: state.scene.zoom,
          rows: state.laneRows, last: 0, panning, scrollTop: $("scroll").scrollTop,
        };
        state.drag.firstPoint = pointer(event, "down");
        state.drag.firstVisualRow = state.drag.lastVisualRow;
        state.drag.markingRange = !rightClick && surface === grid && state.view.tool === "Range";
        ctx.act(state.drag.firstPoint);
      });
      surface.addEventListener("pointermove", (event) => {
        if (surface === grid) handOverHitCard(hitCardAt(event));
        if (surface === controller && state.scene && !state.view.busy) {
          const { x } = pointerIn(controller, event);
          const ms = state.scene.viewStart + x / state.scene.zoom;
          const hovered = nearestHitTime(ms, null, state.scene.zoom);
          if (hovered !== state.volumeHoverTime) {
            state.volumeHoverTime = hovered;
            ctx.paintSoon();
          }
        }
        if (event.pointerId !== state.drag?.id || perf().now() - state.drag.last < 32 || state.requests > 2) return;
        state.drag.last = perf().now();
        state.drag.lastPoint = pointer(event, "move");
        scrollGrab(state.drag.lastPoint);
        ctx.act(state.drag.lastPoint);
      });
      for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
        surface.addEventListener(type, (event) => {
          if (event.pointerId !== state.drag?.id) return;
          const released = type === "pointerup";
          const end = pointer(event, released ? "up" : "cancel");
          const moved = released && Math.hypot(end.x - state.drag.firstPoint.x, end.y - state.drag.firstPoint.y) > 3;
          if (released && state.drag.markingRange) {
            const first = state.drag.firstVisualRow ?? -1;
            const last = state.drag.lastVisualRow ?? -1;
            state.markedRange = moved && first >= 0 && last >= 0 ? {
              path: state.scene.path,
              rows: [...new Set(state.drag.rows.slice(Math.min(first, last), Math.max(first, last) + 1)
                .filter(row => typeof row === "number"))],
            } : null;
          }
          if (moved) {
            scrollGrab(end);
            ctx.act({ ...end, phase: "move" });
          }
          ctx.act(end);
          state.drag = null;
        });
      }
    }

    grid.addEventListener("dblclick", (event) => {
      if (!state.scene || state.view.busy || state.drag || state.view.tool !== "Select" || event.button !== 0 ||
          event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
      const gesture = { surface: grid, start: state.scene.viewStart, zoom: state.scene.zoom, rows: state.laneRows };
      const point = pointer(event, "double_click", gesture);
      if (Number.isInteger(point.row)) {
        event.preventDefault();
        ctx.act(point);
      }
    });

    $("scroll").addEventListener("scroll", () => {
      handOverHitCard(null);
      ctx.paintSoon();
    });
    const ResizeObserverCtor = env.ResizeObserver ?? ResizeObserver;
    new ResizeObserverCtor(() => ctx.paintSoon()).observe($("scroll"));

    waveformGrip.addEventListener("pointerdown", event => {
      if (event.button !== 0 || state.drag || state.rulerDrag || state.waveformDrag) return;
      const growsDown = !waveformGrip.parentElement?.hasAttribute("data-layout-size-from-top");
      state.waveformDrag = { id: event.pointerId, y: event.clientY, height: state.waveformHeight, direction: growsDown ? 1 : -1 };
      waveformGrip.dataset.dragging = "";
      waveformGrip.setPointerCapture(event.pointerId);
    });
    waveformGrip.addEventListener("pointermove", event => {
      if (state.waveformDrag && state.waveformDrag.id === event.pointerId) {
        setWaveformHeight(state.waveformDrag.height + (event.clientY - state.waveformDrag.y) * state.waveformDrag.direction);
      }
    });
    for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
      waveformGrip.addEventListener(type, event => {
        if (state.waveformDrag && state.waveformDrag.id === event.pointerId) {
          if (type === "pointerup") setWaveformHeight(state.waveformDrag.height + (event.clientY - state.waveformDrag.y) * state.waveformDrag.direction);
          finishWaveformDrag(type !== "pointerup");
        }
      });
    }
    waveformGrip.addEventListener("dblclick", () => setWaveformHeight(defaultWaveformHeight));
    waveformGrip.addEventListener("keydown", event => {
      if (event.key === "Escape" && state.waveformDrag) {
        finishWaveformDrag(true);
      } else {
        const step = waveformGrip.parentElement?.hasAttribute("data-layout-size-from-top") ? -8 : 8;
        const heights = {
          ArrowUp: state.waveformHeight - step,
          ArrowDown: state.waveformHeight + step,
          Home: 0, End: 200, Enter: defaultWaveformHeight,
        };
        if (!Object.hasOwn(heights, event.key)) return;
        setWaveformHeight(heights[event.key]);
      }
      event.preventDefault();
    });

    ruler.addEventListener("pointerenter", trackRulerPointer);
    ruler.addEventListener("pointermove", trackRulerPointer);
    ruler.addEventListener("pointerleave", () => {
      ruler.title = "";
      state.rulerPointer = null;
      refreshRulerCursor();
    });
    ruler.addEventListener("pointerdown", (event) => {
      if (!state.scene || state.view.busy || state.drag || state.rulerDrag || event.button !== 0) return;
      const bounds = rulerPoint(event);
      const { x, y } = bounds;
      const ms = state.scene.viewStart + x / state.scene.zoom;
      const side = event.ctrlKey || event.metaKey ? "left" : event.altKey ? "right" : null;
      const flag = event.shiftKey && !side && y < rulerBandHeight ? rulerGestureAt(x, y) : null;
      if (flag === "left" || flag === "right") {
        ctx.act({ kind: "locator", end: flag }, false);
      } else if (side) {
        ctx.act({ kind: "ruler_locators", path: state.scene.path, gesture: side, from: ms, to: ms }, false);
      } else if (y < rulerBandHeight) {
        const mode = rulerGestureAt(x, y);
        const isLocatorZone = y < rulerBandHeight / 2 || mode === "left" || mode === "right";
        ruler.setPointerCapture?.(event.pointerId);
        if (isLocatorZone) {
          state.rulerDrag = {
            path: state.scene.path, id: event.pointerId, x, start: state.scene.viewStart,
            zoom: state.scene.zoom, mode, left: state.view.left, right: state.view.right,
            cycling: state.view.cycle, duration: state.view.duration,
          };
          refreshRulerCursor();
        } else {
          const start = state.scene.viewStart;
          const snapped = snapToActiveGrid(ms);
          const clampedMs = optimisticSeek(snapped);
          state.rulerDrag = {
            path: state.scene.path, id: event.pointerId, x, start,
            zoom: state.scene.zoom, mode: "scrub", lastSeekMs: clampedMs,
          };
          queueRulerSeek(state.scene.path, clampedMs);
          refreshRulerCursor();
        }
      } else {
        const bookmark = bookmarkNearFlag(ms, y);
        const target = bookmark ? null : waveformTarget(ms, y, bounds.height);
        if (target) {
          ctx.act({ kind: "waveform_object", path: state.scene.path, ...target, preview: false });
        } else if (bookmark) {
          const clampedMs = optimisticSeek(bookmark.time);
          queueRulerSeek(state.scene.path, clampedMs);
        } else {
          ruler.setPointerCapture?.(event.pointerId);
          const start = state.scene.viewStart;
          const snapped = snapToActiveGrid(ms);
          const clampedMs = optimisticSeek(snapped);
          state.rulerDrag = {
            path: state.scene.path, id: event.pointerId, x, start,
            zoom: state.scene.zoom, mode: "scrub", lastSeekMs: clampedMs,
          };
          queueRulerSeek(state.scene.path, clampedMs);
          refreshRulerCursor();
        }
      }
    });
    ruler.addEventListener("dblclick", event => {
      if (!state.scene || state.view.busy || state.drag || state.rulerDrag || event.button !== 0 ||
          event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
      const bounds = rulerPoint(event);
      const ms = state.scene.viewStart + bounds.x / state.scene.zoom;
      const { y } = bounds;
      if (y < rulerBandHeight) {
        const gesture = rulerGestureAt(bounds.x, y);
        if ((y < rulerBandHeight / 2 && gesture === "move") || gesture === "left" || gesture === "right") {
          event.preventDefault();
          ctx.act({ kind: "cycle", on: !state.view.cycle }, false);
        }
        return;
      }
      if (bookmarkNearFlag(ms, y)) return;
      const target = waveformTarget(ms, y, bounds.height);
      if (target) ctx.act({ kind: "waveform_object", path: state.scene.path, ...target, preview: true });
    });
    ruler.addEventListener("pointerup", event => {
      if (!state.rulerDrag || state.rulerDrag.id !== event.pointerId) return;
      const gesture = endRulerDrag(event);
      const { x } = rulerPoint(event);
      if (gesture.mode === "scrub") {
        const ms = snapToActiveGrid(gesture.start + x / gesture.zoom);
        const clampedMs = optimisticSeek(ms);
        if (clampedMs !== gesture.lastSeekMs) queueRulerSeek(gesture.path, clampedMs);
        return;
      }
      // Clicking/double-clicking on the cycle bar (or a second click of a double-click) never seeks the playhead.
      const moved = Math.abs(x - gesture.x) >= 3;
      if (!moved && (gesture.mode === "move" || event.detail >= 2)) return;
      const locator = gesture.mode === "left" || gesture.mode === "right";
      const from = gesture.start + gesture.x / gesture.zoom;
      ctx.act({
        kind: "ruler_locators", path: gesture.path, gesture: moved ? gesture.mode : "seek", from,
        to: !moved ? from : locator ? locatorDragTo(gesture, x) : gesture.start + x / gesture.zoom,
      }, false);
    });
    for (const type of ["pointercancel", "lostpointercapture"]) {
      ruler.addEventListener(type, event => {
        if (state.rulerDrag && state.rulerDrag.id === event.pointerId) {
          const gesture = endRulerDrag(event);
          if (gesture.mode === "scrub") state.rulerSeekTarget = null;
        }
      });
    }

    overview.addEventListener("pointerdown", event => {
      if (!state.scene || state.view.busy || state.drag || state.rulerDrag || state.overviewDrag || event.button !== 0) return;
      state.overviewDrag = { id: event.pointerId, path: state.scene.path };
      overview.setPointerCapture(event.pointerId);
      panOverview(event);
    });
    overview.addEventListener("pointermove", panOverview);
    overview.addEventListener("pointerup", event => {
      if (state.overviewDrag?.id !== event.pointerId) return;
      panOverview(event);
      state.overviewDrag = null;
    });
    for (const type of ["pointercancel", "lostpointercapture"]) {
      overview.addEventListener(type, event => {
        if (state.overviewDrag?.id !== event.pointerId) return;
        state.overviewDrag = null;
        state.overviewTarget = null;
      });
    }

    $("scroll").addEventListener("wheel", (event) => {
      if (!state.scene) return;
      const { ctrlKey, shiftKey, altKey, deltaX, deltaY } = event;
      const osu = state.view.keyPreset === "osu!";
      const x = pointerIn(grid, event).x;
      const rows = ctrlKey && shiftKey;
      const horizontal = deltaX && !deltaY;
      const snap = !horizontal && (osu ? ctrlKey : altKey && !ctrlKey);
      const zoomTime = !horizontal && (osu ? altKey && !ctrlKey : ctrlKey);
      const pan = horizontal || (!osu && shiftKey);
      const seek = osu && (shiftKey || x >= 0);
      if (!(rows || snap || zoomTime || pan || seek) || (!deltaX && !deltaY)) return;
      event.preventDefault();
      if (state.view.busy || state.drag) return;
      // Zoom, pan and lane height add up while the editor is still answering, and go as one step
      // when it has (flushWheel); snap steps and seeks are skipped meanwhile.
      const move = rows ? (deltaY ? { kind: "rows", steps: deltaY < 0 ? 1 : -1, anchor: pointerIn($("scroll"), event).y } : null)
        : snap ? null
        : zoomTime ? { kind: "zoom", deltaY, x }
        : pan ? { kind: "pan", delta: deltaX || deltaY } : null;
      if (rows && !move) return;
      if (move) {
        if (move.kind !== "rows") state.manualScrollAt = perf().now();
        queueWheel(move);
        flushWheel();
        return;
      }
      if (state.requests > 1) return;
      if (snap) {
        stepWheelSnap(deltaY < 0);
      } else {
        const id = (deltaY < 0 ? "seekBack" : "seekForward") + (shiftKey ? "Far" : "");
        ctx.act({ kind: "command", id }, false);
      }
    }, { passive: false });

    $("ruler-fit").addEventListener("click", fitTimeline);
    $("ruler-waveform").addEventListener("click", () => {
      if (state.scene && !state.view.busy && !state.waveformDrag) {
        // v3 §4 (Yoge): the switch hides only the waveform; the Song row, circles and digits stay.
        state.waveformShown = !state.waveformShown;
        refreshWaveformSwitch();
        ctx.paintSoon();
      }
    });
    // Sound-first: one click hides the missing-object warnings on the ruler, timeline and overview.
    $("ruler-sound-first").setAttribute?.("aria-pressed", String(state.soundFirst));
    $("ruler-sound-first").addEventListener("click", () => {
      state.soundFirst = !state.soundFirst;
      try { env.window.localStorage?.setItem("dawsu.soundFirst", state.soundFirst ? "1" : "0"); } catch {}
      $("ruler-sound-first").setAttribute?.("aria-pressed", String(state.soundFirst));
      ctx.paintSoon();
    });
    // The lane toolbar (v3 §4): zoom, and Shorter and Taller lanes as View › Shorter lanes and Taller lanes.
    for (const [id, action] of [["ruler-zoom-out", "zoomOut"], ["ruler-zoom-in", "zoomIn"], ["lane-shorter", "rowsOut"], ["lane-taller", "rowsIn"]]) {
      $(id).addEventListener("click", () => {
        if (state.scene && !state.view.busy) ctx.command(action);
      });
    }
  }

  Object.assign(ctx, {
    renderScene, pointer, pointerIn, revealLayerPreview, scrollGrab, refreshToolCursor,
    updateCursorModifiers, handOverHitCard, hitObjectName, refreshHitCard,
    hitCardAt, refreshWaveformSwitch, setWaveformHeight, finishWaveformDrag,
    refreshRulerCursor, trackRulerPointer, refreshRulerWarningTooltip,
    waveformTarget, bookmarkNearFlag, locatorFlags, rulerGestureAt,
    locatorPreview, locatorDragTo, endRulerDrag, drainOverviewNavigation,
    panOverview, timelineStart, followPlayhead, queueFollow, resizeLaneHeaders,
    refreshLaneToolbar, changeView, stepWheelSnap, queueWheel, flushWheel, fitTimeline, bindTimeline,
  });
}
