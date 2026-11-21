<script>
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import {
    BAND_PRESETS,
    BEAT_DIVISORS,
    NODE_HEADER_H,
    NODE_WIDTH,
    RECIPE_OPTIONS,
    SOCKET_ROW_H,
    autoLayoutNodes,
    extractSimpleFlowModel,
    fitViewBox,
    formatParamValue,
    inputSocketPos,
    linkPath,
    nodeBadgeText,
    nodeHeight,
    nodeMeta,
    nodeSubtitle,
    outputSocketPos,
    parseParamInput,
    sensitivityToThreshold,
    socketColour,
    sparklinePoints,
    stageAccentKey,
    thresholdToSensitivity,
    visibleViewerOverlay,
  } from "../lib/node-graph.js";

  let { initialView = null } = $props();

  function takeInitial() {
    return initialView;
  }

  let view = $state.raw(takeInitial());
  let mode = $state("simple"); // "simple" | "raw"
  let error = $state("");
  let statusNotice = $state("");
  let recipeChoice = $state("classic");
  let addMenuOpen = $state(false);
  let checklistOpen = $state(false);
  let svgEl = $state(null);
  let previewCanvas = $state(null);
  let vb = $state({ x: -40, y: -40, w: 1200, h: 620 });
  let fitted = false;
  let drag = $state(null);
  let dragPos = $state(null);
  let curveDrag = $state(null);
  let closed = false;
  let pollTimer = null;

  const flow = $derived(extractSimpleFlowModel(view));
  const selectedNode = $derived.by(() => {
    if (!view || !view.selectedNodes?.length) return null;
    const first = view.selectedNodes[0];
    return view.nodes.find((n) => n.id === first) ?? null;
  });

  async function loadView() {
    try {
      const next = await invoke("node_graph_view");
      view = next;
      error = "";
      if (!fitted && next?.nodes?.length) {
        vb = fitViewBox(next.nodes);
        fitted = true;
      }
      schedulePoll();
    } catch (err) {
      error = String(err);
    }
  }

  function schedulePoll() {
    if (closed) return;
    clearTimeout(pollTimer);
    const delay = view?.status === "running" ? 150 : 1000;
    pollTimer = setTimeout(loadView, delay);
  }

  onMount(() => {
    loadView();
  });

  onDestroy(() => {
    closed = true;
    clearTimeout(pollTimer);
  });

  async function dispatch(action, notice = "") {
    try {
      error = "";
      if (notice) statusNotice = notice;
      const next = await invoke("node_graph_action", { action });
      view = next;
      schedulePoll();
      return next;
    } catch (err) {
      error = String(err);
      return null;
    }
  }

  async function loadRecipe(name) {
    recipeChoice = name;
    const next = await dispatch({ kind: "load_preset", name });
    if (next?.nodes) vb = fitViewBox(next.nodes);
  }

  async function pickOpenFile() {
    try {
      const next = await invoke("node_graph_pick_open");
      if (next) {
        view = next;
        vb = fitViewBox(next.nodes ?? []);
        statusNotice = "Loaded graph recipe.";
      }
    } catch (err) {
      error = String(err);
    }
  }

  async function pickSaveFile() {
    try {
      const saved = await invoke("node_graph_pick_save");
      if (saved) statusNotice = "Saved recipe (.dawsugraph).";
    } catch (err) {
      error = String(err);
    }
  }

  async function copyJsonClipboard() {
    try {
      const json = await invoke("node_graph_export_json");
      if (navigator?.clipboard?.writeText) {
        await navigator.clipboard.writeText(json);
        statusNotice = "Copied graph JSON to clipboard.";
      }
    } catch (err) {
      error = String(err);
    }
  }

  async function browseStemAudio(nodeId) {
    try {
      const picked = await invoke("node_graph_pick_audio");
      if (picked) {
        await dispatch({ kind: "set_param", nodeId, param: "path", value: String(picked) }, "Stem audio updated.");
      }
    } catch (err) {
      error = String(err);
    }
  }

  async function switchToStemFlow() {
    if (flow?.fileNode) {
      await browseStemAudio(flow.fileNode.id);
    } else {
      await loadRecipe("drum_stem");
    }
  }

  async function setDivisorBit(snapNode, divisor) {
    const cur = Array.isArray(snapNode.params?.divisors) ? snapNode.params.divisors : [4];
    let next;
    if (cur.includes(divisor)) {
      next = cur.filter((d) => d !== divisor);
    } else {
      next = [...cur, divisor].sort((a, b) => a - b);
    }
    if (!next.length) next = [divisor];
    await dispatch({ kind: "set_param", nodeId: snapNode.id, param: "divisors", value: next });
  }

  async function setBandPreset(fluxNode, preset) {
    await dispatch({ kind: "set_param", nodeId: fluxNode.id, param: "loHz", value: preset.loHz });
    await dispatch({ kind: "set_param", nodeId: fluxNode.id, param: "hiHz", value: preset.hiHz });
  }

  async function runRange(useLocatorRange) {
    await dispatch({ kind: "set_range_mode", useLocatorRange });
    await dispatch({ kind: "run" }, useLocatorRange ? "Testing L–R loop…" : "Analysing whole song…");
  }

  function curveYForThreshold(thr) {
    const t = Math.max(0.5, Math.min(2.5, Number(thr) || 1.5));
    const norm = (t - 0.5) / 2.0; // 0 = loose (bottom), 1 = strict (top)
    return 72 - norm * 58;
  }

  function thresholdFromPointer(svgNode, clientY) {
    const rect = svgNode.getBoundingClientRect();
    const relY = Math.max(10, Math.min(72, ((clientY - rect.top) / Math.max(1, rect.height)) * 82));
    const norm = (72 - relY) / 58;
    const pct = Math.round((1 - norm) * 100);
    return { pct, threshold: sensitivityToThreshold(pct) };
  }

  function onCurvePointerDown(event, det) {
    event.stopPropagation();
    const el = event.currentTarget;
    el.setPointerCapture?.(event.pointerId);
    const res = thresholdFromPointer(el, event.clientY);
    curveDrag = { nodeId: det.peaks.id, pct: res.pct, threshold: res.threshold };
  }

  function onCurvePointerMove(event, det) {
    if (!curveDrag || curveDrag.nodeId !== det.peaks.id) return;
    const res = thresholdFromPointer(event.currentTarget, event.clientY);
    curveDrag = { nodeId: det.peaks.id, pct: res.pct, threshold: res.threshold };
  }

  async function onCurvePointerUp(event, det) {
    if (!curveDrag || curveDrag.nodeId !== det.peaks.id) return;
    const finalThreshold = curveDrag.threshold;
    curveDrag = null;
    await dispatch({ kind: "set_param", nodeId: det.peaks.id, param: "threshold", value: finalThreshold });
  }

  function detectorThreshold(det) {
    if (curveDrag && curveDrag.nodeId === det.peaks.id) return curveDrag.threshold;
    return Number(det.peaks.params?.threshold ?? 1.5);
  }

  function detectorSensitivity(det) {
    if (curveDrag && curveDrag.nodeId === det.peaks.id) return curveDrag.pct;
    return det.sensitivity;
  }

  // Raw canvas helpers
  function nodeXY(node) {
    if (dragPos && dragPos.id === node.id) return { x: dragPos.x, y: dragPos.y };
    return { x: node.x, y: node.y };
  }

  function svgPoint(event) {
    const rect = svgEl?.getBoundingClientRect();
    if (!rect || !rect.width || !rect.height) return { x: 0, y: 0 };
    return {
      x: vb.x + ((event.clientX - rect.left) / rect.width) * vb.w,
      y: vb.y + ((event.clientY - rect.top) / rect.height) * vb.h,
    };
  }

  function onSvgPointerDown(event) {
    if (event.target !== svgEl) return;
    drag = { kind: "pan", startX: event.clientX, startY: event.clientY, origVb: { ...vb }, moved: false };
  }

  function onNodePointerDown(event, node) {
    if (event.button !== 0) return;
    event.stopPropagation();
    const pos = nodeXY(node);
    const pt = svgPoint(event);
    drag = { kind: "node", id: node.id, startX: pt.x, startY: pt.y, origX: pos.x, origY: pos.y, moved: false };
    dispatch({ kind: "select_nodes", ids: [node.id] });
  }

  function onOutSocketDown(event, node, socketId) {
    if (event.button !== 0) return;
    event.stopPropagation();
    const pos = nodeXY(node);
    const start = outputSocketPos(node, socketId, pos.x - node.x, pos.y - node.y);
    drag = { kind: "wire", fromNode: node.id, fromSocket: socketId, fromPos: start, curPos: svgPoint(event) };
  }

  function onInSocketUp(event, node, socketId) {
    if (!drag || drag.kind !== "wire") return;
    event.stopPropagation();
    const wire = drag;
    drag = null;
    if (wire.fromNode !== node.id) {
      dispatch({
        kind: "connect",
        fromNode: wire.fromNode,
        fromSocket: wire.fromSocket,
        toNode: node.id,
        toSocket: socketId,
      });
    }
  }

  function onSvgPointerMove(event) {
    if (!drag) return;
    if (drag.kind === "pan") {
      const rect = svgEl?.getBoundingClientRect();
      if (!rect || !rect.width || !rect.height) return;
      const dx = ((event.clientX - drag.startX) / rect.width) * drag.origVb.w;
      const dy = ((event.clientY - drag.startY) / rect.height) * drag.origVb.h;
      if (Math.hypot(dx, dy) > 2) drag.moved = true;
      vb = { ...drag.origVb, x: drag.origVb.x - dx, y: drag.origVb.y - dy };
    } else if (drag.kind === "node") {
      const pt = svgPoint(event);
      const nx = drag.origX + (pt.x - drag.startX);
      const ny = drag.origY + (pt.y - drag.startY);
      if (Math.hypot(nx - drag.origX, ny - drag.origY) > 1.5) drag.moved = true;
      dragPos = { id: drag.id, x: Math.round(nx), y: Math.round(ny) };
    } else if (drag.kind === "wire") {
      drag = { ...drag, curPos: svgPoint(event) };
    }
  }

  function onSvgPointerUp() {
    if (!drag) return;
    const active = drag;
    drag = null;
    if (active.kind === "pan" && !active.moved) {
      dispatch({ kind: "select_nodes", ids: [] });
    } else if (active.kind === "node" && active.moved && dragPos) {
      const pos = dragPos;
      dragPos = null;
      dispatch({ kind: "move_nodes", positions: [pos] });
    } else {
      dragPos = null;
    }
  }

  function onSvgWheel(event) {
    event.preventDefault();
    const factor = event.deltaY < 0 ? 0.88 : 1.14;
    const pt = svgPoint(event);
    const nextW = Math.max(360, Math.min(8000, vb.w * factor));
    const nextH = Math.max(200, Math.min(4500, vb.h * factor));
    const rx = (pt.x - vb.x) / vb.w;
    const ry = (pt.y - vb.y) / vb.h;
    vb = { x: pt.x - rx * nextW, y: pt.y - ry * nextH, w: nextW, h: nextH };
  }

  function renderedLinkPath(link) {
    if (!view) return "";
    const fromNode = view.nodes.find((n) => n.id === link.fromNode);
    const toNode = view.nodes.find((n) => n.id === link.toNode);
    if (!fromNode || !toNode) return "";
    const fp = nodeXY(fromNode);
    const tp = nodeXY(toNode);
    return linkPath(
      outputSocketPos(fromNode, link.fromSocket, fp.x - fromNode.x, fp.y - fromNode.y),
      inputSocketPos(toNode, link.toSocket, tp.x - toNode.x, tp.y - toNode.y),
    );
  }

  async function autoArrange() {
    if (!view?.nodes?.length) return;
    const positions = autoLayoutNodes(view.nodes, view.links ?? []);
    const next = await dispatch({ kind: "move_nodes", positions });
    if (next?.nodes) vb = fitViewBox(next.nodes);
  }

  function handleWindowKeyDown(event) {
    if (mode !== "raw" || event.defaultPrevented) return;
    if (event.target?.closest?.("input, select, textarea")) return;
    const mod = event.ctrlKey || event.metaKey;
    if (mod && event.key.toLowerCase() === "z") {
      event.preventDefault();
      dispatch({ kind: event.shiftKey ? "redo" : "undo" });
    } else if (mod && event.key.toLowerCase() === "y") {
      event.preventDefault();
      dispatch({ kind: "redo" });
    } else if (mod && event.key.toLowerCase() === "d" && selectedNode) {
      event.preventDefault();
      dispatch({ kind: "duplicate_selected", offsetX: 30, offsetY: 30 });
    } else if ((event.key === "Delete" || event.key === "Backspace") && view?.selectedNodes?.length) {
      event.preventDefault();
      dispatch({ kind: "remove_nodes", ids: view.selectedNodes });
    }
  }

  function paramMeta(typeId, paramId) {
    return nodeMeta(typeId).params?.[paramId] ?? { label: paramId, control: "auto", advanced: false };
  }

  function isAdvancedParam(typeId, paramId) {
    return !!nodeMeta(typeId).params?.[paramId]?.advanced;
  }

  function handleParamChange(node, spec, event) {
    const parsed = parseParamInput(spec, event.currentTarget.value, event.currentTarget.checked);
    if (!parsed.ok) {
      error = parsed.error;
      return;
    }
    dispatch({ kind: "set_param", nodeId: node.id, param: spec.id, value: parsed.value });
  }

  function isViewerEndpoint(nodeId, socketId) {
    return view?.viewerSocket?.node === nodeId && view?.viewerSocket?.socket === socketId;
  }

  $effect(() => {
    const canvas = previewCanvas;
    const curView = view;
    if (!canvas || !curView) return;
    const dpr = globalThis.devicePixelRatio || 1;
    const w = canvas.clientWidth || 1000;
    const h = canvas.clientHeight || 78;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    ctx.fillStyle = "#0c0c0c";
    ctx.fillRect(0, 0, w, h);

    const start = curView.rangeFromMs ?? 0;
    const end = Math.max(start + 1, curView.rangeToMs ?? 1000);
    const xOf = (ms) => ((ms - start) / (end - start)) * w;

    const overlay = visibleViewerOverlay(curView.viewer, start, end, xOf, 8, h - 24);
    if (overlay?.kind === "curve" && overlay.points.length) {
      ctx.strokeStyle = "#f1ad67";
      ctx.lineWidth = 1.6;
      ctx.beginPath();
      for (const [i, [px, py]] of overlay.points.entries()) {
        if (i === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      }
      ctx.stroke();
    } else if (overlay?.kind === "events" && overlay.markers.length) {
      ctx.strokeStyle = "#62d6cf";
      ctx.fillStyle = "#62d6cf";
      for (const m of overlay.markers) {
        ctx.beginPath();
        ctx.moveTo(m.x, 6);
        ctx.lineTo(m.x, h - 18);
        ctx.stroke();
      }
    }

    const proposals = curView.batch?.proposals ?? [];
    for (const p of proposals) {
      if (!p.checked) continue;
      const px = xOf(p.timeMs);
      ctx.fillStyle = /snare|clap/i.test(p.slot) ? "#e28dd1" : /hat/i.test(p.slot) ? "#9c9cfc" : "#f1ad67";
      const py = h - 10;
      ctx.beginPath();
      ctx.moveTo(px, py - 4);
      ctx.lineTo(px + 4, py);
      ctx.lineTo(px, py + 4);
      ctx.lineTo(px - 4, py);
      ctx.closePath();
      ctx.fill();
    }
  });
</script>

<svelte:window onkeydown={handleWindowKeyDown} />

<div class="ng-win">
  <header class="ng-header">
    <div class="ng-brand">
      <span class="ng-diamond" aria-hidden="true"></span>
      <span>Node Graph</span>
    </div>

    {#if view?.hasDocument}
      <span class="ng-map-chip">{view.mapTitle || "Open map"}{view.mapVersion ? " [" + view.mapVersion + "]" : ""}</span>
    {:else}
      <span class="ng-map-chip">No difficulty open</span>
    {/if}

    <div class="ng-recipe-box">
      <span>Recipe:</span>
      <select aria-label="Recipe" value={recipeChoice} onchange={(e) => loadRecipe(e.currentTarget.value)}>
        {#each RECIPE_OPTIONS as opt (opt.id)}
          <option value={opt.id}>{opt.label}</option>
        {/each}
      </select>
    </div>

    <button type="button" class="ng-btn" onclick={pickOpenFile}>Load…</button>
    <button type="button" class="ng-btn" onclick={pickSaveFile}>Save…</button>

    <div class="ng-spacer"></div>

    <div class="ng-seg" role="group" aria-label="View mode">
      <button type="button" class="ng-seg-btn" aria-pressed={mode === "simple"} onclick={() => { mode = "simple"; addMenuOpen = false; }}>
        Simple Flow
      </button>
      <button type="button" class="ng-seg-btn" aria-pressed={mode === "raw"} onclick={() => { mode = "raw"; if (view?.nodes?.length) vb = fitViewBox(view.nodes); }}>
        Raw Blocks ({view?.nodes?.length ?? 0})
      </button>
    </div>

    <button type="button" class="ng-btn ng-btn-teal" onclick={() => invoke("show_in_editor")}>Show in Editor</button>
    <button type="button" class="ng-btn" onclick={() => invoke("close_node_graph_window")}>Close</button>
  </header>

  {#if error || view?.error}
    <div class="ng-banner-err">{error || view.error}</div>
  {/if}

  {#if mode === "simple" && view && flow}
    <div class="ng-flow">
      <!-- 1. SOURCE AUDIO -->
      <div class="ng-col">
        <section class="ng-card">
          <div class="ng-stage-head ng-stage-source">
            <span>1 · SOURCE AUDIO</span>
          </div>

          <div class="ng-seg" role="group" aria-label="Audio source">
            <button
              type="button"
              class="ng-seg-btn"
              aria-pressed={!flow.isStem}
              onclick={() => { if (flow.isStem) loadRecipe("classic"); }}
            >
              Song Audio
            </button>
            <button
              type="button"
              class="ng-seg-btn"
              aria-pressed={flow.isStem}
              onclick={switchToStemFlow}
            >
              Stem File…
            </button>
          </div>

          {#if flow.fileNode}
            <div class="ng-row">
              <span class="ng-muted">{nodeSubtitle(flow.fileNode, view.slots)}</span>
              <button type="button" class="ng-btn" onclick={() => browseStemAudio(flow.fileNode.id)}>Browse…</button>
            </div>
          {/if}

          <div class="ng-label">Analyse Range</div>
          <div class="ng-chips">
            <button
              type="button"
              class={view.useLocatorRange ? "ng-btn" : "ng-btn ng-btn-teal"}
              onclick={() => dispatch({ kind: "set_range_mode", useLocatorRange: false })}
            >
              Whole Song
            </button>
            <button
              type="button"
              class={view.useLocatorRange ? "ng-btn ng-btn-teal" : "ng-btn"}
              disabled={!view.hasLocatorRange}
              onclick={() => dispatch({ kind: "set_range_mode", useLocatorRange: true })}
            >
              L–R Loop
            </button>
          </div>

          {#if flow.hpssNode}
            <button
              type="button"
              class="ng-switch-card"
              onclick={() => dispatch({ kind: "set_muted", nodeId: flow.hpssNode.id, muted: !flow.hpssNode.muted })}
            >
              <div class="ng-row">
                <strong>Isolate Drums</strong>
                <span class={flow.hpssNode.muted ? "ng-pill-off" : "ng-pill-on"}>
                  {flow.hpssNode.muted ? "OFF" : "ON"}
                </span>
              </div>
              <p class="ng-muted">Suppresses vocals &amp; guitars before picking hits.</p>
            </button>
          {/if}

          <button type="button" class="ng-btn" onclick={() => { mode = "raw"; vb = fitViewBox(view.nodes); }}>
            ▸ Expand Raw Blocks ({view.nodes.length})
          </button>
        </section>
      </div>

      <!-- 2. DETECTORS -->
      <div class="ng-col ng-col-wide">
        {#if flow.detectors.length === 0}
          <section class="ng-card">
            <div class="ng-stage-head ng-stage-detect"><span>2 · DETECTORS</span></div>
            <p class="ng-muted">This custom graph has no standard peak detectors. Switch to Raw Blocks or pick a starter recipe above.</p>
            <button type="button" class="ng-btn ng-btn-teal" onclick={() => { mode = "raw"; vb = fitViewBox(view.nodes); }}>
              Open Raw Blocks Editor
            </button>
          </section>
        {:else}
          {#each flow.detectors as det (det.peaks.id)}
            {@const sensPct = detectorSensitivity(det)}
            {@const thrY = curveYForThreshold(detectorThreshold(det))}
            {@const sparkPts = det.curveSource?.summary?.kind === "curve" ? sparklinePoints(det.curveSource.summary.sparkline, 300, 68, 8) : ""}
            <section class="ng-card">
              <div class={"ng-stage-head ng-stage-" + det.accent}>
                <span>2 · {det.title.toUpperCase()}</span>
                <span class={"ng-badge ng-badge-" + det.accent}>
                  {det.foundCount != null ? det.foundCount + " found" : "Not run"}
                </span>
              </div>

              {#if det.flux}
                <div class="ng-chips">
                  {#each BAND_PRESETS as band (band.id)}
                    {@const activeBand = Number(det.flux.params?.loHz) === band.loHz && Number(det.flux.params?.hiHz) === band.hiHz}
                    <button
                      type="button"
                      class={activeBand ? "ng-btn ng-btn-teal" : "ng-btn"}
                      onclick={() => setBandPreset(det.flux, band)}
                    >
                      {band.label}
                    </button>
                  {/each}
                </div>
              {/if}

              <div class="ng-well">
                <svg
                  class="ng-curve-svg"
                  viewBox="0 0 300 82"
                  role="slider"
                  tabindex="0"
                  aria-label={det.title + " Sensitivity"}
                  aria-valuemin="0"
                  aria-valuemax="100"
                  aria-valuenow={sensPct}
                  onpointerdown={(e) => onCurvePointerDown(e, det)}
                  onpointermove={(e) => onCurvePointerMove(e, det)}
                  onpointerup={(e) => onCurvePointerUp(e, det)}
                >
                  {#if sparkPts}
                    <polyline fill="none" stroke={det.accent === "clap" ? "#e28dd1" : det.accent === "hat" ? "#9c9cfc" : "#f1ad67"} stroke-width="2" points={sparkPts}></polyline>
                  {/if}
                  <line x1="0" y1={thrY} x2="300" y2={thrY} stroke="#efc16a" stroke-width="1.5" stroke-dasharray="5 4"></line>
                  <rect x="252" y={Math.max(2, thrY - 9)} width="44" height="17" rx="4" fill="#efc16a"></rect>
                  <text x="274" y={Math.max(13, thrY + 3)} text-anchor="middle" font-size="10" font-weight="700" fill="#111111">{sensPct}%</text>
                  <text x="6" y="12" font-size="9" fill="#a3a3a3">Drag line up/down · Sensitivity {sensPct}%</text>
                </svg>
              </div>

              <div class="ng-row">
                <span class="ng-label">Sensitivity: {sensPct}% · Gap: {Math.round(det.minGapMs)} ms</span>
                <div class="ng-chips">
                  {#if det.bleed}
                    <button
                      type="button"
                      class={det.bleed.muted ? "ng-btn" : "ng-btn ng-btn-teal"}
                      onclick={() => dispatch({ kind: "set_muted", nodeId: det.bleed.id, muted: !det.bleed.muted })}
                    >
                      Bleed guard: {det.bleed.muted ? "Off" : "Auto"}
                    </button>
                  {/if}
                  {#if det.curveSource}
                    {@const previewing = isViewerEndpoint(det.curveSource.id, "curve")}
                    <button
                      type="button"
                      class={previewing ? "ng-btn ng-btn-teal" : "ng-btn"}
                      onclick={() => dispatch({
                        kind: "set_viewer_target",
                        node: previewing ? null : det.curveSource.id,
                        socket: previewing ? null : "curve",
                      })}
                    >
                      {previewing ? "● Previewing" : "Preview"}
                    </button>
                  {/if}
                  <button
                    type="button"
                    class="ng-btn"
                    onclick={() => {
                      mode = "raw";
                      dispatch({ kind: "select_nodes", ids: [det.peaks.id] });
                      vb = fitViewBox(view.nodes);
                    }}
                  >
                    Edit block ▸
                  </button>
                </div>
              </div>
            </section>
          {/each}
        {/if}
      </div>

      <!-- 3. BEAT SNAP -->
      <div class="ng-col">
        <section class="ng-card">
          <div class="ng-stage-head ng-stage-detect">
            <span>3 · BEAT SNAP</span>
          </div>

          {#if flow.snapNode}
            {@const divs = Array.isArray(flow.snapNode.params?.divisors) ? flow.snapNode.params.divisors : [4]}
            <div class="ng-label">Allowed Grid Divisions</div>
            <div class="ng-chips">
              {#each BEAT_DIVISORS as d (d)}
                <button
                  type="button"
                  class={divs.includes(d) ? "ng-btn ng-btn-primary" : "ng-btn"}
                  onclick={() => setDivisorBit(flow.snapNode, d)}
                >
                  1/{d}
                </button>
              {/each}
            </div>

            <label class="ng-label">
              Snap window: ±{Math.round(Number(flow.snapNode.params?.toleranceMs ?? 20))} ms
              <input
                class="ng-slider"
                type="range"
                min="5"
                max="50"
                step="1"
                value={Number(flow.snapNode.params?.toleranceMs ?? 20)}
                onchange={(e) => dispatch({ kind: "set_param", nodeId: flow.snapNode.id, param: "toleranceMs", value: Number(e.currentTarget.value) })}
              />
            </label>

            <button
              type="button"
              class={flow.snapNode.params?.measureLag !== false ? "ng-btn ng-btn-teal" : "ng-btn"}
              onclick={() => dispatch({ kind: "set_param", nodeId: flow.snapNode.id, param: "measureLag", value: !(flow.snapNode.params?.measureLag !== false) })}
            >
              {flow.snapNode.params?.measureLag !== false ? "✓ Auto lag align" : "○ Auto lag align off"}
            </button>

            <button
              type="button"
              class={flow.snapNode.params?.dropOffGrid !== false ? "ng-btn ng-btn-teal" : "ng-btn"}
              onclick={() => dispatch({ kind: "set_param", nodeId: flow.snapNode.id, param: "dropOffGrid", value: !(flow.snapNode.params?.dropOffGrid !== false) })}
            >
              {flow.snapNode.params?.dropOffGrid !== false ? "✓ Drop off-grid noise" : "○ Keep off-grid hits"}
            </button>
          {:else}
            <p class="ng-muted">No Beat Snap block in this graph.</p>
          {/if}
        </section>
      </div>

      <!-- 4. HITSOUND LANES -->
      <div class="ng-col">
        <section class="ng-card">
          <div class="ng-stage-head ng-stage-output">
            <span>4 · HITSOUND LANES</span>
          </div>

          {#if flow.slotOutputs.length === 0}
            <p class="ng-muted">Add an Output Lane block in Raw Blocks to route hits onto your beatmap.</p>
          {:else}
            {#each flow.slotOutputs as slot (slot.slot)}
              <div class="ng-slot-box">
                <div class="ng-row">
                  <strong>{slot.slot} slot</strong>
                  {#if slot.hitCount != null}
                    <span class="ng-badge ng-badge-accent">{slot.hitCount} hits</span>
                  {/if}
                </div>
                <select
                  class="ng-select"
                  aria-label={"Lane for slot " + slot.slot}
                  value={slot.boundLaneKey ?? ""}
                  onchange={(e) => dispatch({ kind: "set_binding", slot: slot.slot, laneKey: e.currentTarget.value || null })}
                >
                  <option value="">Choose hitsound lane…</option>
                  {#each view.lanes as lane (lane.key)}
                    <option value={lane.key}>{lane.name}{lane.hidden ? " (hidden)" : ""}</option>
                  {/each}
                </select>
              </div>
            {/each}
          {/if}

          <p class="ng-muted">Changing lanes remaps hits immediately without re-analysing audio.</p>
        </section>
      </div>
    </div>
  {:else if view}
    <!-- CONCEPT B: RAW BLOCKS PATCHBAY -->
    <div class="ng-raw-subbar">
      <button type="button" class="ng-btn ng-btn-teal" onclick={() => { addMenuOpen = !addMenuOpen; }}>
        + Add Block ▾
      </button>
      <button type="button" class="ng-btn" onclick={autoArrange}>Auto-arrange</button>
      <button type="button" class="ng-btn" onclick={() => { vb = fitViewBox(view.nodes); }}>Fit</button>
      <button type="button" class="ng-btn" disabled={!view.canUndo} onclick={() => dispatch({ kind: "undo" })}>Undo</button>
      <button type="button" class="ng-btn" disabled={!view.canRedo} onclick={() => dispatch({ kind: "redo" })}>Redo</button>
      <button type="button" class="ng-btn" onclick={copyJsonClipboard}>Copy JSON</button>

      <div class="ng-spacer"></div>
      <span class="ng-muted">Slots:</span>
      {#each view.slots as slot (slot.slot)}
        <select
          class="ng-select"
          aria-label={"Lane for slot " + slot.slot}
          value={slot.boundLaneKey ?? ""}
          onchange={(e) => dispatch({ kind: "set_binding", slot: slot.slot, laneKey: e.currentTarget.value || null })}
        >
          <option value="">{slot.slot} → Unbound</option>
          {#each view.lanes as lane (lane.key)}
            <option value={lane.key}>{slot.slot} → {lane.name}</option>
          {/each}
        </select>
      {/each}
    </div>

    <div class="ng-raw-split">
      <div class="ng-svg-wrap" role="region" aria-label="Node graph patchbay canvas">
        {#if addMenuOpen}
          <div class="ng-add-panel">
            <div class="ng-row">
              <strong>Add a Processing Block</strong>
              <button type="button" class="ng-btn" onclick={() => { addMenuOpen = false; }}>Close</button>
            </div>
            {#each view.catalog as item (item.typeId)}
              {@const meta = nodeMeta(item.typeId)}
              <button
                type="button"
                class="ng-add-item"
                onclick={() => {
                  addMenuOpen = false;
                  dispatch({
                    kind: "add_node",
                    typeId: item.typeId,
                    x: Math.round(vb.x + vb.w * 0.4),
                    y: Math.round(vb.y + vb.h * 0.4),
                  });
                }}
              >
                <div><strong>{meta.title}</strong> <span class="ng-muted">({meta.stage})</span></div>
                <div class="ng-muted">{meta.description}</div>
              </button>
            {/each}
          </div>
        {/if}

        <svg
          bind:this={svgEl}
          class="ng-svg"
          viewBox={`${vb.x} ${vb.y} ${vb.w} ${vb.h}`}
          onpointerdown={onSvgPointerDown}
          onpointermove={onSvgPointerMove}
          onpointerup={onSvgPointerUp}
          onwheel={onSvgWheel}
          role="application"
          aria-label="Node patchbay canvas"
        >
          {#each view.links as link (link.fromNode + "." + link.fromSocket + "->" + link.toNode + "." + link.toSocket)}
            <path
              class="ng-wire"
              d={renderedLinkPath(link)}
              stroke={socketColour(link.socketType)}
              role="button"
              tabindex="0"
              aria-label={`Disconnect ${link.fromNode}.${link.fromSocket} from ${link.toNode}.${link.toSocket}`}
              onclick={(e) => {
                e.stopPropagation();
                dispatch({
                  kind: "disconnect",
                  fromNode: link.fromNode,
                  fromSocket: link.fromSocket,
                  toNode: link.toNode,
                  toSocket: link.toSocket,
                });
              }}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === "Delete" || e.key === "Backspace") {
                  dispatch({
                    kind: "disconnect",
                    fromNode: link.fromNode,
                    fromSocket: link.fromSocket,
                    toNode: link.toNode,
                    toSocket: link.toSocket,
                  });
                }
              }}
            >
              <title>Click to disconnect</title>
            </path>
          {/each}

          {#if drag?.kind === "wire"}
            <path class="ng-wire-draft" d={linkPath(drag.fromPos, drag.curPos)}></path>
          {/if}

          {#each view.nodes as node (node.id)}
            {@const pos = nodeXY(node)}
            {@const h = nodeHeight(node)}
            {@const meta = nodeMeta(node.typeId)}
            {@const stageClass = stageAccentKey(meta.stage)}
            {@const selected = view.selectedNodes?.includes(node.id)}
            {@const badge = nodeBadgeText(node)}
            {@const hasCurve = node.summary?.kind === "curve" && node.summary.sparkline?.length}
            {@const sockTop = NODE_HEADER_H + (hasCurve ? 30 : 20)}
            <g
              class={(selected ? "ng-node-sel " : "") + (node.muted ? "ng-node-muted" : "")}
              transform={`translate(${pos.x} ${pos.y})`}
              role="button"
              tabindex="0"
              aria-label={meta.title}
              onpointerdown={(e) => onNodePointerDown(e, node)}
              onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") dispatch({ kind: "select_nodes", ids: [node.id] }); }}
            >
              <rect class="ng-node-card" width={NODE_WIDTH} height={h} rx="8"></rect>
              <rect class={"ng-node-hd-" + stageClass} width={NODE_WIDTH} height={NODE_HEADER_H} rx="6"></rect>
              <text class="ng-node-title" x="10" y="17">{meta.title}</text>
              {#if badge}
                <rect class="ng-node-badge-bg" x={NODE_WIDTH - 48} y="6" width="42" height="14" rx="7"></rect>
                <text class="ng-node-badge-text" x={NODE_WIDTH - 27} y="16" text-anchor="middle">{badge}</text>
              {/if}
              <text class="ng-node-sub" x="10" y="40">{nodeSubtitle(node, view.slots)}</text>

              {#if hasCurve}
                {@const sPts = sparklinePoints(node.summary.sparkline, NODE_WIDTH - 20, 20, 2)}
                <svg x="10" y="34" width={NODE_WIDTH - 20} height="20">
                  <polyline fill="none" stroke="#f1ad67" stroke-width="1.5" points={sPts}></polyline>
                </svg>
              {/if}

              {#each node.inputs as sock, idx (sock.id)}
                {@const sy = sockTop + idx * SOCKET_ROW_H + SOCKET_ROW_H * 0.5}
                <circle
                  class="ng-sock-dot"
                  cx="0"
                  cy={sy}
                  r="5"
                  fill={socketColour(sock.socketType)}
                  role="button"
                  tabindex="-1"
                  aria-label={"Input " + sock.id}
                  onpointerup={(e) => onInSocketUp(e, node, sock.id)}
                ></circle>
                <text class="ng-sock-label" x="10" y={sy + 3}>{sock.id}</text>
              {/each}

              {#each node.outputs as sock, idx (sock.id)}
                {@const sy = sockTop + idx * SOCKET_ROW_H + SOCKET_ROW_H * 0.5}
                <circle
                  class="ng-sock-dot"
                  cx={NODE_WIDTH}
                  cy={sy}
                  r="5"
                  fill={socketColour(sock.socketType)}
                  role="button"
                  tabindex="-1"
                  aria-label={"Output " + sock.id}
                  onpointerdown={(e) => onOutSocketDown(e, node, sock.id)}
                ></circle>
                <text class="ng-sock-label" x={NODE_WIDTH - 10} y={sy + 3} text-anchor="end">{sock.id}</text>
              {/each}
            </g>
          {/each}
        </svg>
      </div>

      <aside class="ng-drawer" aria-label="Block Settings">
        <div class="ng-drawer-body">
          {#if view.validationIssues?.length}
            <div class="ng-banner-warn">
              {#each view.validationIssues as issue}
                <div>{issue}</div>
              {/each}
            </div>
          {/if}

          {#if selectedNode}
            {@const meta = nodeMeta(selectedNode.typeId)}
            {@const basicParams = selectedNode.paramSpecs.filter((s) => !isAdvancedParam(selectedNode.typeId, s.id))}
            {@const advParams = selectedNode.paramSpecs.filter((s) => isAdvancedParam(selectedNode.typeId, s.id))}
            <div>
              <strong>{meta.title}</strong>
              <div class="ng-muted">{selectedNode.id} · {selectedNode.typeId}</div>
              <p class="ng-muted">{meta.description}</p>
            </div>

            <div class="ng-chips">
              <button
                type="button"
                class={selectedNode.muted ? "ng-btn ng-btn-teal" : "ng-btn"}
                onclick={() => dispatch({ kind: "set_muted", nodeId: selectedNode.id, muted: !selectedNode.muted })}
              >
                {selectedNode.muted ? "Turn On" : "Bypass (Mute)"}
              </button>
              <button type="button" class="ng-btn" onclick={() => dispatch({ kind: "duplicate_selected", offsetX: 30, offsetY: 30 })}>Duplicate</button>
              <button type="button" class="ng-btn ng-btn-danger" onclick={() => dispatch({ kind: "remove_nodes", ids: [selectedNode.id] })}>Delete</button>
            </div>

            {#if selectedNode.outputs?.length}
              {#each selectedNode.outputs as out (out.id)}
                {@const previewing = isViewerEndpoint(selectedNode.id, out.id)}
                <button
                  type="button"
                  class={previewing ? "ng-btn ng-btn-teal" : "ng-btn"}
                  onclick={() => dispatch({
                    kind: "set_viewer_target",
                    node: previewing ? null : selectedNode.id,
                    socket: previewing ? null : out.id,
                  })}
                >
                  {previewing ? "● Previewing " + out.id + " on Timeline" : "Preview " + out.id + " on Timeline"}
                </button>
              {/each}
            {/if}

            {#if selectedNode.typeId === "input.file"}
              <button type="button" class="ng-btn ng-btn-teal" onclick={() => browseStemAudio(selectedNode.id)}>
                Browse Audio File…
              </button>
            {/if}

            {#each basicParams as spec (spec.id)}
              {@const pm = paramMeta(selectedNode.typeId, spec.id)}
              {#if pm.control === "sensitivity"}
                {@const sens = thresholdToSensitivity(selectedNode.params[spec.id] ?? spec.default)}
                <label class="ng-label">
                  {pm.label}: {sens}%
                  <input
                    class="ng-slider"
                    type="range"
                    min="0"
                    max="100"
                    value={sens}
                    onchange={(e) => dispatch({ kind: "set_param", nodeId: selectedNode.id, param: spec.id, value: sensitivityToThreshold(e.currentTarget.value) })}
                  />
                </label>
              {:else if spec.socketType === "bool"}
                <label class="ng-row ng-label">
                  <span>{pm.label}</span>
                  <input
                    type="checkbox"
                    checked={!!(selectedNode.params[spec.id] ?? spec.default)}
                    onchange={(e) => handleParamChange(selectedNode, spec, e)}
                  />
                </label>
              {:else if spec.socketType === "enum" && spec.enumValues?.length}
                <label class="ng-row ng-label">
                  <span>{pm.label}</span>
                  <select class="ng-select" value={String(selectedNode.params[spec.id] ?? spec.default ?? "")} onchange={(e) => handleParamChange(selectedNode, spec, e)}>
                    {#each spec.enumValues as choice}
                      <option value={choice}>{choice}</option>
                    {/each}
                  </select>
                </label>
              {:else}
                <label class="ng-row ng-label">
                  <span>{pm.label}</span>
                  <input
                    class="ng-input"
                    type={spec.socketType === "number" || spec.socketType === "integer" ? "number" : "text"}
                    step={spec.socketType === "number" ? "any" : "1"}
                    value={formatParamValue(spec, selectedNode.params[spec.id])}
                    onchange={(e) => handleParamChange(selectedNode, spec, e)}
                  />
                </label>
              {/if}
            {/each}

            {#if advParams.length}
              <details class="ng-details">
                <summary>Advanced DSP Settings ({advParams.length})</summary>
                <div class="ng-details-body">
                  {#each advParams as spec (spec.id)}
                    {@const pm = paramMeta(selectedNode.typeId, spec.id)}
                    <label class="ng-row ng-label">
                      <span>{pm.label}</span>
                      {#if spec.socketType === "enum" && spec.enumValues?.length}
                        <select class="ng-select" value={String(selectedNode.params[spec.id] ?? spec.default ?? "")} onchange={(e) => handleParamChange(selectedNode, spec, e)}>
                          {#each spec.enumValues as choice}
                            <option value={choice}>{choice}</option>
                          {/each}
                        </select>
                      {:else}
                        <input
                          class="ng-input"
                          type={spec.socketType === "number" || spec.socketType === "integer" ? "number" : "text"}
                          step={spec.socketType === "number" ? "any" : "1"}
                          value={formatParamValue(spec, selectedNode.params[spec.id])}
                          onchange={(e) => handleParamChange(selectedNode, spec, e)}
                        />
                      {/if}
                    </label>
                  {/each}
                </div>
              </details>
            {/if}
          {:else}
            <p class="ng-muted">Click any block on the canvas to edit its controls, or drag between circles to connect blocks. Press Del to delete or Ctrl+Z to undo.</p>
          {/if}
        </div>
      </aside>
    </div>
  {/if}

  <!-- Bottom Timeline Preview Strip -->
  <section class="ng-preview-strip" aria-label="Timeline preview">
    <div class="ng-row">
      <span class="ng-label">
        TIMELINE PREVIEW (synced with main editor)
        {#if view?.viewerSocket}
          · {view.viewerSocket.node}.{view.viewerSocket.socket}
        {/if}
      </span>
      {#if view?.viewerSocket}
        <button type="button" class="ng-btn" onclick={() => dispatch({ kind: "set_viewer_target", node: null, socket: null })}>Clear Preview</button>
      {/if}
    </div>
    <div class="ng-well">
      <canvas bind:this={previewCanvas} class="ng-preview-canvas" aria-label="Node graph preview waveform"></canvas>
    </div>
  </section>

  <!-- Footer Action Bar -->
  <footer class="ng-footer">
    {#if view?.status === "running"}
      <span class="ng-status-ok">Analysing audio… {view.done}%</span>
      <button type="button" class="ng-btn" onclick={() => dispatch({ kind: "cancel" })}>Cancel</button>
    {:else if view?.batch?.stale}
      <span class="ng-banner-warn">Map or graph changed since last run — run again before applying.</span>
    {:else if view?.batch}
      <span class="ng-status-ok">
        Ready · {view.batch.counts.checked} checked hits (1 undo step)
      </span>
    {:else}
      <span class="ng-muted">{statusNotice || "Pick lanes and click Run or Test L–R Loop."}</span>
    {/if}

    <div class="ng-spacer"></div>

    {#if view?.batch}
      <button type="button" class="ng-btn" onclick={() => { checklistOpen = true; }}>
        Review Hits ({view.batch.counts.checked})
      </button>
    {/if}
    {#if view?.canSelectLastHits}
      <button type="button" class="ng-btn" onclick={() => dispatch({ kind: "select_last_hits" }, "Selected last applied hits in editor.")}>
        Select Last Hits
      </button>
    {/if}
    <button
      type="button"
      class="ng-btn ng-btn-teal"
      disabled={!view?.hasDocument || !view?.hasLocatorRange}
      onclick={() => runRange(true)}
    >
      Test L–R Loop
    </button>
    <button
      type="button"
      class="ng-btn"
      disabled={!view?.hasDocument}
      onclick={() => runRange(false)}
    >
      Run Whole Song
    </button>
    <button
      type="button"
      class="ng-btn ng-btn-primary"
      disabled={!view?.batch || view.batch.stale || !view.batch.counts?.checked}
      onclick={() => dispatch({ kind: "apply" }, "Applied hits to map (Ctrl+Z in editor to undo).")}
    >
      Apply {view?.batch?.counts?.checked ?? 0} Hits
    </button>
  </footer>
</div>

{#if checklistOpen && view?.batch}
  <div class="ng-checklist-overlay" role="dialog" aria-label="Review proposed hits">
    <div class="ng-checklist-card">
      <div class="ng-header">
        <strong>Review Proposed Hits ({view.batch.counts.checked} / {view.batch.counts.total})</strong>
        <div class="ng-spacer"></div>
        <button type="button" class="ng-btn" onclick={() => dispatch({ kind: "set_all_proposals", checked: true })}>All</button>
        <button type="button" class="ng-btn" onclick={() => dispatch({ kind: "set_all_proposals", checked: false })}>None</button>
        <button type="button" class="ng-btn" onclick={() => { checklistOpen = false; }}>Close</button>
      </div>
      <div class="ng-checklist-rows">
        {#each view.batch.proposals as item (item.id)}
          <label class="ng-check-row">
            <span>
              <input
                type="checkbox"
                checked={item.checked}
                onchange={(e) => dispatch({ kind: "toggle_proposal", id: item.id, checked: e.currentTarget.checked })}
              />
              <span class="ng-mono">{Math.round(item.timeMs)} ms</span>
              <strong>{item.slot}</strong>
              <span class="ng-muted">→ {item.laneName}</span>
            </span>
            <span class="ng-muted">{item.strength.toFixed(2)}</span>
          </label>
        {/each}
      </div>
    </div>
  </div>
{/if}
