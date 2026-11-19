"use strict";

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const grid = $("grid");
const controller = $("controller");
const ruler = $("ruler");
const overview = $("overview");
const dialog = $("dialog");
let view = { busy: false };
let scene = null;
let drag = null;
let modalKind = "";
let modalClose = null;
let pending = Promise.resolve();
let requests = 0;
let refreshing = false;
let lastRefresh = 0;
let frame = 0;

const nameOf = (path) => path.split(/[\\/]/).at(-1);
const limit = (x, min, max) => Math.min(Math.max(x, min), Math.max(min, max));
const command = (id) => act({ kind: "command", id });
const toggle = (id, kind) => $(id).addEventListener("change", () => act({ kind, on: $(id).checked }));
const number = (id, fallback) => $(id).valueAsNumber || ($(id).value === "0" ? 0 : fallback);

function makeButton(label, click) {
  const button = document.createElement("button");
  button.textContent = label;
  button.addEventListener("click", click);
  return button;
}

function run(task) {
  requests++;
  const result = pending.then(task).catch((error) => {
    $("status").textContent = String(error);
    console.error(error);
  }).finally(() => { requests--; });
  pending = result;
  return result;
}

async function update(action, full = true) {
  let next = await invoke("editor_view", { scene: full, action: action ?? null });
  if (!next.fault && !full && (view.busy !== next.busy || view.hasDocument !== next.hasDocument)) {
    next = await invoke("editor_view", { scene: true, action: null });
  }
  show(next);
}

function act(action, full = true) {
  return run(() => update(action, full));
}

function show(next) {
  if (next.fault) {
    view.fault = next.fault;
    $("status").textContent = next.fault;
    const close = () => run(() => invoke("close_editor", { discard: false }));
    modal("fault", "Editor stopped", [paragraph(next.fault)], [["Close dawsu!", close, true]], close);
    return;
  }
  view = next;
  if (!view.hasDocument) scene = null;
  if (view.scene) scene = view.scene;
  $("home").hidden = !!scene;
  $("workspace").hidden = !scene;
  $("title").textContent = scene ? scene.title + (view.dirty ? " *" : "") : "";
  $("status").textContent = view.status || (view.busy ? "Working…" : "");
  $("status").title = $("status").textContent;
  $("warnings").hidden = !(view.warnings?.length);
  $("cancel-job").hidden = !view.busy;
  for (const button of document.querySelectorAll("[data-doc]")) button.disabled = !scene || !!view.busy;
  $("open").disabled = $("home-open").disabled = !!view.busy;

  if (!scene && view.recent) renderRecent(view.recent);
  if (!scene && view.recovery) renderRecovery(view.recovery);
  if (view.scene) renderScene();
  if (scene) {
    setValue("speed", String(view.speed).replace(/^0\./, "."));
    $("cycle").checked = view.cycle;
    $("audition").checked = view.audition;
    $("play").textContent = view.playing ? "❚❚ Pause" : "▶ Play";
    $("position").textContent = clock(view.position);
    for (const button of $("tools").children) button.classList.toggle("active", button.textContent === view.tool);
  }
  renderDialogs();
  paintSoon();
}

function setValue(id, value) {
  if (document.activeElement !== $(id)) $(id).value = value;
}

function renderRecent(paths) {
  const panel = $("recent");
  if (panel.dataset.paths === JSON.stringify(paths)) return;
  panel.dataset.paths = JSON.stringify(paths);
  panel.replaceChildren(...paths.map((path) => {
    const button = makeButton(nameOf(path), () => act({ kind: "open", path }));
    button.title = path;
    return button;
  }));
}

function renderRecovery({ offers = [], error }) {
  const section = $("recovery");
  const entries = $("recovery-entries");
  const state = JSON.stringify([offers, error, view.busy]);
  if (entries.dataset.state === state) return;
  entries.dataset.state = state;
  section.hidden = !error && offers.length === 0;
  entries.replaceChildren();
  entries.style.maxHeight = "200px";
  entries.style.overflowY = "auto";
  if (error) entries.append(paragraph(error));
  for (const offer of offers) {
    const name = offer.mapset + " [" + offer.difficulty + "]";
    const date = new Date(offer.created_ms);
    const label = paragraph(name);
    const time = document.createElement("time");
    time.textContent = date.toLocaleString();
    label.append(document.createElement("br"), time);
    const restore = makeButton("Restore", () => act({ kind: "restore_recovery", id: offer.id }));
    restore.setAttribute("aria-label", "Restore " + name);
    restore.disabled = view.busy;
    entries.append(label, restore);
  }
}

function renderScene() {
  const lanePanel = $("lanes");
  const laneKey = JSON.stringify([scene.lanes, view.activeLane, scene.rowHeight]);
  if (lanePanel.dataset.state !== laneKey) {
    lanePanel.dataset.state = laneKey;
    lanePanel.replaceChildren(...scene.lanes.map((lane, row) => {
      const strip = document.createElement("div");
      strip.className = "lane" + (view.activeLane === row ? " active" : "");
      strip.style.height = scene.rowHeight + "px";
      const laneAction = (operation) => act({ kind: "lane", row, operation });
      const name = makeButton(lane.name, () => laneAction("audition"));
      name.className = "name";
      name.title = lane.name + " · Audition";
      const mute = makeButton("M", () => laneAction("mute"));
      mute.setAttribute("aria-label", "Mute " + lane.name);
      mute.setAttribute("aria-pressed", lane.muted);
      const solo = makeButton("S", () => laneAction("solo"));
      solo.setAttribute("aria-label", "Solo " + lane.name);
      solo.setAttribute("aria-pressed", lane.solo);
      strip.append(name, mute, solo);
      return strip;
    }));
  }
  $("snap").checked = scene.snap;
  setValue("snap-mode", scene.snapMode);
  setValue("beats", scene.beats);
  setValue("divisor", scene.divisor);
  setValue("insert", scene.insertVolume);
  setValue("zoom", Math.log10(scene.zoom));
  $("protect").checked = scene.protect;
  $("hs").checked = scene.hs;
  $("selected").textContent = scene.selected ? "(" + scene.selected + ")" : "";
  $("undo").disabled = view.busy || scene.historyPosition === 0;
  $("redo").disabled = view.busy || scene.historyPosition === scene.history.length;
  const difficulty = $("difficulty");
  const pathKey = JSON.stringify([scene.path, scene.difficulties]);
  if (difficulty.dataset.state !== pathKey) {
    difficulty.dataset.state = pathKey;
    const paths = [...new Set([scene.path, ...scene.difficulties])];
    difficulty.replaceChildren(...paths.map((path) => {
      const name = nameOf(path);
      return new Option(name.match(/\[([^\]]+)\]\.osu$/i)?.[1] || name, path);
    }));
    difficulty.value = scene.path;
  }
  const history = $("history");
  const historyKey = JSON.stringify([scene.historyPosition, scene.history]);
  if (history.dataset.state !== historyKey) {
    history.dataset.state = historyKey;
    history.replaceChildren(...["Opened difficulty", ...scene.history].map((label, position) => {
      const button = makeButton(label, () => act({ kind: "history", position }));
      button.classList.toggle("active", position === scene.historyPosition);
      return button;
    }).reverse());
  }
}

function clock(ms) {
  const rounded = Math.max(Math.round(ms), 0);
  const minutes = Math.floor(rounded / 60000);
  const seconds = (rounded % 60000 / 1000).toFixed(3);
  return String(minutes).padStart(2, "0") + ":" + seconds.padStart(6, "0");
}

function prepare(canvas, height = canvas.clientHeight) {
  const width = canvas.clientWidth;
  const scale = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(width * scale));
  const h = Math.max(1, Math.round(height * scale));
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;
  const ctx = canvas.getContext("2d");
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, width, height);
  return { ctx, width, height };
}

function line(ctx, color, x1, y1, x2, y2) {
  ctx.strokeStyle = color;
  ctx.beginPath();
  ctx.moveTo(x1 + .5, y1);
  ctx.lineTo(x2 + .5, y2);
  ctx.stroke();
}

function waveform(ctx, width, center, height, start, span) {
  const bins = scene.waveform;
  if (!bins.length || !view.duration) return;
  for (const [channel, color] of [[0, "#334a63"], [1, "#47618b"], [2, "#657ab5"], [3, "#7aaac6"]]) {
    ctx.fillStyle = color;
    for (let x = 0; x < width; x += 2) {
      const a = Math.floor((start + x * span / width) / view.duration * bins.length);
      const b = Math.min(bins.length, Math.ceil((start + (x + 2) * span / width) / view.duration * bins.length));
      let peak = 0;
      for (let i = Math.max(0, a); i < b; i++) peak = Math.max(peak, bins[i][channel]);
      const extent = limit(peak, 0, 1) * height;
      ctx.fillRect(x, center - extent, 2, extent * 2);
    }
  }
}

function paintSoon() {
  if (!frame) frame = requestAnimationFrame(() => { frame = 0; paint(); });
}

function paint() {
  if (!scene || $("workspace").hidden) return;
  const rowHeight = scene.rowHeight;
  const contentHeight = Math.max($("scroll").clientHeight, scene.lanes.length * rowHeight);
  const scrollTop = $("scroll").scrollTop;
  const canvasHeight = Math.min(contentHeight, $("scroll").clientHeight);
  grid.style.height = canvasHeight + "px";
  const g = prepare(grid, canvasHeight);
  const r = prepare(ruler);
  const v = prepare(controller);
  const o = prepare(overview);
  const { start, zoom } = { start: scene.viewStart, zoom: scene.zoom };
  const xOf = (time) => (time - start) * zoom;
  const end = start + g.width / zoom;
  const range = scene.range;
  v.ctx.fillStyle = "#172230";
  v.ctx.fillRect(0, 0, v.width, v.height);
  line(v.ctx, "#304054", 0, v.height / 2, v.width, v.height / 2);
  for (let row = Math.floor(scrollTop / rowHeight); row <= Math.ceil((scrollTop + g.height) / rowHeight); row++) {
    const y = row * rowHeight - scrollTop;
    const lane = scene.lanes[row];
    g.ctx.fillStyle = row === view.activeLane ? "#1d3440" : row % 2 ? "#192532" : "#15212e";
    if (!lane) g.ctx.fillStyle = "#121a24";
    g.ctx.fillRect(0, y, g.width, rowHeight);
    if (lane) line(g.ctx, "#304054", 0, y + rowHeight - .5, g.width, y + rowHeight - .5);
  }
  if (range) {
    g.ctx.fillStyle = "#4eafbb35";
    g.ctx.fillRect(xOf(range[0]), 0, xOf(range[1]) - xOf(range[0]), g.height);
  }
  for (const [i, [origin, beat, meter]] of scene.sections.entries()) {
    if (beat <= 0 || !meter) continue;
    const sectionEnd = scene.sections[i + 1]?.[0] ?? end;
    const subdivision = scene.snapMode !== "objects" ? beat * scene.beats / scene.divisor : beat;
    const step = beat * meter * zoom < 10 ? beat * meter * Math.ceil(24 / (beat * meter * zoom)) :
      beat * zoom < 8 ? beat * meter :
      subdivision * zoom < 7 ? beat : subdivision;
    const begin = Math.max(start, i ? origin : start);
    for (let n = Math.ceil((begin - origin) / step); ; n++) {
      const time = origin + n * step;
      if (time >= Math.min(end, sectionEnd)) break;
      const x = xOf(time);
      const bar = Math.abs((time - origin) / (beat * meter) - Math.round((time - origin) / (beat * meter))) < .001;
      line(g.ctx, bar ? "#4b6276" : "#29384a", x, 0, x, g.height);
    }
  }
  for (const [time, volume, object, hits] of scene.moments) {
    const x = xOf(time);
    if (x < -8 || x > g.width + 8) continue;
    if (object && zoom >= .06) line(g.ctx, "#69809965", x, 0, x, g.height);
    const hue = 210 + limit(volume, 0, 100) * 1.5;
    const volumeHeight = limit(volume, 0, 100) / 100 * (v.height - 3);
    v.ctx.fillStyle = "hsl(" + hue + " 70% 63%)";
    v.ctx.fillRect(x - 1, v.height - volumeHeight, 2, volumeHeight);
    for (const [row, selected] of hits) {
      const y = (row + .5) * rowHeight - scrollTop;
      if (y < -8 || y > g.height + 8) continue;
      g.ctx.fillStyle = "hsl(" + hue + " 78% 67%)";
      g.ctx.beginPath();
      g.ctx.moveTo(x, y - 6);
      g.ctx.lineTo(x + 6, y);
      g.ctx.lineTo(x, y + 6);
      g.ctx.lineTo(x - 6, y);
      g.ctx.closePath();
      g.ctx.fill();
      if (selected) {
        g.ctx.lineWidth = 2;
        g.ctx.strokeStyle = "#e5fbff";
        g.ctx.stroke();
        g.ctx.lineWidth = 1;
      }
    }
  }
  if (drag?.surface === grid && drag.lastPoint && ["Select", "Range"].includes(view.tool)) {
    const x = drag.firstPoint.x;
    const y = drag.firstPoint.y - scrollTop;
    const width = drag.lastPoint.x - x;
    const height = drag.lastPoint.y - drag.firstPoint.y;
    if ((view.tool === "Range" || !drag.hit) && Math.hypot(width, height) > 3) {
      g.ctx.fillStyle = "#74d3e02f";
      g.ctx.fillRect(x, y, width, height);
      g.ctx.strokeStyle = "#87dbe7";
      g.ctx.strokeRect(x, y, width, height);
    }
  }
  if (drag?.surface === controller && drag.lastPoint && view.tool === "Ramp") {
    const a = drag.firstPoint;
    const b = drag.lastPoint;
    line(v.ctx, "#a5feff", a.x, a.y, b.x, b.y);
  }
  r.ctx.fillStyle = "#192532";
  r.ctx.fillRect(0, 0, r.width, r.height);
  waveform(r.ctx, r.width, (r.height + 20) / 2, (r.height - 21) / 2, start, r.width / zoom);
  r.ctx.fillStyle = "#1f2b3b";
  r.ctx.fillRect(0, 0, r.width, 21);
  r.ctx.font = "11px Consolas, monospace";
  const rulerSteps = [25, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 30000, 60000];
  const labelStep = rulerSteps.find((step) => step * zoom >= 76) ?? 120000;
  for (let time = Math.ceil(start / labelStep) * labelStep; time < end; time += labelStep) {
    const x = xOf(time);
    line(r.ctx, "#607189", x, 3, x, 21);
    r.ctx.fillStyle = "#c6d3e4";
    r.ctx.fillText(clock(time), x + 5, 14);
  }
  if (view.cycle) {
    r.ctx.fillStyle = "#4eafbb46";
    r.ctx.fillRect(xOf(view.left), 0, xOf(view.right) - xOf(view.left), 20);
  }
  for (const time of [view.left, view.right]) {
    line(r.ctx, "#8bcaac", xOf(time), 0, xOf(time), r.height);
  }
  line(g.ctx, "#64e1e8", xOf(view.position), 0, xOf(view.position), g.height);
  line(r.ctx, "#64e1e8", xOf(view.position), 0, xOf(view.position), r.height);
  line(v.ctx, "#64e1e8", xOf(view.position), 0, xOf(view.position), v.height);
  o.ctx.fillStyle = "#192532";
  o.ctx.fillRect(0, 0, o.width, o.height);
  waveform(o.ctx, o.width, o.height / 2, o.height / 2 - 2, 0, view.duration);
  if (view.duration > 0) {
    const overviewX = start * o.width / view.duration;
    const overviewWidth = (g.width / zoom) * o.width / view.duration;
    o.ctx.fillStyle = "#72e3ed3b";
    o.ctx.fillRect(overviewX, 0, overviewWidth, o.height);
    o.ctx.strokeStyle = "#6ebdc7";
    o.ctx.strokeRect(overviewX, .5, overviewWidth, o.height - 1);
    const playX = view.position * o.width / view.duration;
    line(o.ctx, "#a5feff", playX, 0, playX, o.height);
  }
}

function pointer(event, phase) {
  const rect = drag.surface.getBoundingClientRect();
  const x = event.clientX - rect.x;
  const valueLane = drag.surface === controller;
  const y = event.clientY - rect.y + (valueLane ? 0 : $("scroll").scrollTop);
  const row = valueLane ? view.activeLane : Math.floor(y / scene.rowHeight);
  let ms = drag.start + x / drag.zoom;
  if (phase === "down" && !valueLane && ["Select", "Erase", "Mute", "Grab"].includes(view.tool)) {
    let best = 6.5 / drag.zoom;
    for (const [time, , , hits] of scene.moments) {
      if (Math.abs(time - ms) < best && hits.some(([lane]) => row === lane)) {
        best = Math.abs(time - ms);
        drag.hit = time;
      }
    }
    if (drag.hit != null) ms = drag.hit;
  }
  return {
    kind: "pointer", phase, ms, row, x, y, value: valueLane ? limit((1 - y / rect.height) * 100, 0, 100) : null,
    shift: event.shiftKey, ctrl: event.ctrlKey || event.metaKey, alt: event.altKey,
  };
}

for (const surface of [grid, controller]) {
  surface.addEventListener("pointerdown", (event) => {
    if (!scene || view.busy || drag || event.button !== 0) return;
    event.preventDefault();
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    surface.setPointerCapture(event.pointerId);
    drag = { surface, id: event.pointerId, start: scene.viewStart, zoom: scene.zoom, last: 0 };
    drag.firstPoint = pointer(event, "down");
    act(drag.firstPoint);
  });
  surface.addEventListener("pointermove", (event) => {
    if (event.pointerId !== drag?.id || performance.now() - drag.last < 32 || requests > 2) return;
    drag.last = performance.now();
    drag.lastPoint = pointer(event, "move");
    act(drag.lastPoint);
  });
  for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
    surface.addEventListener(type, (event) => {
      if (event.pointerId !== drag?.id) return;
      const released = type === "pointerup";
      const end = pointer(event, released ? "up" : "cancel");
      if (released && Math.hypot(end.x - drag.firstPoint.x, end.y - drag.firstPoint.y) > 3) {
        act({ ...end, phase: "move" });
      }
      act(end);
      drag = null;
    });
  }
}
$("scroll").addEventListener("scroll", paintSoon);
new ResizeObserver(paintSoon).observe($("scroll"));
ruler.addEventListener("pointerdown", (event) => {
  if (!scene) return;
  const x = event.clientX - ruler.getBoundingClientRect().x;
  act({ kind: "seek", ms: limit(scene.viewStart + x / scene.zoom, 0, view.duration) }, false);
});
overview.addEventListener("pointerdown", (event) => {
  if (!scene) return;
  const position = (event.clientX - overview.getBoundingClientRect().x) / overview.clientWidth * view.duration;
  changeView(position - grid.clientWidth / scene.zoom / 2, scene.zoom);
});

function changeView(start, zoom) {
  const right = Math.max(0, view.duration - grid.clientWidth / zoom);
  return act({ kind: "view", start: limit(start, 0, right), zoom, row_height: scene.rowHeight });
}
grid.addEventListener("wheel", (event) => {
  if (!scene || !(event.ctrlKey || event.shiftKey)) return;
  event.preventDefault();
  if (requests > 1) return;
  const x = event.clientX - grid.getBoundingClientRect().x;
  const zoom = event.ctrlKey ? limit(scene.zoom * Math.exp(-event.deltaY * .003), .002, 3) : scene.zoom;
  const start = event.ctrlKey ? scene.viewStart + x * (1 / scene.zoom - 1 / zoom) :
    scene.viewStart + (event.deltaX || event.deltaY) / scene.zoom;
  changeView(start, zoom);
}, { passive: false });

function modal(kind, title, content, buttons, cancel = null) {
  if (modalKind === kind && dialog.open) return;
  modalKind = kind;
  modalClose = cancel;
  $("dialog-title").textContent = title;
  $("dialog-body").replaceChildren(...content);
  $("dialog-actions").replaceChildren(...buttons.map(([label, task, primary]) => {
    const button = makeButton(label, task);
    if (primary) button.className = "primary";
    return button;
  }));
  if (!dialog.open) dialog.showModal();
}
function closeModal() {
  modalKind = "";
  modalClose = null;
  dialog.close();
}
dialog.addEventListener("cancel", (event) => {
  event.preventDefault();
  if (modalClose) modalClose();
  else closeModal();
});

function paragraph(value) {
  const node = document.createElement("p");
  node.textContent = value;
  return node;
}

function renderDialogs() {
  if (view.error) {
    const dismiss = () => { closeModal(); act({ kind: "dismiss_error" }); };
    modal("error", "Editor message", [paragraph(view.error)], [["OK", dismiss, true]], dismiss);
  } else if (view.pendingOpen) {
    const resolve = (choice) => { closeModal(); act({ kind: "pending", choice }); };
    modal("pending", "Save changes before opening?", [paragraph("The open difficulty has unsaved changes.")],
      [["Cancel", () => resolve("cancel")], ["Discard", () => resolve("discard")], ["Save", () => resolve("save"), true]],
      () => resolve("cancel"));
  } else if (view.revert) {
    const cancel = () => { closeModal(); act({ kind: "cancel_revert" }); };
    modal("revert", "Revert to saved difficulty?", [paragraph("This reloads the saved difficulty from disk.")],
      [["Cancel", cancel], ["Revert", () => { closeModal(); act({ kind: "confirm_revert" }); }, true]], cancel);
  } else if (view.review) {
    const answer = (apply) => { closeModal(); act({ kind: "output", apply }); };
    const list = Array.isArray(view.review.details) ? view.review.details : [view.review.details];
    modal("review", view.review.label, [...list.filter(Boolean).map(paragraph), ...view.review.files.map(paragraph)],
      [["Cancel", () => answer(false)], ["Write files", () => answer(true), true]], () => answer(false));
  } else if (["error", "pending", "revert", "review"].includes(modalKind)) {
    closeModal();
  }
}

const open = () => run(async () => show(await invoke("pick_map")));
$("open").addEventListener("click", open);
$("home-open").addEventListener("click", open);
for (const name of ["Select", "Draw", "Erase", "Mute", "Grab", "Range", "Ramp"]) {
  $("tools").append(makeButton(name, () => act({ kind: "tool", name })));
}
for (const button of document.querySelectorAll("[data-action]")) {
  button.addEventListener("click", () => act({ kind: button.dataset.action }));
}
for (const button of document.querySelectorAll("[data-command]")) {
  button.addEventListener("click", () => command(button.dataset.command));
}
for (const button of document.querySelectorAll("[data-volume]")) {
  button.addEventListener("click", () => act({ kind: "volume", relative: true, value: +button.dataset.volume }));
}
for (const button of document.querySelectorAll("[data-locator]")) {
  button.addEventListener("click", (event) => act({ kind: "locator", end: button.dataset.locator, jump: event.shiftKey }));
}
for (const id of ["snap", "snap-mode", "beats", "divisor"]) {
  $(id).addEventListener("change", () => {
    act({ kind: "snap", on: $("snap").checked, mode: $("snap-mode").value,
      beats: limit(number("beats", scene.beats), 1, 64), divisor: limit(number("divisor", scene.divisor), 1, 64) });
  });
}
toggle("hs", "hs_mode");
toggle("protect", "protect");
toggle("cycle", "cycle");
toggle("audition", "audition");
$("difficulty").addEventListener("change", () => {
  const path = $("difficulty").value;
  $("difficulty").value = scene.path;
  act({ kind: "open", path });
});
$("zoom").addEventListener("input", () => {
  if (requests < 2) changeView(scene.viewStart, 10 ** +$("zoom").value);
});
$("zoom").addEventListener("change", () => changeView(scene.viewStart, 10 ** +$("zoom").value));
$("speed").addEventListener("change", () => act({ kind: "speed", value: +$("speed").value }));
$("insert").addEventListener("change", () => act({ kind: "insert_volume", value: limit(number("insert", 100), 0, 100) }));
$("set-volume").addEventListener("click", () => act({ kind: "volume", value: limit(number("volume", 100), 0, 100), relative: false }));

function input(type, value, min, max, step = 1) {
  const field = document.createElement("input");
  field.type = type;
  field.value = value;
  field.min = min;
  field.max = max;
  field.step = step;
  return field;
}
function labeled(label, control) {
  const node = document.createElement("label");
  node.append(label, control);
  return node;
}
$("new").addEventListener("click", () => {
  const name = input("text", "", 0, 0);
  name.placeholder = "Difficulty name";
  const submit = () => {
    if (name.value.trim()) {
      closeModal();
      act({ kind: "new_difficulty", name: name.value.trim() });
    }
  };
  name.addEventListener("keydown", (event) => { if (event.key === "Enter") submit(); });
  modal("new", "New difficulty", [labeled("Name", name)], [["Cancel", closeModal], ["Review output", submit, true]]);
  name.focus();
});
$("copy").addEventListener("click", () => {
  const paths = scene.difficulties.filter((path) => path !== scene.path);
  const checkboxes = paths.map(() => input("checkbox", "", 0, 0));
  const choices = paths.map((path, i) => labeled(nameOf(path), checkboxes[i]));
  const leniency = input("number", 5, 0, 1000);
  const submit = () => {
    const targets = paths.filter((_, i) => checkboxes[i].checked);
    if (!targets.length) return;
    closeModal();
    act({ kind: "copy", targets, leniency: limit(leniency.valueAsNumber || 0, 0, 1000) });
  };
  if (!paths.length) {
    modal("copy", "Copy hitsounds", [paragraph("There are no other difficulties in this mapset.")], [["Close", closeModal, true]]);
    return;
  }
  modal("copy", "Copy hitsounds", [...choices, labeled("Timing tolerance (ms)", leniency)],
    [["Cancel", closeModal], ["Review output", submit, true]]);
});
$("warnings").addEventListener("click", () => {
  modal("warnings", "Audio warnings", view.warnings.map(paragraph), [["Close", closeModal, true]]);
});
$("audio").addEventListener("click", () => {
  const song = input("range", view.song, 0, 1, .01);
  const main = input("range", view.main, .2, 1, .01);
  const mute = input("checkbox", "", 0, 0);
  mute.checked = view.songMuted;
  const offset = input("number", view.offset, -60000, 60000, 1);
  const gains = () => act({ kind: "gains", song: +song.value, main: +main.value }, false);
  song.addEventListener("change", gains);
  main.addEventListener("change", gains);
  mute.addEventListener("change", () => act({ kind: "song_muted", on: mute.checked }, false));
  offset.addEventListener("change", () => act({ kind: "offset", ms: offset.valueAsNumber || 0 }, false));
  modal("audio", "Audio", [labeled("Song volume", song), labeled("Master volume", main),
    labeled("Mute song", mute), labeled("Song offset (ms)", offset)], [["Done", closeModal, true]]);
});

$("help").addEventListener("click", () => {
  modal("help", "Keyboard shortcuts", [
    "Ctrl+O   Open a difficulty or mapset", "Ctrl+S   Save", "Space   Play or pause",
    "Ctrl+Z / Ctrl+Shift+Z   Undo / redo", "Ctrl+X/C/V   Cut / copy / paste",
    "Ctrl+A / Ctrl+Shift+A   Select all / deselect", "Ctrl+D   Duplicate", "Delete   Delete selected hits",
    "Shift+wheel   Scroll time", "Ctrl+wheel   Zoom around pointer",
  ].map(paragraph), [["Close", closeModal, true]]);
});
document.addEventListener("keydown", (event) => {
  if (dialog.open || event.target.matches("input, select, textarea")) return;
  const key = event.key.toLowerCase();
  if (key === "f1") {
    event.preventDefault();
    $("help").click();
    return;
  }
  const ctrl = event.ctrlKey || event.metaKey;
  if (ctrl && key === "o") {
    event.preventDefault();
    if (!view.busy) open();
    return;
  }
  if (!scene || view.busy) return;
  let id;
  if (ctrl) id = { z: event.shiftKey ? "redo" : "undo", y: "redo", a: event.shiftKey ? "deselect" : "selectAll",
    c: "copy", x: "cut", v: "paste", d: "duplicate", s: "save" }[key];
  else if (!event.altKey) id = { " ": "play", delete: "delete", backspace: "delete", escape: "deselect", q: "quantize" }[key];
  if (id) {
    if (key === " " && event.target.matches("button")) return;
    event.preventDefault();
    if (id === "play" || id === "save") act({ kind: id });
    else command(id);
  }
});

window.__TAURI__.event.listen("editor-close-requested", ({ payload }) => {
  if (view.fault) return;
  if (view.busy) {
    modal("close-busy", "Job in progress", [paragraph(payload)], [["Keep editor open", closeModal, true]]);
    return;
  }
  modal("close-editor", "Quit dawsu!?", [paragraph(payload)], [
    ["Cancel", closeModal],
    ["Discard changes", () => run(() => invoke("close_editor", { discard: true }))],
    ["Save and quit", async () => {
      closeModal();
      await act({ kind: "save" });
      if (view.busy) quitAfterSave = true;
      else if (!view.dirty) run(() => invoke("close_editor", { discard: false }));
    }, true],
  ]);
});

let quitAfterSave = false;
setInterval(async () => {
  const now = performance.now();
  if (view.fault || requests || refreshing || now - lastRefresh < (view.playing || view.busy ? 85 : 1000)) return;
  refreshing = true;
  lastRefresh = now;
  await run(() => update(null, false));
  refreshing = false;
  if (quitAfterSave && !view.busy) {
    quitAfterSave = false;
    if (!view.dirty && !view.error) run(() => invoke("close_editor", { discard: false }));
  }
}, 85);
act(null);
