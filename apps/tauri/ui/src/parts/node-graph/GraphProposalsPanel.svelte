<script>
  import { visibleViewerOverlay } from "../../lib/node-graph.js";

  let {
    view,
    statusNotice = "",
    checklistOpen = $bindable(false),
    ondispatch,
    onrunrange,
  } = $props();

  let previewCanvas = $state(null);

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
      <button type="button" class="ng-btn" onclick={() => ondispatch({ kind: "set_viewer_target", node: null, socket: null })}>Clear Preview</button>
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
    <button type="button" class="ng-btn" onclick={() => ondispatch({ kind: "cancel" })}>Cancel</button>
  {:else if view?.batch?.stale}
    <span class="ng-banner-warn">Map or graph changed since last run. Run again before applying.</span>
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
    <button type="button" class="ng-btn" onclick={() => ondispatch({ kind: "select_last_hits" }, "Selected last applied hits in editor.")}>
      Select Last Hits
    </button>
  {/if}
  <button
    type="button"
    class="ng-btn ng-btn-teal"
    disabled={!view?.hasDocument || !view?.hasLocatorRange}
    onclick={() => onrunrange(true)}
  >
    Test L–R Loop
  </button>
  <button
    type="button"
    class="ng-btn"
    disabled={!view?.hasDocument}
    onclick={() => onrunrange(false)}
  >
    Run Whole Song
  </button>
  <button
    type="button"
    class="ng-btn ng-btn-primary"
    disabled={!view?.batch || view.batch.stale || !view.batch.counts?.checked}
    onclick={() => ondispatch({ kind: "apply" }, "Applied hits to map (Ctrl+Z in editor to undo).")}
  >
    Apply {view?.batch?.counts?.checked ?? 0} Hits
  </button>
</footer>

{#if checklistOpen && view?.batch}
  <div class="ng-checklist-overlay" role="dialog" aria-label="Review proposed hits">
    <div class="ng-checklist-card">
      <div class="ng-header">
        <strong>Review Proposed Hits ({view.batch.counts.checked} / {view.batch.counts.total})</strong>
        <div class="ng-spacer"></div>
        <button type="button" class="ng-btn" onclick={() => ondispatch({ kind: "set_all_proposals", checked: true })}>All</button>
        <button type="button" class="ng-btn" onclick={() => ondispatch({ kind: "set_all_proposals", checked: false })}>None</button>
        <button type="button" class="ng-btn" onclick={() => { checklistOpen = false; }}>Close</button>
      </div>
      <div class="ng-checklist-rows">
        {#each view.batch.proposals as item (item.id)}
          <label class="ng-check-row">
            <span>
              <input
                type="checkbox"
                checked={item.checked}
                onchange={(e) => ondispatch({ kind: "toggle_proposal", id: item.id, checked: e.currentTarget.checked })}
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
