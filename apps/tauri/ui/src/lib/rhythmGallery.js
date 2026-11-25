// Musical rhythm presets for Pattern Stamp, derived from the active bar length in ms.

export function activeBarMs(scene) {
  const step = Number(scene?.patternStampStep);
  if (Number.isFinite(step) && step > 0) return step;
  const beatMs = Number(scene?.beatLengthMs);
  const meter = Number(scene?.beatsPerBar) || 4;
  if (Number.isFinite(beatMs) && beatMs > 0) return beatMs * meter;
  return 2000;
}

const roundMs = value => Math.max(1, Math.round(value * 1000) / 1000);

export function rhythmPresets(barMs = 2000, beatsPerBar = 4) {
  const safeBar = Number.isFinite(barMs) && barMs > 0 ? barMs : 2000;
  const safeMeter = Number.isFinite(beatsPerBar) && beatsPerBar >= 1 ? beatsPerBar : 4;
  const beatMs = safeBar / safeMeter;
  return [
    {
      id: "half-bar",
      label: "1 & 3",
      modalLabel: "Beats 1 & 3 (half-bar)",
      stepMs: roundMs(safeBar / 2),
      count: 4,
      title: "Stamp every half-bar (beats 1 & 3 or 2 & 4), 4 times",
    },
    {
      id: "bar",
      label: "Bar",
      modalLabel: "Every bar",
      stepMs: roundMs(safeBar),
      count: 4,
      title: "Stamp once per bar, 4 times",
    },
    {
      id: "beat",
      label: "1/1",
      modalLabel: "Every beat (1/1)",
      stepMs: roundMs(beatMs),
      count: 8,
      title: "Stamp every beat (1/1), 8 times",
    },
    {
      id: "offbeat",
      label: "1/2",
      modalLabel: "Offbeats (1/2)",
      stepMs: roundMs(beatMs / 2),
      count: 8,
      title: "Stamp every half-beat (1/2), 8 times",
    },
    {
      id: "roll",
      label: "1/4",
      modalLabel: "1/4 roll",
      stepMs: roundMs(beatMs / 4),
      count: 8,
      title: "Stamp every quarter-beat (1/4 roll), 8 times",
    },
    {
      id: "two-bars",
      label: "2 bars",
      modalLabel: "Every 2 bars",
      stepMs: roundMs(safeBar * 2),
      count: 4,
      title: "Stamp once every 2 bars, 4 times",
    },
  ];
}

export function rhythmStampAction(preset, { snapToObjects = true, quantizeVolumes = false } = {}) {
  return {
    kind: "pattern_stamp",
    count: preset.count,
    step_ms: preset.stepMs,
    snap_to_objects: snapToObjects,
    quantize_volumes: quantizeVolumes,
  };
}
