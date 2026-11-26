// Pure spectrum calibration, note conversion, magma colormapLUT, and binary 0x02 frame parser.

export const SPECTRUM_BINS = 128;
export const SPECTRUM_MIN_HZ = 20;
export const SPECTRUM_MAX_HZ = 20000;
export const SPECTRUM_FLOOR_DB = -120;

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const LOG_RANGE = Math.log2(SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ);

export function binToFreq(index, count = SPECTRUM_BINS) {
  const denom = Math.max(1, count - 1);
  const t = Math.min(1, Math.max(0, index / denom));
  return SPECTRUM_MIN_HZ * (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ) ** t;
}

export function freqToX(hz, width) {
  if (!(hz > 0) || !(width > 0)) return 0;
  const clamped = Math.min(SPECTRUM_MAX_HZ, Math.max(SPECTRUM_MIN_HZ, hz));
  return (Math.log2(clamped / SPECTRUM_MIN_HZ) / LOG_RANGE) * width;
}

export function xToFreq(x, width) {
  if (!(width > 0)) return SPECTRUM_MIN_HZ;
  const t = Math.min(1, Math.max(0, x / width));
  return SPECTRUM_MIN_HZ * (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ) ** t;
}

export function applyTilt(db, freqHz, tiltDbPerOct = 4.5) {
  if (!Number.isFinite(db) || !(freqHz > 0) || !tiltDbPerOct) return db;
  return db + tiltDbPerOct * Math.log2(freqHz / 1000);
}

export function freqToNote(hz) {
  if (!Number.isFinite(hz) || hz < 16 || hz > 22000) return null;
  const midiF = 69 + 12 * Math.log2(hz / 440);
  const midi = Math.round(midiF);
  if (midi < 0 || midi > 127) return null;
  const cents = Math.round((midiF - midi) * 100);
  const pitch = NOTE_NAMES[((midi % 12) + 12) % 12];
  const octave = Math.floor(midi / 12) - 1;
  return { name: `${pitch}${octave}`, midi, cents };
}

const MAGMA_STOPS = [
  [0.0, [10, 8, 18]],
  [0.25, [59, 15, 112]],
  [0.5, [140, 41, 129]],
  [0.72, [222, 73, 104]],
  [0.88, [254, 159, 109]],
  [1.0, [252, 253, 191]],
];

const magmaLut = (() => {
  const out = new Array(256);
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    let hi = 1;
    while (hi < MAGMA_STOPS.length - 1 && t > MAGMA_STOPS[hi][0]) hi++;
    const [t0, c0] = MAGMA_STOPS[hi - 1];
    const [t1, c1] = MAGMA_STOPS[hi];
    const f = t1 > t0 ? (t - t0) / (t1 - t0) : 0;
    const r = Math.round(c0[0] + (c1[0] - c0[0]) * f);
    const g = Math.round(c0[1] + (c1[1] - c0[1]) * f);
    const b = Math.round(c0[2] + (c1[2] - c0[2]) * f);
    out[i] = `rgb(${r},${g},${b})`;
  }
  return out;
})();

export function magmaColour(norm) {
  const idx = Math.min(255, Math.max(0, Math.round((Number.isFinite(norm) ? norm : 0) * 255)));
  return magmaLut[idx];
}

export function magmaRgb(norm) {
  const t = Math.min(1, Math.max(0, Number.isFinite(norm) ? norm : 0));
  let hi = 1;
  while (hi < MAGMA_STOPS.length - 1 && t > MAGMA_STOPS[hi][0]) hi++;
  const [t0, c0] = MAGMA_STOPS[hi - 1];
  const [t1, c1] = MAGMA_STOPS[hi];
  const f = t1 > t0 ? (t - t0) / (t1 - t0) : 0;
  return [
    Math.round(c0[0] + (c1[0] - c0[0]) * f),
    Math.round(c0[1] + (c1[1] - c0[1]) * f),
    Math.round(c0[2] + (c1[2] - c0[2]) * f),
  ];
}

export function parseSpectrumFrame(payload) {
  let bytes = null;
  if (payload instanceof Uint8Array) bytes = payload;
  else if (ArrayBuffer.isView(payload)) bytes = new Uint8Array(payload.buffer, payload.byteOffset, payload.byteLength);
  else if (typeof payload?.byteLength === "number") bytes = new Uint8Array(payload);
  else if (Array.isArray(payload) && payload.length >= 272) bytes = Uint8Array.from(payload);
  if (!bytes || bytes.byteLength < 272 || bytes[0] !== 0x02) return null;

  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const flags = dv.getUint8(1);
  const binCount = Math.min(SPECTRUM_BINS, dv.getUint16(2, true) || SPECTRUM_BINS);
  const peakHz = dv.getFloat32(4, true);
  const peakDb = dv.getFloat32(8, true);
  const midiNote = dv.getInt16(12, true);
  const cents = dv.getInt16(14, true);
  const binsDb = new Float32Array(SPECTRUM_BINS);
  for (let i = 0; i < binCount; i++) {
    binsDb[i] = dv.getInt16(16 + i * 2, true) / 100;
  }
  return {
    version: 2,
    active: (flags & 1) !== 0,
    hasPitch: (flags & 2) !== 0 && midiNote >= 0,
    peakHz,
    peakDb,
    midiNote: midiNote >= 0 ? midiNote : null,
    cents,
    binsDb,
  };
}
