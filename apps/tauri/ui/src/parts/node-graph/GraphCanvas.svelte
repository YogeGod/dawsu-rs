<script>
  import {
    NODE_HEADER_H,
    NODE_WIDTH,
    SOCKET_ROW_H,
    inputSocketPos,
    linkPath,
    nodeBadgeText,
    nodeHeight,
    nodeMeta,
    nodeSubtitle,
    outputSocketPos,
    socketColour,
    sparklinePoints,
    stageAccentKey,
  } from "../../lib/node-graph.js";

  let {
    view,
    vb = $bindable({ x: -40, y: -40, w: 1200, h: 620 }),
    addMenuOpen = $bindable(false),
    ondispatch,
    onautoarrange,
    onfit,
    oncopyjson,
    children,
  } = $props();

  let svgEl = $state(null);
  let drag = $state(null);
  let dragPos = $state(null);

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
    ondispatch({ kind: "select_nodes", ids: [node.id] });
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
      ondispatch({
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
      ondispatch({ kind: "select_nodes", ids: [] });
    } else if (active.kind === "node" && active.moved && dragPos) {
      const pos = dragPos;
      dragPos = null;
      ondispatch({ kind: "move_nodes", positions: [pos] });
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
</script>

<!-- CONCEPT B: RAW BLOCKS PATCHBAY -->
<div class="ng-raw-subbar">
  <button type="button" class="ng-btn ng-btn-teal" onclick={() => { addMenuOpen = !addMenuOpen; }}>
    + Add Block ▾
  </button>
  <button type="button" class="ng-btn" onclick={onautoarrange}>Auto-arrange</button>
  <button type="button" class="ng-btn" onclick={onfit}>Fit</button>
  <button type="button" class="ng-btn" disabled={!view.canUndo} onclick={() => ondispatch({ kind: "undo" })}>Undo</button>
  <button type="button" class="ng-btn" disabled={!view.canRedo} onclick={() => ondispatch({ kind: "redo" })}>Redo</button>
  <button type="button" class="ng-btn" onclick={oncopyjson}>Copy JSON</button>

  <div class="ng-spacer"></div>
  <span class="ng-muted">Slots:</span>
  {#each view.slots as slot (slot.slot)}
    <select
      class="ng-select"
      aria-label={"Lane for slot " + slot.slot}
      value={slot.boundLaneKey ?? ""}
      onchange={(e) => ondispatch({ kind: "set_binding", slot: slot.slot, laneKey: e.currentTarget.value || null })}
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
              ondispatch({
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
            ondispatch({
              kind: "disconnect",
              fromNode: link.fromNode,
              fromSocket: link.fromSocket,
              toNode: link.toNode,
              toSocket: link.toSocket,
            });
          }}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === "Delete" || e.key === "Backspace") {
              ondispatch({
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
          onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") ondispatch({ kind: "select_nodes", ids: [node.id] }); }}
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

  {@render children?.()}
</div>
