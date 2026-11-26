// Pure helpers for mixer VST3 FX strip state, labels, wet/dry, and PDC latency formatting.

export function stripFxKey(kind, lane = null) {
  if (kind === "song" || kind === "hitsounds" || kind === "master") {
    return kind;
  }
  if (kind === "lane" && lane) {
    if (typeof lane.fxKey === "string" && lane.fxKey.startsWith("lane:")) {
      return lane.fxKey;
    }
    if (typeof lane.colourKey === "string" && lane.colourKey.length > 0) {
      return "lane:" + lane.colourKey;
    }
    if (typeof lane.sourceKey === "string" && lane.sourceKey.length > 0) {
      return "lane:" + lane.sourceKey;
    }
  }
  return null;
}

export function normalizeMixerFxState(raw) {
  if (!raw || typeof raw !== "object") {
    return {
      revision: 0,
      totalPdcSamples: 0,
      sampleRate: 44100,
      canUndo: false,
      canRedo: false,
      sidecarAvailable: false,
      availableEffects: [],
      strips: {},
    };
  }
  return {
    revision: Number(raw.revision ?? 0),
    totalPdcSamples: Number(raw.totalPdcSamples ?? 0),
    sampleRate: Math.max(1, Number(raw.sampleRate ?? 44100)),
    canUndo: !!raw.canUndo,
    canRedo: !!raw.canRedo,
    sidecarAvailable: !!raw.sidecarAvailable,
    availableEffects: Array.isArray(raw.availableEffects) ? raw.availableEffects : [],
    strips: raw.strips && typeof raw.strips === "object" ? raw.strips : {},
  };
}

export function wetDryPercent(wetDry) {
  const num = Number(wetDry);
  if (!Number.isFinite(num)) return 100;
  return Math.round(Math.min(1, Math.max(0, num)) * 100);
}

export function latencyLabel(samples, sampleRate = 44100) {
  const n = Math.max(0, Math.round(Number(samples) || 0));
  const sr = Math.max(1, Number(sampleRate) || 44100);
  if (n === 0) return "0 smp";
  const ms = (n * 1000) / sr;
  return `${n} smp (${ms.toFixed(1)} ms)`;
}

export function fxSlotSummary(slots) {
  const list = Array.isArray(slots) ? slots : [];
  let active = 0;
  let bypassed = 0;
  let totalLatency = 0;
  for (const slot of list) {
    if (slot?.bypassed) bypassed += 1;
    else active += 1;
    totalLatency += Math.max(0, Number(slot?.latencySamples) || 0);
  }
  return {
    total: list.length,
    active,
    bypassed,
    totalLatency,
  };
}
