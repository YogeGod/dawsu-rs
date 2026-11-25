<script>
  import { budgetTone, formatMegabytes, OSZ_LIMIT_BYTES } from "../lib/bounce.js";

  let { baseBytes = 0, bouncedBytes = 0, limitBytes = OSZ_LIMIT_BYTES } = $props();

  const totalBytes = $derived(baseBytes + bouncedBytes);
  const pct = $derived(Math.min(100, Math.round((totalBytes / Math.max(1, limitBytes)) * 100)));
  const tone = $derived(budgetTone(totalBytes, limitBytes));
  const label = $derived(
    tone === "over" ? "Over 10.0 MB ranking ceiling" : tone === "caution" ? "Near 10.0 MB ceiling" : "Ranking safe",
  );
</script>

<div class="osz-gauge" data-tone={tone}>
  <div class="osz-gauge-head">
    <span>.osz size estimate: <strong>{formatMegabytes(totalBytes)}</strong> / {formatMegabytes(limitBytes)}</span>
    <span class="osz-gauge-tag">{label}</span>
  </div>
  <div class="osz-track" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct}>
    <span class="osz-fill" style:--fill="{pct}%"></span>
  </div>
</div>

<style>
  .osz-gauge {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
  }
  .osz-gauge-head {
    display: flex;
    justify-content: space-between;
    font-variant-numeric: tabular-nums;
  }
  .osz-track {
    height: 6px;
    border-radius: 999px;
    background: var(--editor-ink);
    overflow: hidden;
  }
  .osz-fill {
    display: block;
    height: 100%;
    width: var(--fill);
    background: #34c759;
  }
  .osz-gauge[data-tone="caution"] .osz-fill {
    background: #f59e0b;
  }
  .osz-gauge[data-tone="over"] .osz-fill {
    background: #ef4444;
  }
</style>
