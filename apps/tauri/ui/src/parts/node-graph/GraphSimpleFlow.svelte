<script>
  import {
    BAND_PRESETS,
    BEAT_DIVISORS,
    nodeSubtitle,
    sensitivityToThreshold,
    sparklinePoints,
  } from "../../lib/node-graph.js";

  let {
    view,
    flow,
    ondispatch,
    onloadrecipe,
    onswitchtostem,
    onbrowsestem,
    onexpandraw,
    oneditblock,
  } = $props();

  let curveDrag = $state(null);

  async function setDivisorBit(snapNode, divisor) {
    const cur = Array.isArray(snapNode.params?.divisors) ? snapNode.params.divisors : [4];
    let next;
    if (cur.includes(divisor)) {
      next = cur.filter((d) => d !== divisor);
    } else {
      next = [...cur, divisor].sort((a, b) => a - b);
    }
    if (!next.length) next = [divisor];
    await ondispatch({ kind: "set_param", nodeId: snapNode.id, param: "divisors", value: next });
  }

  async function setBandPreset(fluxNode, preset) {
    await ondispatch({ kind: "set_param", nodeId: fluxNode.id, param: "loHz", value: preset.loHz });
    await ondispatch({ kind: "set_param", nodeId: fluxNode.id, param: "hiHz", value: preset.hiHz });
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
    await ondispatch({ kind: "set_param", nodeId: det.peaks.id, param: "threshold", value: finalThreshold });
  }

  function detectorThreshold(det) {
    if (curveDrag && curveDrag.nodeId === det.peaks.id) return curveDrag.threshold;
    return Number(det.peaks.params?.threshold ?? 1.5);
  }

  function detectorSensitivity(det) {
    if (curveDrag && curveDrag.nodeId === det.peaks.id) return curveDrag.pct;
    return det.sensitivity;
  }

  function isViewerEndpoint(nodeId, socketId) {
    return view?.viewerSocket?.node === nodeId && view?.viewerSocket?.socket === socketId;
  }
</script>

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
          onclick={() => { if (flow.isStem) onloadrecipe("classic"); }}
        >
          Song Audio
        </button>
        <button
          type="button"
          class="ng-seg-btn"
          aria-pressed={flow.isStem}
          onclick={onswitchtostem}
        >
          Stem File…
        </button>
      </div>

      {#if flow.fileNode}
        <div class="ng-row">
          <span class="ng-muted">{nodeSubtitle(flow.fileNode, view.slots)}</span>
          <button type="button" class="ng-btn" onclick={() => onbrowsestem(flow.fileNode.id)}>Browse…</button>
        </div>
      {/if}

      <div class="ng-label">Analyse Range</div>
      <div class="ng-chips">
        <button
          type="button"
          class={view.useLocatorRange ? "ng-btn" : "ng-btn ng-btn-teal"}
          onclick={() => ondispatch({ kind: "set_range_mode", useLocatorRange: false })}
        >
          Whole Song
        </button>
        <button
          type="button"
          class={view.useLocatorRange ? "ng-btn ng-btn-teal" : "ng-btn"}
          disabled={!view.hasLocatorRange}
          onclick={() => ondispatch({ kind: "set_range_mode", useLocatorRange: true })}
        >
          L–R Loop
        </button>
      </div>

      {#if flow.hpssNode}
        <button
          type="button"
          class="ng-switch-card"
          onclick={() => ondispatch({ kind: "set_muted", nodeId: flow.hpssNode.id, muted: !flow.hpssNode.muted })}
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

      <button type="button" class="ng-btn" onclick={onexpandraw}>
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
        <button type="button" class="ng-btn ng-btn-teal" onclick={onexpandraw}>
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
              <text x="6" y="12" font-size="9" fill="#a3a3a3">Drag line up/down, sensitivity {sensPct}%</text>
            </svg>
          </div>

          <div class="ng-row">
            <span class="ng-label">Sensitivity: {sensPct}% · Gap: {Math.round(det.minGapMs)} ms</span>
            <div class="ng-chips">
              {#if det.bleed}
                <button
                  type="button"
                  class={det.bleed.muted ? "ng-btn" : "ng-btn ng-btn-teal"}
                  onclick={() => ondispatch({ kind: "set_muted", nodeId: det.bleed.id, muted: !det.bleed.muted })}
                >
                  Bleed guard: {det.bleed.muted ? "Off" : "Auto"}
                </button>
              {/if}
              {#if det.curveSource}
                {@const previewing = isViewerEndpoint(det.curveSource.id, "curve")}
                <button
                  type="button"
                  class={previewing ? "ng-btn ng-btn-teal" : "ng-btn"}
                  onclick={() => ondispatch({
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
                onclick={() => oneditblock(det.peaks.id)}
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
            onchange={(e) => ondispatch({ kind: "set_param", nodeId: flow.snapNode.id, param: "toleranceMs", value: Number(e.currentTarget.value) })}
          />
        </label>

        <button
          type="button"
          class={flow.snapNode.params?.measureLag !== false ? "ng-btn ng-btn-teal" : "ng-btn"}
          onclick={() => ondispatch({ kind: "set_param", nodeId: flow.snapNode.id, param: "measureLag", value: !(flow.snapNode.params?.measureLag !== false) })}
        >
          {flow.snapNode.params?.measureLag !== false ? "✓ Auto lag align" : "○ Auto lag align off"}
        </button>

        <button
          type="button"
          class={flow.snapNode.params?.dropOffGrid !== false ? "ng-btn ng-btn-teal" : "ng-btn"}
          onclick={() => ondispatch({ kind: "set_param", nodeId: flow.snapNode.id, param: "dropOffGrid", value: !(flow.snapNode.params?.dropOffGrid !== false) })}
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
              onchange={(e) => ondispatch({ kind: "set_binding", slot: slot.slot, laneKey: e.currentTarget.value || null })}
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
