// Lane and folder colours as hexes (v3 §4, ADR 0068). The webviews on every platform draw hexes,
// so nothing here leaves oklch() or color-mix() to CSS.

const canvas = "#151515"; // --editor-canvas

const toLinear = c => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toGamma = c => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
const byte = c => Math.round(Math.min(1, Math.max(0, toGamma(c))) * 255).toString(16).padStart(2, "0");
const isHex = value => typeof value === "string" && /^#[\da-f]{6}$/i.test(value);

// Björn Ottosson's OKLab.
function toOklch(hex) {
  const [r, g, b] = [1, 3, 5].map(i => toLinear(parseInt(hex.slice(i, i + 2), 16) / 255));
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return [L, Math.hypot(a, B), Math.atan2(B, a) * 180 / Math.PI];
}

export function oklchToHex(L, C, h) {
  const a = C * Math.cos(h * Math.PI / 180);
  const b = C * Math.sin(h * Math.PI / 180);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return "#" + [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ].map(byte).join("");
}

// color-mix(in oklch, colour p%, canvas). The canvas (or another grey base) has no hue, so the
// mix keeps the colour's hue.
export function mixIntoCanvas(colour, percent, base = canvas) {
  if (percent >= 100) return colour.toLowerCase();
  if (percent <= 0) return base;
  const [L1, C1, h] = toOklch(colour);
  const [L2, C2] = toOklch(base);
  const p = percent / 100;
  return oklchToHex(L1 * p + L2 * (1 - p), C1 * p + C2 * (1 - p), h);
}

// v3's lane lightness with no chroma: the lane the mapper has not coloured.
export const laneGrey = oklchToHex(0.70, 0, 0);
// Darker than a lane, so a lane's folder spine and its number strip stay apart.
export const folderGrey = oklchToHex(0.55, 0, 0);

// Strip and number, icon tile 26 %, header 18 %, hover 28 %, selected 36 %.
export function laneShades(colour) {
  const strip = isHex(colour) ? colour.toLowerCase() : laneGrey;
  return {
    strip, tile: mixIntoCanvas(strip, 26), header: mixIntoCanvas(strip, 18),
    hover: mixIntoCanvas(strip, 28), selected: mixIntoCanvas(strip, 36),
  };
}

export const folderShades = () => ({ strip: folderGrey, header: mixIntoCanvas(folderGrey, 34) });

// v3 8b: the Inspector names a picked lane colour by its OKLCH hue. A colour with almost no
// chroma (a grey from the custom picker) has no hue to name.
const tintBands = [[40, "Red"], [70, "Orange"], [100, "Amber"], [125, "Yellow"], [145, "Lime"], [165, "Green"],
  [190, "Teal"], [220, "Cyan"], [255, "Blue"], [285, "Indigo"], [315, "Violet"], [340, "Magenta"], [361, "Pink"]];

export function tintName(colour) {
  const [, chroma, hue] = toOklch(colour.toLowerCase());
  if (chroma < 0.02) return "Grey";
  const degrees = (hue + 360) % 360;
  return tintBands.find(([limit]) => degrees < limit)[1];
}

const LANE_PALETTE_ROWS = [
  "d04932 db642b d78628 deab27 c9bc30 a6be32 70be3d 44ba4b 23b76c 21af91 24abb2 289dc9 388ada 5776d4 7464ce 945ac5 b751bb cc50a2",
  "b0412e b95728 b57326 bb9125 aa9f2c 8da12e 61a137 3d9e43 229b5e 20957c 239197 2686aa 3376b8 4c66b3 6457ae 7e4fa7 9b479e ac478a",
  "91392b 984b26 966124 9a7923 8d8429 76862b 538632 36833b 218150 1f7c68 21797e 24708d 2e6498 435794 554b90 6a458a 813f84 8f3e73",
  "74342a 794126 775125 7a6224 706a29 606b2a 476b2f 326935 226845 226456 236266 255c70 2c5378 3b4976 494173 583c6f 68386a 72385e",
];

export const LANE_COLOUR_PALETTE = Object.freeze(
  LANE_PALETTE_ROWS.flatMap(row => row.split(" ").map(hex => "#" + hex)),
);
const LANE_COLOUR_CYCLE = Object.freeze([null, ...LANE_COLOUR_PALETTE]);

export function cycleLaneColour(current, delta) {
  const normalized = isHex(current) ? current.toLowerCase() : null;
  const step = delta > 0 ? 1 : delta < 0 ? -1 : 0;
  if (!step) return normalized;
  const idx = LANE_COLOUR_CYCLE.indexOf(normalized);
  if (idx < 0) {
    return step > 0 ? LANE_COLOUR_PALETTE[0] : LANE_COLOUR_PALETTE[LANE_COLOUR_PALETTE.length - 1];
  }
  return LANE_COLOUR_CYCLE[(idx + step + LANE_COLOUR_CYCLE.length) % LANE_COLOUR_CYCLE.length];
}
