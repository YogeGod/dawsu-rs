<script>
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import { forwardSpace } from "../lib/space.js";
  import {
    autoLayoutNodes,
    extractSimpleFlowModel,
    fitViewBox,
  } from "../lib/node-graph.js";
  import GraphToolbar from "./node-graph/GraphToolbar.svelte";
  import GraphSimpleFlow from "./node-graph/GraphSimpleFlow.svelte";
  import GraphCanvas from "./node-graph/GraphCanvas.svelte";
  import GraphInspector from "./node-graph/GraphInspector.svelte";
  import GraphProposalsPanel from "./node-graph/GraphProposalsPanel.svelte";

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
  let vb = $state({ x: -40, y: -40, w: 1200, h: 620 });
  let fitted = false;
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

  async function runRange(useLocatorRange) {
    await dispatch({ kind: "set_range_mode", useLocatorRange });
    await dispatch({ kind: "run" }, useLocatorRange ? "Testing L–R loop…" : "Analysing whole song…");
  }

  async function autoArrange() {
    if (!view?.nodes?.length) return;
    const positions = autoLayoutNodes(view.nodes, view.links ?? []);
    const next = await dispatch({ kind: "move_nodes", positions });
    if (next?.nodes) vb = fitViewBox(next.nodes);
  }

  function setMode(nextMode) {
    mode = nextMode;
    if (nextMode === "simple") {
      addMenuOpen = false;
    } else if (view?.nodes?.length) {
      vb = fitViewBox(view.nodes);
    }
  }

  function handleWindowKeyDown(event) {
    // Space plays and stops the editor from this window too, in both modes, unless a field has it.
    if (forwardSpace(event)) return;
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
</script>

<svelte:window onkeydown={handleWindowKeyDown} />

<div class="ng-win">
  <GraphToolbar
    {view}
    {recipeChoice}
    {mode}
    {error}
    onloadrecipe={loadRecipe}
    onpickopen={pickOpenFile}
    onpicksave={pickSaveFile}
    onsetmode={setMode}
    onshowineditor={() => invoke("show_in_editor")}
    onclose={() => invoke("close_node_graph_window")}
  />

  {#if mode === "simple" && view && flow}
    <GraphSimpleFlow
      {view}
      {flow}
      ondispatch={dispatch}
      onloadrecipe={loadRecipe}
      onswitchtostem={switchToStemFlow}
      onbrowsestem={browseStemAudio}
      onexpandraw={() => { mode = "raw"; vb = fitViewBox(view.nodes); }}
      oneditblock={(id) => {
        mode = "raw";
        dispatch({ kind: "select_nodes", ids: [id] });
        vb = fitViewBox(view.nodes);
      }}
    />
  {:else if view}
    <GraphCanvas
      {view}
      bind:vb
      bind:addMenuOpen
      ondispatch={dispatch}
      onautoarrange={autoArrange}
      onfit={() => { vb = fitViewBox(view.nodes); }}
      oncopyjson={copyJsonClipboard}
    >
      <GraphInspector
        {view}
        {selectedNode}
        ondispatch={dispatch}
        onbrowsestem={browseStemAudio}
        onerror={(msg) => { error = msg; }}
      />
    </GraphCanvas>
  {/if}

  <GraphProposalsPanel
    {view}
    {statusNotice}
    bind:checklistOpen
    ondispatch={dispatch}
    onrunrange={runRange}
  />
</div>
