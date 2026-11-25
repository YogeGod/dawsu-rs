<script>
  import {
    formatParamValue,
    nodeMeta,
    parseParamInput,
    sensitivityToThreshold,
    thresholdToSensitivity,
  } from "../../lib/node-graph.js";

  let {
    view,
    selectedNode = null,
    ondispatch,
    onbrowsestem,
    onerror,
  } = $props();

  function paramMeta(typeId, paramId) {
    return nodeMeta(typeId).params?.[paramId] ?? { label: paramId, control: "auto", advanced: false };
  }

  function isAdvancedParam(typeId, paramId) {
    return !!nodeMeta(typeId).params?.[paramId]?.advanced;
  }

  function handleParamChange(node, spec, event) {
    const parsed = parseParamInput(spec, event.currentTarget.value, event.currentTarget.checked);
    if (!parsed.ok) {
      onerror?.(parsed.error);
      return;
    }
    ondispatch({ kind: "set_param", nodeId: node.id, param: spec.id, value: parsed.value });
  }

  function isViewerEndpoint(nodeId, socketId) {
    return view?.viewerSocket?.node === nodeId && view?.viewerSocket?.socket === socketId;
  }
</script>

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
          onclick={() => ondispatch({ kind: "set_muted", nodeId: selectedNode.id, muted: !selectedNode.muted })}
        >
          {selectedNode.muted ? "Turn On" : "Bypass (Mute)"}
        </button>
        <button type="button" class="ng-btn" onclick={() => ondispatch({ kind: "duplicate_selected", offsetX: 30, offsetY: 30 })}>Duplicate</button>
        <button type="button" class="ng-btn ng-btn-danger" onclick={() => ondispatch({ kind: "remove_nodes", ids: [selectedNode.id] })}>Delete</button>
      </div>

      {#if selectedNode.outputs?.length}
        {#each selectedNode.outputs as out (out.id)}
          {@const previewing = isViewerEndpoint(selectedNode.id, out.id)}
          <button
            type="button"
            class={previewing ? "ng-btn ng-btn-teal" : "ng-btn"}
            onclick={() => ondispatch({
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
        <button type="button" class="ng-btn ng-btn-teal" onclick={() => onbrowsestem(selectedNode.id)}>
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
              onchange={(e) => ondispatch({ kind: "set_param", nodeId: selectedNode.id, param: spec.id, value: sensitivityToThreshold(e.currentTarget.value) })}
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
