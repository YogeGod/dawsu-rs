<script>
  import { untrack } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    NODE_HEADER_H,
    NODE_WIDTH,
    SOCKET_ROW_H,
    fitViewBox,
    formatParamValue,
    inputSocketPos,
    linkPath,
    nodeHeight,
    outputSocketPos,
    parseParamInput,
    socketColour,
  } from "../lib/node-graph.js";

  let view = $state.raw(null);
  let error = $state("");
  let drawerTab = $state("inspector");
  let addTypeId = $state("onset.flux");
  let presetChoice = $state("classic");
  let jsonOpen = $state(false);
  let jsonText = $state("");
  let svgEl = $state(null);
  let vb = $state({ x: -60, y: -220, w: 1500, h: 620 });
  let fittedForDoc = $state("");
  let drag = $state(null);
  let dragPos = $state(null);

  const open = $derived(bridge.nodeGraphOpen && !!bridge.scene);
  const path = $derived(bridge.scene?.path ?? null);

  function syncViewer(next) {
    view = next;
    bridge.nodeGraphViewer = next?.viewer ?? null;
    globalThis.paintSoon?.();
  }

  async function refreshView() {
    try {
      const next = await invoke("node_graph_view");
      syncViewer(next);
      error = "";
      if (next?.catalog?.length && !next.catalog.some((c) => c.typeId === addTypeId)) {
        addTypeId = next.catalog[0].typeId;
      }
      if (next && fittedForDoc !== (path ?? "") + ":" + next.nodes.length && next.nodes.length && fittedForDoc === "") {
        vb = fitViewBox(next.nodes);
        fittedForDoc = path ?? "open";
      }
    } catch (err) {
      error = String(err);
    }
  }

  async function dispatch(action, refreshTimeline = false) {
    try {
      error = "";
      const next = await invoke("node_graph_action", { action });
      syncViewer(next);
      if (refreshTimeline) {
        await globalThis.update?.(null, true);
      }
      return next;
    } catch (err) {
      error = String(err);
      return null;
    }
  }

  $effect(() => {
    if (!open || !path) {
      bridge.nodeGraphViewer = null;
      globalThis.paintSoon?.();
      return;
    }
    void path;
    untrack(() => {
      fittedForDoc = "";
      refreshView();
    });
  });

  $effect(() => {
    if (!open || view?.status !== "running") return;
    const timer = setInterval(refreshView, 150);
    return () => clearInterval(timer);
  });

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
    const pt = svgPoint(event);
    drag = { kind: "pan", startX: event.clientX, startY: event.clientY, origVb: { ...vb }, moved: false, pt };
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
    const nextW = Math.max(320, Math.min(8000, vb.w * factor));
    const nextH = Math.max(180, Math.min(4500, vb.h * factor));
    const rx = (pt.x - vb.x) / vb.w;
    const ry = (pt.y - vb.y) / vb.h;
    vb = { x: pt.x - rx * nextW, y: pt.y - ry * nextH, w: nextW, h: nextH };
  }

  const selectedNode = $derived.by(() => {
    if (!view || !view.selectedNodes?.length) return null;
    const first = view.selectedNodes[0];
    return view.nodes.find((n) => n.id === first) ?? null;
  });

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

  async function toggleJsonBox() {
    jsonOpen = !jsonOpen;
    if (jsonOpen) {
      try {
        jsonText = await invoke("node_graph_export_json");
      } catch (err) {
        error = String(err);
      }
    }
  }

  function isSelected(id) {
    return !!view?.selectedNodes?.includes(id);
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
</script>

{#if open}
  <section class="ng-shell" aria-label="Node Graph">
    <div class="ng-toolbar">
      <span class="ng-title">Node Graph</span>
      <select aria-label="Preset" bind:value={presetChoice}>
        <option value="classic">Kick &amp; snare (classic)</option>
        <option value="empty">Empty graph</option>
      </select>
      <button type="button" onclick={() => dispatch({ kind: "load_preset", name: presetChoice }).then((n) => { if (n) vb = fitViewBox(n.nodes); })}>
        Load preset
      </button>
      {#if view?.catalog?.length}
        <select aria-label="Node type" bind:value={addTypeId}>
          {#each view.catalog as item (item.typeId)}
            <option value={item.typeId}>{item.typeId}</option>
          {/each}
        </select>
        <button type="button" onclick={() => dispatch({ kind: "add_node", typeId: addTypeId, x: Math.round(vb.x + vb.w * 0.45), y: Math.round(vb.y + vb.h * 0.4) })}>
          + Node
        </button>
      {/if}
      <button type="button" disabled={!view?.canUndo} onclick={() => dispatch({ kind: "undo" })}>Undo</button>
      <button type="button" disabled={!view?.canRedo} onclick={() => dispatch({ kind: "redo" })}>Redo</button>
      <button type="button" onclick={() => { if (view) vb = fitViewBox(view.nodes); }}>Fit</button>
      <button
        type="button"
        onclick={() => dispatch({ kind: "set_range_mode", useLocatorRange: !view?.useLocatorRange })}
      >
        {view?.useLocatorRange ? "Range: Locators" : "Range: Full song"}
      </button>
      <button type="button" onclick={toggleJsonBox}>JSON</button>

      <div class="ng-spacer"></div>

      {#if view?.status === "running"}
        <span class="ng-status">Running {view.done}%</span>
        <button type="button" onclick={() => dispatch({ kind: "cancel" })}>Cancel</button>
      {:else}
        <button type="button" class="ng-btn-primary" disabled={!view?.hasDocument} onclick={() => dispatch({ kind: "run" })}>
          Run
        </button>
      {/if}

      <button
        type="button"
        class="ng-btn-accent"
        disabled={!view?.batch || view.batch.stale || !view.batch.counts?.checked}
        onclick={() => dispatch({ kind: "apply" }, true)}
      >
        Apply ({view?.batch?.counts?.checked ?? 0})
      </button>
      <button
        type="button"
        disabled={!view?.canSelectLastHits}
        onclick={() => dispatch({ kind: "select_last_hits" }, true)}
      >
        Select last
      </button>
      <button type="button" aria-label="Close Node Graph" onclick={() => { bridge.nodeGraphOpen = false; }}>
        Close
      </button>
    </div>

    <div class="ng-body">
      <div class="ng-canvas-wrap">
        <svg
          bind:this={svgEl}
          class="ng-svg"
          viewBox={`${vb.x} ${vb.y} ${vb.w} ${vb.h}`}
          onpointerdown={onSvgPointerDown}
          onpointermove={onSvgPointerMove}
          onpointerup={onSvgPointerUp}
          onwheel={onSvgWheel}
          role="application"
          aria-label="Audio analysis node graph canvas"
        >
          {#if view}
            {#each view.links as link (link.fromNode + "." + link.fromSocket + "->" + link.toNode + "." + link.toSocket)}
              <path
                class="ng-link"
                d={renderedLinkPath(link)}
                stroke={socketColour(link.socketType)}
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
              >
                <title>Click to disconnect {link.fromNode}.{link.fromSocket}</title>
              </path>
            {/each}

            {#if drag?.kind === "wire"}
              <path class="ng-link-draft" d={linkPath(drag.fromPos, drag.curPos)}></path>
            {/if}

            {#each view.nodes as node (node.id)}
              {@const pos = nodeXY(node)}
              {@const h = nodeHeight(node)}
              <g
                class="ng-node"
                class:ng-node-selected={isSelected(node.id)}
                class:ng-node-muted={node.muted}
                transform={`translate(${pos.x} ${pos.y})`}
                onpointerdown={(e) => onNodePointerDown(e, node)}
              >
                <rect class="ng-node-rect" width={NODE_WIDTH} height={h} rx="6"></rect>
                <rect class="ng-node-header" width={NODE_WIDTH} height={NODE_HEADER_H} rx="5"></rect>
                <text class="ng-node-title" x="10" y="17">{node.id} · {node.typeId}</text>

                {#each node.inputs as sock, idx (sock.id)}
                  {@const sy = NODE_HEADER_H + idx * SOCKET_ROW_H + SOCKET_ROW_H * 0.5}
                  <circle
                    class="ng-socket-dot"
                    cx="0"
                    cy={sy}
                    r="5"
                    fill={socketColour(sock.socketType)}
                    onpointerup={(e) => onInSocketUp(e, node, sock.id)}
                  >
                    <title>in: {sock.id} ({sock.socketType})</title>
                  </circle>
                  <text class="ng-socket-label" x="10" y={sy + 3}>{sock.id}</text>
                {/each}

                {#each node.outputs as sock, idx (sock.id)}
                  {@const sy = NODE_HEADER_H + idx * SOCKET_ROW_H + SOCKET_ROW_H * 0.5}
                  <circle
                    class="ng-socket-dot"
                    cx={NODE_WIDTH}
                    cy={sy}
                    r="5"
                    fill={socketColour(sock.socketType)}
                    onpointerdown={(e) => onOutSocketDown(e, node, sock.id)}
                  >
                    <title>out: {sock.id} ({sock.socketType})</title>
                  </circle>
                  <text class="ng-socket-label" x={NODE_WIDTH - 10} y={sy + 3} text-anchor="end">{sock.id}</text>
                {/each}
              </g>
            {/each}
          {/if}
        </svg>
      </div>

      <aside class="ng-drawer" aria-label="Node inspector and review">
        <div class="ng-tabs" role="tablist">
          <button
            type="button"
            role="tab"
            class="ng-tab"
            aria-selected={drawerTab === "inspector"}
            onclick={() => { drawerTab = "inspector"; }}
          >
            Inspector
          </button>
          <button
            type="button"
            role="tab"
            class="ng-tab"
            aria-selected={drawerTab === "review"}
            onclick={() => { drawerTab = "review"; }}
          >
            Slots &amp; Review ({view?.batch?.counts?.checked ?? 0})
          </button>
        </div>

        <div class="ng-drawer-scroll">
          {#if error || view?.error}
            <div class="ng-banner-error">{error || view.error}</div>
          {/if}

          {#if view?.validationIssues?.length}
            <div class="ng-banner-warn">
              {#each view.validationIssues as issue}
                <div>{issue}</div>
              {/each}
            </div>
          {/if}

          {#if jsonOpen}
            <textarea class="ng-json-box" bind:value={jsonText} aria-label="Graph JSON"></textarea>
            <div class="ng-row">
              <button type="button" onclick={() => dispatch({ kind: "import_json", json: jsonText }).then((n) => { if (n) { jsonOpen = false; vb = fitViewBox(n.nodes); } })}>
                Import JSON
              </button>
              <button type="button" onclick={() => { jsonOpen = false; }}>Close JSON</button>
            </div>
          {/if}

          {#if drawerTab === "inspector"}
            {#if selectedNode}
              <div class="ng-row">
                <strong>{selectedNode.id}</strong>
                <span>{selectedNode.typeId}</span>
              </div>
              <div class="ng-row">
                <label class="ng-field-label">
                  <input
                    type="checkbox"
                    checked={selectedNode.muted}
                    onchange={(e) => dispatch({ kind: "set_muted", nodeId: selectedNode.id, muted: e.currentTarget.checked })}
                  />
                  Muted
                </label>
                <button type="button" onclick={() => dispatch({ kind: "duplicate_selected", offsetX: 30, offsetY: 30 })}>
                  Duplicate
                </button>
                <button type="button" onclick={() => dispatch({ kind: "remove_nodes", ids: [selectedNode.id] })}>
                  Delete
                </button>
              </div>

              {#if selectedNode.placeholder}
                <div class="ng-banner-warn">{selectedNode.placeholder}</div>
              {/if}

              {#if selectedNode.diagnostics?.length}
                {#each selectedNode.diagnostics as d}
                  <div class={d.severity === "error" ? "ng-banner-error" : "ng-banner-warn"}>{d.message}</div>
                {/each}
              {/if}

              {#if selectedNode.outputs?.length}
                <div class="ng-field-label">Timeline preview socket</div>
                {#each selectedNode.outputs as out (out.id)}
                  <div class="ng-row">
                    <span>{out.id} ({out.socketType})</span>
                    <button
                      type="button"
                      class={isViewerEndpoint(selectedNode.id, out.id) ? "ng-btn-primary" : ""}
                      onclick={() => {
                        const wasPreviewing = isViewerEndpoint(selectedNode.id, out.id);
                        const wasReady = view?.status === "ready";
                        dispatch({
                          kind: "set_viewer_target",
                          node: wasPreviewing ? null : selectedNode.id,
                          socket: wasPreviewing ? null : out.id,
                        }).then((next) => {
                          if (!wasPreviewing && next && wasReady) {
                            dispatch({ kind: "run" });
                          }
                        });
                      }}
                    >
                      {isViewerEndpoint(selectedNode.id, out.id) ? "Previewing" : "Preview"}
                    </button>
                  </div>
                {/each}
              {/if}

              {#each selectedNode.paramSpecs as spec (spec.id)}
                <div class="ng-row">
                  <label class="ng-field-label" for={"ng-param-" + spec.id}>{spec.id}</label>
                  {#if spec.socketType === "bool"}
                    <input
                      id={"ng-param-" + spec.id}
                      type="checkbox"
                      checked={!!(selectedNode.params[spec.id] ?? spec.default)}
                      onchange={(e) => handleParamChange(selectedNode, spec, e)}
                    />
                  {:else if spec.socketType === "enum" && spec.enumValues?.length}
                    <select
                      id={"ng-param-" + spec.id}
                      value={String(selectedNode.params[spec.id] ?? spec.default ?? "")}
                      onchange={(e) => handleParamChange(selectedNode, spec, e)}
                    >
                      {#each spec.enumValues as choice}
                        <option value={choice}>{choice}</option>
                      {/each}
                    </select>
                  {:else}
                    <input
                      id={"ng-param-" + spec.id}
                      type={spec.socketType === "number" || spec.socketType === "integer" ? "number" : "text"}
                      step={spec.socketType === "number" ? "any" : "1"}
                      value={formatParamValue(spec, selectedNode.params[spec.id])}
                      onchange={(e) => handleParamChange(selectedNode, spec, e)}
                    />
                  {/if}
                </div>
              {/each}
            {:else}
              <p class="ng-status">Click any node on the canvas to inspect its parameters, mute state, or preview one of its outputs on the timeline.</p>
            {/if}
          {:else if view}
            <div class="ng-field-label">Output slot lane bindings</div>
            {#each view.slots as slot (slot.slot)}
              <div class="ng-row">
                <span>{slot.slot}</span>
                <select
                  aria-label={"Lane for slot " + slot.slot}
                  value={slot.boundLaneKey ?? ""}
                  onchange={(e) => dispatch({ kind: "set_binding", slot: slot.slot, laneKey: e.currentTarget.value || null })}
                >
                  <option value="">Unbound</option>
                  {#each view.lanes as lane (lane.key)}
                    <option value={lane.key}>{lane.name}{lane.hidden ? " (hidden)" : ""}</option>
                  {/each}
                </select>
              </div>
            {/each}

            {#if view.batch?.stale}
              <div class="ng-banner-warn">
                The map or graph changed since the last run ({view.batch.staleReason}). Run again before applying.
              </div>
            {/if}

            <div class="ng-row">
              <span class="ng-field-label">Snap ms: {view.applyOptions.toleranceMs}</span>
              <button
                type="button"
                onclick={() => dispatch({
                  kind: "set_apply_options",
                  toleranceMs: view.applyOptions.toleranceMs,
                  dedupeMs: view.applyOptions.dedupeMs,
                  includeUnmatched: !view.applyOptions.includeUnmatched,
                })}
              >
                {view.applyOptions.includeUnmatched ? "Unmatched: Keep" : "Unmatched: Drop"}
              </button>
            </div>

            {#if view.batch}
              <div class="ng-row">
                <span>Proposals: {view.batch.counts.checked} / {view.batch.counts.total}</span>
                <button type="button" onclick={() => dispatch({ kind: "set_all_proposals", checked: true })}>All</button>
                <button type="button" onclick={() => dispatch({ kind: "set_all_proposals", checked: false })}>None</button>
              </div>
              <div class="ng-proposal-list">
                {#each view.batch.proposals as item (item.id)}
                  <label class="ng-proposal-item">
                    <input
                      type="checkbox"
                      checked={item.checked}
                      onchange={(e) => dispatch({ kind: "toggle_proposal", id: item.id, checked: e.currentTarget.checked })}
                    />
                    <span>{Math.round(item.timeMs)} ms</span>
                    <span>{item.slot}</span>
                    <span>{item.laneName}</span>
                  </label>
                {/each}
              </div>
            {/if}
          {/if}
        </div>
      </aside>
    </div>
  </section>
{/if}
