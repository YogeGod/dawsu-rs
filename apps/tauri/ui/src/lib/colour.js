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
