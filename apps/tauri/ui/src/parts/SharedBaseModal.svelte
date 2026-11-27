<script>
  import { onMount } from "svelte";
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    CLEAR_BASE_CONFIRM,
    DEFAULT_LENIENCY_MS,
    MAX_LENIENCY_MS,
    RESET_OVERRIDES_CONFIRM,
    applyAllConfirm,
    applyArgs,
    applyToast,
    askFirst,
    confirmCaptureFirst,
    formatApplyAllSummary,
    formatApplySummary,
    formatStatusLine,
    uniqueLaneOptions,
  } from "../lib/sharedBase.js";

  // Map › Shared Base layer…: per-mapset Shared Base score, live difficulty tabs,
  // non-destructive per-difficulty lane rules, and green-line-only volume curve.
  // menus.js opens it via bridge modal registry.
  let open = $state(false);
  let tab = $state("overview"); // "overview" | "rules" | "volume"
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let status = $state({
    enabled: false,
    hasBase: false,
    sourceDifficulty: null,
    baseMomentCount: 0,
    currentDiffOverrides: 0,
    currentDiffInherited: 0,
  });
  let leniencyMs = $state(DEFAULT_LENIENCY_MS);
  let overwriteOverrides = $state(false);

  // Live difficulty projection tabs
  let diffTabs = $state([]);
  let selectedDiffFile = $state(null);

  // Per-difficulty rules
  let rules = $state([]);
  let ruleOp = $state("mute_lane");
  let ruleLane = $state("");
  let ruleToLane = $state("");
  let ruleUseRange = $state(false);
  let ruleStartMs = $state(0);
  let ruleEndMs = $state(10000);
  let ruleTimeMs = $state(1000);
  let ruleVolume = $state(80);

  // Green-line-only master volume curve (shared or per-difficulty)
  let volumeScope = $state("shared"); // "shared" | "diff"
  let envelopePoints = $state([]);
  let selectedPointIdx = $state(null);
  let newPtTime = $state(1000);
  let newPtVol = $state(80);
  let newPtCurve = $state("linear");

  const hasDoc = $derived(Boolean(bridge.scene));
  const durationMs = $derived(Math.max(1000, bridge.view?.duration ?? 60000));
  const laneOptions = $derived(uniqueLaneOptions(bridge.scene?.lanes));
  const activeDiffFile = $derived(
    selectedDiffFile ||
      diffTabs.find((t) => t.isActive)?.file ||
      (bridge.scene?.path ? String(bridge.scene.path).split(/[\\/]/).pop() : null)
  );

  const confirm = request => bridge.callAction("confirm", request);

  async function refreshAll() {
    const [nextStatus, nextTabs] = await Promise.all([
      invoke("shared_base_action", { action: { kind: "status" } }),
      invoke("shared_base_action", { action: { kind: "diff_projections", leniencyMs } }),
    ]);
    if (nextStatus) status = nextStatus;
    if (Array.isArray(nextTabs)) {
      diffTabs = nextTabs;
      if (!selectedDiffFile) {
        selectedDiffFile = nextTabs.find((t) => t.isActive)?.file ?? nextTabs[0]?.file ?? null;
      }
    }
    await Promise.all([refreshRules(), refreshEnvelope()]);
  }

  async function refreshRules() {
    const res = await invoke("shared_base_action", { action: { kind: "rules", diffFile: activeDiffFile } });
    rules = Array.isArray(res) ? res : [];
  }

  async function refreshEnvelope() {
    const diffFile = volumeScope === "diff" ? activeDiffFile : null;
    const env = await invoke("shared_base_action", { action: { kind: "volume_envelope", diffFile } });
    envelopePoints = Array.isArray(env?.points) ? env.points : [];
    selectedPointIdx = envelopePoints.length ? 0 : null;
  }

  async function openSharedBase() {
    if (!hasDoc) return;
    error = "";
    notice = "";
    open = true;
    busy = true;
    try {
      await refreshAll();
      if (!ruleLane && laneOptions.length) ruleLane = laneOptions[0];
      if (!ruleToLane && laneOptions.length > 1) ruleToLane = laneOptions[1];
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function toggleEnabled(event) {
    if (busy) return;
    const enabled = Boolean(event.currentTarget.checked);
    busy = true;
    error = "";
    notice = "";
    try {
      const next = await invoke("shared_base_action", { action: { kind: "set_enabled", enabled } });
      if (next) status = next;
      bridge.bumpSharedBaseRev();
      notice = enabled
        ? "Shared Base is on for this mapset. Edits on this difficulty are tracked as overrides."
        : "Shared Base is off for this mapset.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function captureBase() {
    if (busy) return;
    busy = true;
    error = "";
    notice = "";
    try {
      if (!(await confirmCaptureFirst({ invoke, confirm }))) return;
      const next = await invoke("shared_base_action", { action: { kind: "capture" } });
      if (next) status = next;
      bridge.bumpSharedBaseRev();
      await refreshAll();
      notice = `Captured ${next?.baseMomentCount ?? 0} moments as the mapset Shared Base.`;
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function applyBase() {
    if (busy || !status.hasBase) return;
    busy = true;
    error = "";
    notice = "";
    try {
      const result = await invoke("shared_base_action", {
        action: { kind: "apply", ...applyArgs({ leniencyMs, overwriteOverrides }) },
      });
      if (result?.status) status = result.status;
      notice = formatApplySummary(result);
      await act(null);
      bridge.showToast(applyToast(false, result));
      bridge.bumpSharedBaseRev();
      await refreshAll();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function applyBaseAll() {
    if (busy || !status.hasBase) return;
    busy = true;
    error = "";
    notice = "";
    try {
      const others = (bridge.scene?.difficulties?.length ?? 1) - 1;
      if (!(await askFirst(confirm, applyAllConfirm(others)))) return;
      const result = await invoke("shared_base_action", {
        action: { kind: "apply_all", ...applyArgs({ leniencyMs, overwriteOverrides }) },
      });
      if (result?.status) status = result.status;
      notice = formatApplyAllSummary(result);
      if (Array.isArray(result?.failedDiffs) && result.failedDiffs.length > 0) {
        error = result.failedDiffs.join(" | ");
      }
      await act(null);
      bridge.showToast(applyToast(true, result));
      bridge.bumpSharedBaseRev();
      await refreshAll();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function resetOverrides() {
    if (busy) return;
    busy = true;
    error = "";
    notice = "";
    try {
      if (!(await askFirst(confirm, RESET_OVERRIDES_CONFIRM))) return;
      const next = await invoke("shared_base_action", { action: { kind: "clear_overrides" } });
      if (next) status = next;
      bridge.bumpSharedBaseRev();
      notice = "Cleared per-difficulty overrides for this difficulty.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function clearBase() {
    if (busy) return;
    busy = true;
    error = "";
    notice = "";
    try {
      if (!(await askFirst(confirm, CLEAR_BASE_CONFIRM))) return;
      const next = await invoke("shared_base_action", { action: { kind: "clear" } });
      if (next) status = next;
      bridge.bumpSharedBaseRev();
      await refreshAll();
      notice = "Cleared Shared Base for this mapset.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function switchDiffTab(tabItem) {
    selectedDiffFile = tabItem.file;
    await Promise.all([refreshRules(), refreshEnvelope()]);
  }

  function openDiffInEditor(path) {
    closeModal();
    bridge.callAction("openSiblingDifficulty", path);
  }

  async function addRule() {
    if (busy || !ruleLane.trim()) return;
    const range = ruleUseRange
      ? { startMs: Math.round(Number(ruleStartMs) || 0), endMs: Math.round(Number(ruleEndMs) || 0) }
      : null;
    let rule;
    if (ruleOp === "mute_lane") {
      rule = { op: "mute_lane", lane: ruleLane.trim(), range };
    } else if (ruleOp === "replace_lane") {
      if (!ruleToLane.trim()) return;
      rule = { op: "replace_lane", from: ruleLane.trim(), to: ruleToLane.trim(), range };
    } else if (ruleOp === "add_hit") {
      rule = {
        op: "add_hit",
        lane: ruleLane.trim(),
        timeMs: Math.round(Number(ruleTimeMs) || 0),
        volume: Math.max(5, Math.min(100, Math.round(Number(ruleVolume) || 80))),
      };
    } else {
      rule = {
        op: "remove_hit",
        lane: ruleLane.trim(),
        timeMs: Math.round(Number(ruleTimeMs) || 0),
      };
    }
    busy = true;
    error = "";
    try {
      rules = (await invoke("shared_base_action", { action: { kind: "add_rule", diffFile: activeDiffFile, rule } })) ?? [];
      bridge.bumpSharedBaseRev();
      await refreshAll();
      notice = "Saved per-difficulty lane rule.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function removeRule(index) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      rules = (await invoke("shared_base_action", { action: { kind: "remove_rule", diffFile: activeDiffFile, index } })) ?? [];
      bridge.bumpSharedBaseRev();
      await refreshAll();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  function formatRule(rule) {
    const r = rule.range ? ` (${rule.range.startMs}–${rule.range.endMs} ms)` : " (whole difficulty)";
    if (rule.op === "mute_lane") return `Mute ${rule.lane}${r}`;
    if (rule.op === "replace_lane") return `Replace ${rule.from} → ${rule.to}${r}`;
    if (rule.op === "add_hit") return `Add ${rule.lane} at ${rule.timeMs} ms (${rule.volume}%)`;
    if (rule.op === "remove_hit") return `Remove ${rule.lane} at ${rule.timeMs} ms`;
    return `${rule.op}`;
  }

  async function saveEnvelope(points) {
    const diffFile = volumeScope === "diff" ? activeDiffFile : null;
    const sorted = [...points]
      .map((p) => ({
        timeMs: Math.max(0, Math.round(Number(p.timeMs) || 0)),
        volume: Math.max(5, Math.min(100, Math.round(Number(p.volume) || 100))),
        curve: p.curve === "step" ? "step" : "linear",
      }))
      .sort((a, b) => a.timeMs - b.timeMs);
    busy = true;
    error = "";
    try {
      const saved = await invoke("shared_base_action", {
        action: {
          kind: "set_volume_envelope",
          diffFile,
          envelope: { points: sorted },
        },
      });
      envelopePoints = saved?.points ?? [];
      bridge.bumpSharedBaseRev();
      notice = envelopePoints.length
        ? `Saved volume curve (${envelopePoints.length} points, green-line only).`
        : "Cleared volume curve.";
      await refreshAll();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  function addEnvelopePoint() {
    const next = [
      ...envelopePoints.filter((p) => p.timeMs !== Math.round(Number(newPtTime) || 0)),
      { timeMs: Math.round(Number(newPtTime) || 0), volume: Math.round(Number(newPtVol) || 80), curve: newPtCurve },
    ];
    saveEnvelope(next);
  }

  function onCurveSvgClick(event) {
    if (!event.ctrlKey) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const xRatio = Math.max(0, Math.min(1, (event.clientX - rect.left) / Math.max(1, rect.width)));
    const yRatio = Math.max(0, Math.min(1, 1 - (event.clientY - rect.top) / Math.max(1, rect.height)));
    const timeMs = Math.round(xRatio * durationMs);
    const volume = Math.max(5, Math.min(100, Math.round(5 + yRatio * 95)));
    saveEnvelope([
      ...envelopePoints.filter((p) => p.timeMs !== timeMs),
      { timeMs, volume, curve: "linear" },
    ]);
  }

  function togglePointCurve(idx) {
    const next = envelopePoints.map((p, i) =>
      i === idx ? { ...p, curve: p.curve === "step" ? "linear" : "step" } : p
    );
    selectedPointIdx = idx;
    saveEnvelope(next);
  }

  function updateSelectedPointVolume(val) {
    if (selectedPointIdx == null || !envelopePoints[selectedPointIdx]) return;
    const next = envelopePoints.map((p, i) =>
      i === selectedPointIdx ? { ...p, volume: Math.max(5, Math.min(100, Math.round(val))) } : p
    );
    saveEnvelope(next);
  }

  function removeEnvelopePoint(idx) {
    saveEnvelope(envelopePoints.filter((_, i) => i !== idx));
    selectedPointIdx = null;
  }

  const svgPolyline = $derived.by(() => {
    if (!envelopePoints.length) return "0,4 360,4";
    const W = 360;
    const H = 96;
    const toX = (t) => Math.max(0, Math.min(W, (t / durationMs) * W));
    const toY = (v) => H - ((Math.max(5, Math.min(100, v)) - 5) / 95) * (H - 8) - 4;
    const pts = [];
    pts.push(`${0},${toY(envelopePoints[0].volume).toFixed(1)}`);
    for (let i = 0; i < envelopePoints.length; i++) {
      const p = envelopePoints[i];
      const x = toX(p.timeMs);
      const y = toY(p.volume);
      pts.push(`${x.toFixed(1)},${y.toFixed(1)}`);
      if (p.curve === "step" && i + 1 < envelopePoints.length) {
        const nextX = toX(envelopePoints[i + 1].timeMs);
        pts.push(`${nextX.toFixed(1)},${y.toFixed(1)}`);
      }
    }
    const last = envelopePoints[envelopePoints.length - 1];
    pts.push(`${W},${toY(last.volume).toFixed(1)}`);
    return pts.join(" ");
  });

  function closeModal() {
    if (busy) return;
    open = false;
    bridge.bumpSharedBaseRev();
  }

  // A confirm opened above this modal takes the Escape for itself.
  function onKeydown(event) {
    if (open && event.key === "Escape" && bridge.onlyModalOpen("shared-base")) {
      event.stopPropagation();
      closeModal();
    }
  }

  $effect(() => {
    bridge.setModalOpen("shared-base", open);
    return () => { bridge.setModalOpen("shared-base", false); };
  });

  onMount(() => {
    return bridge.registerModal("openSharedBase", openSharedBase);
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div class="sb-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal(); }}>
    <div class="sb-modal" role="dialog" aria-modal="true" aria-label="Shared Base layer">
      <header class="sb-head">
        <span class="sb-title">Shared Base layer</span>
        <div class="sb-tabs" role="tablist" aria-label="Shared Base views">
          <button type="button" role="tab" aria-selected={tab === "overview"} class:active={tab === "overview"}
            onclick={() => (tab = "overview")}>Live diff tabs</button>
          <button type="button" role="tab" aria-selected={tab === "rules"} class:active={tab === "rules"}
            onclick={() => (tab = "rules")}>Lane rules ({rules.length})</button>
          <button type="button" role="tab" aria-selected={tab === "volume"} class:active={tab === "volume"}
            onclick={() => (tab = "volume")}>Volume curve ({envelopePoints.length})</button>
        </div>
        <button type="button" class="sb-close" aria-label="Close" onclick={closeModal}>×</button>
      </header>

      <div class="sb-body">
        {#if diffTabs.length > 0}
          <div class="sb-diff-strip" role="group" aria-label="Mapset difficulties">
            {#each diffTabs as dt (dt.file)}
              <button type="button" class="sb-diff-pill" class:selected={dt.file === activeDiffFile}
                class:current={dt.isActive}
                title={`${dt.version}: ${dt.matched} matched, ${dt.empty} empty, ${dt.orphaned} orphaned`}
                onclick={() => switchDiffTab(dt)}>
                <strong>{dt.version}</strong>
                <span class="sb-pill-meta">{dt.matched}✓ {dt.empty}○ {dt.orphaned}!</span>
                {#if dt.ruleCount > 0}<span class="sb-badge">{dt.ruleCount}R</span>{/if}
                {#if dt.hasVolumeEnvelope}<span class="sb-badge">V</span>{/if}
              </button>
            {/each}
          </div>
        {/if}

        {#if tab === "overview"}
          <label class="sb-check">
            <input type="checkbox" checked={status.enabled} disabled={busy} onchange={toggleEnabled} />
            <span>Use Shared Base for this mapset</span>
          </label>

          <div class="sb-summary">
            <div class="sb-summary-main">{formatStatusLine(status)}</div>
            {#if status.hasBase}
              <div class="sb-summary-counts">
                <span>Inherited on this difficulty: {status.currentDiffInherited}</span>
                <span>Overridden on this difficulty: {status.currentDiffOverrides}</span>
              </div>
            {/if}
          </div>

          {#if diffTabs.length > 0}
            <div class="sb-diff-table">
              {#each diffTabs as dt (dt.file)}
                <div class="sb-diff-row" class:current={dt.isActive}>
                  <span class="sb-diff-name">{dt.version}{dt.isActive ? " (open)" : ""}</span>
                  <span class="sb-diff-counts">{dt.matched} matched · {dt.empty} empty · {dt.orphaned} orphaned</span>
                  {#if !dt.isActive}
                    <button type="button" class="sb-mini-btn" onclick={() => openDiffInEditor(dt.path)}>Open</button>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}

          <div class="sb-section">
            <div class="sb-row">
              <button type="button" disabled={busy} onclick={captureBase}>
                Capture current difficulty as Base
              </button>
            </div>
          </div>

          <div class="sb-section sb-divider">
            <label class="sb-field">
              <span>Match leniency (ms)</span>
              <input type="number" min="0" max={MAX_LENIENCY_MS} step="1"
                disabled={busy || !status.hasBase} bind:value={leniencyMs} />
            </label>
            <label class="sb-check">
              <input type="checkbox" disabled={busy || !status.hasBase} bind:checked={overwriteOverrides} />
              <span>Overwrite per-difficulty overrides</span>
            </label>
            <div class="sb-row">
              <button type="button" class="sb-primary" disabled={busy || !status.hasBase} onclick={applyBase}>
                {busy ? "Working…" : "Apply Base to this difficulty"}
              </button>
              <button type="button" disabled={busy || !status.hasBase} onclick={applyBaseAll}>
                {busy ? "Working…" : "Apply Base to all difficulties"}
              </button>
            </div>
            <p class="sb-hint">
              "Apply Base to all difficulties" applies each difficulty's lane rules and green-line volume curve and saves sibling <code>.osu</code> files with backups.
            </p>
          </div>

          <div class="sb-section sb-divider sb-actions-row">
            <button type="button" disabled={busy || status.currentDiffOverrides === 0} onclick={resetOverrides}>
              Reset overrides
            </button>
            <button type="button" class="sb-danger" disabled={busy || (!status.hasBase && !status.enabled)} onclick={clearBase}>
              Clear Shared Base
            </button>
          </div>

        {:else if tab === "rules"}
          <p class="sb-hint">
            Non-destructive lane rules for <strong>{activeDiffFile ?? "this difficulty"}</strong>. Rules layer on top of the Shared Base without changing other difficulties.
          </p>
          <div class="sb-rule-form">
            <select aria-label="Rule action" bind:value={ruleOp}>
              <option value="mute_lane">Mute lane</option>
              <option value="replace_lane">Replace lane</option>
              <option value="add_hit">Add hit</option>
              <option value="remove_hit">Remove hit</option>
            </select>
            <input type="text" list="sb-lanes-list" placeholder="Lane name (e.g. soft-hitclap)" bind:value={ruleLane} />
            {#if ruleOp === "replace_lane"}
              <input type="text" list="sb-lanes-list" placeholder="Replace with lane…" bind:value={ruleToLane} />
            {/if}
            {#if ruleOp === "mute_lane" || ruleOp === "replace_lane"}
              <label class="sb-check">
                <input type="checkbox" bind:checked={ruleUseRange} />
                <span>Limit to time range (ms)</span>
              </label>
              {#if ruleUseRange}
                <div class="sb-range-row">
                  <input type="number" min="0" step="100" aria-label="Start ms" bind:value={ruleStartMs} />
                  <span>to</span>
                  <input type="number" min="0" step="100" aria-label="End ms" bind:value={ruleEndMs} />
                  <span>ms</span>
                </div>
              {/if}
            {:else}
              <div class="sb-range-row">
                <input type="number" min="0" step="10" aria-label="Time ms" bind:value={ruleTimeMs} />
                <span>ms</span>
                {#if ruleOp === "add_hit"}
                  <input type="number" min="5" max="100" step="5" aria-label="Volume %" bind:value={ruleVolume} />
                  <span>%</span>
                {/if}
              </div>
            {/if}
            <button type="button" class="sb-primary" disabled={busy || !ruleLane.trim()} onclick={addRule}>
              Add rule
            </button>
            <datalist id="sb-lanes-list">
              {#each laneOptions as opt (opt)}
                <option value={opt}></option>
              {/each}
            </datalist>
          </div>

          <ul class="sb-rule-list">
            {#if !rules.length}
              <li class="sb-empty">No rules for this difficulty yet.</li>
            {:else}
              {#each rules as rule, idx (idx)}
                <li class="sb-rule-item">
                  <span>{formatRule(rule)}</span>
                  <button type="button" class="sb-mini-btn" disabled={busy} onclick={() => removeRule(idx)}>Remove</button>
                </li>
              {/each}
            {/if}
          </ul>

        {:else if tab === "volume"}
          <div class="sb-row">
            <label class="sb-field">
              <span>Scope</span>
              <select value={volumeScope} onchange={(e) => { volumeScope = e.currentTarget.value; refreshEnvelope(); }}>
                <option value="shared">Whole mapset (shared)</option>
                <option value="diff">{activeDiffFile ?? "Current difficulty"} only</option>
              </select>
            </label>
            <button type="button" disabled={busy || !envelopePoints.length} onclick={() => saveEnvelope([])}>
              Clear curve
            </button>
          </div>
          <p class="sb-hint">
            Scales only <code>[TimingPoints]</code> green lines on export (0 extra sample mixes). <strong>Ctrl+click</strong> the graph to add a point; click any point to flip Linear ramp ↔ Step hold.
          </p>
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
          <svg class="sb-curve-svg" viewBox="0 0 360 96" onclick={onCurveSvgClick} role="img" aria-label="Master volume envelope curve">
            <line x1="0" y1="48" x2="360" y2="48" stroke="rgba(255,255,255,0.12)" stroke-dasharray="4 4" />
            <polyline fill="none" stroke="var(--editor-accent, #4a9eff)" stroke-width="2" points={svgPolyline} />
            {#each envelopePoints as pt, idx (`${pt.timeMs}-${idx}`)}
              {@const cx = Math.max(4, Math.min(356, (pt.timeMs / durationMs) * 360))}
              {@const cy = 96 - ((Math.max(5, Math.min(100, pt.volume)) - 5) / 95) * 88 - 4}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <circle cx={cx} cy={cy} r={selectedPointIdx === idx ? 5.5 : 4.5}
                fill={pt.curve === "step" ? "#f59e0b" : "var(--editor-accent, #4a9eff)"}
                stroke="#0f1216" stroke-width="1.5"
                onclick={(e) => {
                  e.stopPropagation();
                  if (selectedPointIdx !== idx) {
                    selectedPointIdx = idx;
                  } else {
                    togglePointCurve(idx);
                  }
                }}>
                <title>{pt.timeMs} ms · {pt.volume}% · {pt.curve === "step" ? "Step" : "Linear"} (click to select; click again to flip Linear/Step)</title>
              </circle>
            {/each}
          </svg>

          {#if selectedPointIdx != null && envelopePoints[selectedPointIdx]}
            {@const sel = envelopePoints[selectedPointIdx]}
            <div class="sb-bubble" role="group" aria-label="Selected volume point">
              <span>Point at {sel.timeMs} ms ({sel.curve}):</span>
              <input type="number" min="5" max="100" step="1" aria-label="Point volume percent"
                value={sel.volume}
                onchange={(e) => updateSelectedPointVolume(e.currentTarget.valueAsNumber)} />
              <span>%</span>
              <button type="button" class="sb-mini-btn" onclick={() => togglePointCurve(selectedPointIdx)}>
                Flip {sel.curve === "step" ? "→ Linear" : "→ Step"}
              </button>
              <button type="button" class="sb-mini-btn" onclick={() => removeEnvelopePoint(selectedPointIdx)}>Delete</button>
            </div>
          {/if}

          <div class="sb-range-row">
            <input type="number" min="0" step="250" aria-label="New point ms" bind:value={newPtTime} />
            <span>ms</span>
            <input type="number" min="5" max="100" step="5" aria-label="New point volume %" bind:value={newPtVol} />
            <span>%</span>
            <select aria-label="Curve kind" bind:value={newPtCurve}>
              <option value="linear">Linear ramp</option>
              <option value="step">Step hold</option>
            </select>
            <button type="button" class="sb-primary" disabled={busy} onclick={addEnvelopePoint}>Add point</button>
          </div>
        {/if}

        {#if error}
          <p class="sb-error" role="alert">{error}</p>
        {:else if notice}
          <p class="sb-notice" role="status">{notice}</p>
        {/if}
      </div>

      <footer class="sb-foot">
        <button type="button" disabled={busy} onclick={closeModal}>Close</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .sb-backdrop {
    position: fixed;
    inset: 0;
    z-index: 62;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.68);
  }
  .sb-modal {
    display: flex;
    flex-direction: column;
    width: min(620px, calc(100vw - 40px));
    max-height: min(660px, calc(100vh - 48px));
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.62);
    overflow: hidden;
  }
  .sb-head {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 48px;
    padding: 0 16px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .sb-title { font-size: 14px; font-weight: 600; }
  .sb-tabs { display: flex; gap: 4px; margin-left: auto; }
  .sb-tabs button {
    padding: 4px 9px;
    font-size: 12px;
    border-radius: 6px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--editor-muted);
    cursor: pointer;
  }
  .sb-tabs button.active {
    background: var(--editor-control);
    border-color: var(--editor-border);
    color: var(--editor-text);
  }
  .sb-close {
    width: 28px; height: 28px; border: 0; border-radius: 6px;
    background: transparent; color: var(--editor-muted); font-size: 18px; cursor: pointer;
  }
  .sb-body {
    display: flex; flex-direction: column; gap: 12px; padding: 14px 16px; overflow: auto;
  }
  .sb-diff-strip { display: flex; flex-wrap: wrap; gap: 6px; }
  .sb-diff-pill {
    display: inline-flex; align-items: center; gap: 6px; padding: 4px 8px;
    border-radius: 6px; border: 1px solid var(--editor-border);
    background: var(--editor-control); color: var(--editor-text); font-size: 12px; cursor: pointer;
  }
  .sb-diff-pill.selected { border-color: var(--editor-accent); }
  .sb-pill-meta { font-size: 11px; color: var(--editor-muted); }
  .sb-badge {
    padding: 0 4px; border-radius: 4px; background: var(--editor-accent);
    color: #111; font-size: 10px; font-weight: 700;
  }
  .sb-diff-table { display: flex; flex-direction: column; gap: 4px; }
  .sb-diff-row {
    display: flex; align-items: center; gap: 8px; padding: 5px 8px;
    border-radius: 6px; background: rgba(255, 255, 255, 0.03);
    font-size: 12px;
  }
  .sb-diff-name { font-weight: 600; }
  .sb-diff-counts { color: var(--editor-muted); margin-left: auto; font-size: 11px; }
  .sb-mini-btn {
    padding: 2px 8px; border-radius: 4px; border: 1px solid var(--editor-border);
    background: var(--editor-control); color: var(--editor-text); font-size: 11px; cursor: pointer;
  }
  .sb-summary {
    padding: 10px 12px; border-radius: 8px;
    background: var(--editor-chrome);
    border: 1px solid var(--editor-border); font-size: 12px;
  }
  .sb-summary-main { font-weight: 600; }
  .sb-summary-counts { display: flex; gap: 14px; margin-top: 4px; color: var(--editor-muted); }
  .sb-section { display: flex; flex-direction: column; gap: 8px; }
  .sb-divider { border-top: 1px solid var(--editor-border); padding-top: 10px; }
  .sb-row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .sb-actions-row { flex-direction: row; justify-content: space-between; }
  .sb-check { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; }
  .sb-field { display: inline-flex; align-items: center; gap: 8px; font-size: 12px; }
  .sb-hint { margin: 0; font-size: 12px; color: var(--editor-muted); line-height: 1.4; }
  .sb-primary {
    padding: 6px 12px; border-radius: 6px; border: 0;
    background: var(--editor-accent); color: #111; font-weight: 600; font-size: 12px; cursor: pointer;
  }
  .sb-danger {
    padding: 6px 12px; border-radius: 6px; border: 1px solid #ef4444;
    background: transparent; color: #f87171; font-size: 12px; cursor: pointer;
  }
  .sb-rule-form { display: flex; flex-direction: column; gap: 8px; }
  .sb-range-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; font-size: 12px; }
  .sb-range-row input { width: 84px; }
  .sb-rule-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  .sb-rule-item {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 6px 9px; border-radius: 6px; border: 1px solid var(--editor-border); font-size: 12px;
  }
  .sb-empty { color: var(--editor-muted); font-size: 12px; }
  .sb-curve-svg {
    width: 100%; height: 108px; border-radius: 8px;
    border: 1px solid var(--editor-border); background: #0e1116; cursor: crosshair;
  }
  .sb-curve-svg circle { cursor: pointer; }
  .sb-bubble {
    display: flex; align-items: center; gap: 8px; padding: 6px 10px;
    border-radius: 6px; border: 1px solid var(--editor-accent);
    background: var(--editor-selected); font-size: 12px;
  }
  .sb-bubble input { width: 62px; }
  .sb-error { margin: 0; font-size: 12px; color: #f87171; }
  .sb-notice { margin: 0; font-size: 12px; color: #60a5fa; }
  .sb-foot {
    display: flex; justify-content: flex-end; padding: 10px 16px;
    border-top: 1px solid var(--editor-border); background: var(--editor-chrome);
  }
</style>
