// osu!lazer's default skin (Argon) for the map preview, drawn in code. Sizes and colours are
// lazer's at 131fe72 (osu.Game.Rulesets.Osu/Skinning/Argon: ArgonMainCirclePiece, ArgonSliderBody,
// ArgonSliderBall, ArgonReverseArrow, ArgonSliderScorePoint, ArgonSpinnerDisc), as dawsu! 3.1.4
// redrew them (PlayfieldView.cs at c81774e). lazer's images (the approach circle texture, the
// Torus digits) are never shipped; plain stand-ins take their place. Every piece is drawn in
// lazer's object units, where a hit circle is 128 across, centred on the origin.

export const DIAMETER = 128;
const BORDER = (DIAMETER * 2) / 58;
const GRADIENT = BORDER * 2.5;
export const OUTER_GRADIENT = DIAMETER - BORDER * 4;
const INNER_GRADIENT = OUTER_GRADIENT - GRADIENT * 2;
const INNER_FILL = INNER_GRADIENT - GRADIENT * 2;
const SPINNER_PINK = "#fc618f";

const clamp01 = value => Math.min(1, Math.max(0, value));
export const outQuint = t => 1 - (1 - clamp01(t)) ** 5;
const outQuad = t => 1 - (1 - clamp01(t)) ** 2;

// lazer's Color4.Darken: each channel divided by 1 + amount.
export function darken(hex, amount) {
  const value = Number.parseInt(hex.slice(1), 16);
  const channel = shift => Math.floor(((value >> shift) & 255) / (1 + amount));
  return `rgb(${channel(16)},${channel(8)},${channel(0)})`;
}

function disc(g, radius, fill) {
  g.beginPath();
  g.arc(0, 0, radius, 0, Math.PI * 2);
  g.fillStyle = fill;
  g.fill();
}

// A top-to-bottom gradient over a circle of the given diameter.
function vertical(g, diameter, top, bottom) {
  const gradient = g.createLinearGradient(0, -diameter / 2, 0, diameter / 2);
  gradient.addColorStop(0, top);
  gradient.addColorStop(1, bottom);
  return gradient;
}

// A ring as lazer's masked border: the stroke lies inside the given diameter.
export function ring(g, colour, diameter, thickness) {
  g.beginPath();
  g.arc(0, 0, Math.max(0, (diameter - thickness) / 2), 0, Math.PI * 2);
  g.lineWidth = thickness;
  g.strokeStyle = colour;
  g.stroke();
}

// ArgonMainCirclePiece: dark fill, bright outer gradient, darker inner gradient, the dark centre
// with the combo number, and the white border on top. A slider head has no outer dark fill.
export function circle(g, accent, number, outerFill = true) {
  const fill = darken(accent, 4);
  if (outerFill) disc(g, (DIAMETER - 1) / 2, fill);
  disc(g, OUTER_GRADIENT / 2, vertical(g, OUTER_GRADIENT, accent, darken(accent, 0.1)));
  disc(g, INNER_GRADIENT / 2, vertical(g, INNER_GRADIENT, darken(accent, 0.5), darken(accent, 0.6)));
  disc(g, INNER_FILL / 2, fill);
  if (number > 0) {
    g.fillStyle = "#ffffff";
    g.font = "700 44px system-ui, sans-serif";
    g.textAlign = "center";
    g.textBaseline = "middle";
    g.fillText(String(number), 0, -2);
  }
  ring(g, "#ffffff", DIAMETER, BORDER);
}

// The editor's hit marker (HitCircleOverlapMarker) after a circle's time: a ring in the combo
// colour that grows a little and fades. `passed` is 0 at the circle's time, 1 at the fade's end.
export function hitMarker(g, accent, passed) {
  const alpha = 0.9 * (1 - passed * passed);
  if (alpha <= 0) return;
  g.save();
  g.globalAlpha *= alpha;
  const size = 1 + 0.1 * outQuint(passed * 2);
  g.scale(size, size);
  ring(g, accent, DIAMETER, 4);
  g.restore();
}

// The approach circle: a plain ring in the combo colour (lazer's texture is not shipped), `size`
// times the circle (4 → 1).
export function approachCircle(g, accent, size) {
  ring(g, accent, DIAMETER * size, 4);
}

// The slider body (ArgonSliderBody) along `points` in playfield units, `unit` playfield units per
// object unit: the combo colour as its border, the darkened colour inside, with round ends.
export function sliderBody(g, points, unit, accent) {
  const radius = (OUTER_GRADIENT / 2) * unit;
  const trace = width => {
    g.beginPath();
    points.forEach((point, index) => (index ? g.lineTo(point.x, point.y) : g.moveTo(point.x, point.y)));
    // A slider with no length is still a dot.
    if (points.length === 1) g.lineTo(points[0].x + 0.01, points[0].y);
    g.lineWidth = width;
    g.lineCap = "round";
    g.lineJoin = "round";
    g.stroke();
  };
  g.strokeStyle = accent;
  trace(radius * 2);
  g.strokeStyle = darken(accent, 4);
  trace((radius - GRADIENT * unit) * 2);
}

// A slider tick still to come (ArgonSliderScorePoint): a 12-unit ring in the combo colour.
export function tick(g, accent) {
  ring(g, accent, 12, 3);
}

// One Font Awesome "angle" chevron pointing along +x, for an icon `size` across.
function chevron(g, colour, x, size) {
  const [w, h] = [size * 0.14, size * 0.27];
  g.beginPath();
  g.moveTo(x - w, -h);
  g.lineTo(x + w, 0);
  g.lineTo(x - w, h);
  g.lineWidth = size * 0.13;
  g.lineCap = "round";
  g.lineJoin = "round";
  g.strokeStyle = colour;
  g.stroke();
}

// ArgonReverseArrow pointing along +x: a white 40 × 20 pill with a dark double chevron that jumps
// to 1.3× every 300 ms and settles. `since` is the time since the arrow appeared.
export function reverseArrow(g, accent, since) {
  const [moveOut, moveIn, total] = [35, 250, 300];
  const loop = ((since % total) + total) % total;
  const pulse = loop < moveOut ? outQuad(loop / moveOut) : 1 - outQuad((loop - moveOut) / moveIn);
  g.save();
  g.scale(1 + 0.3 * pulse, 1 + 0.3 * pulse);
  g.beginPath();
  g.moveTo(-10, -10);
  g.lineTo(10, -10);
  g.arc(10, 0, 10, -Math.PI / 2, Math.PI / 2);
  g.lineTo(-10, 10);
  g.arc(-10, 0, 10, Math.PI / 2, (3 * Math.PI) / 2);
  g.closePath();
  g.fillStyle = "#ffffff";
  g.fill();
  const dark = darken(accent, 4);
  chevron(g, dark, -3.2, 16);
  chevron(g, dark, 3.2, 16);
  g.restore();
}

// The slider ball (ArgonSliderBall): the combo-colour gradient inside a white ring, with a white
// chevron turned the way the ball travels (`angle`, radians).
export function sliderBall(g, accent, angle) {
  disc(g, OUTER_GRADIENT / 2, vertical(g, OUTER_GRADIENT, accent, darken(accent, 0.5)));
  ring(g, "#ffffff", OUTER_GRADIENT, GRADIENT);
  g.save();
  g.rotate(angle);
  g.scale(0.6, 0.8);
  chevron(g, "#ffffff", 0, 48);
  g.restore();
}

// Argon's spinner disc at rest (the editor never spins it), `radius` across in screen units: the
// small pink fill, 25 ticks, two thin arcs top and bottom, two wide arcs at the sides, the centre.
export function spinner(g, radius) {
  disc(g, radius * 0.1, "rgba(252,97,143,0.2)");
  const unit = radius / 192;
  g.fillStyle = "rgba(255,255,255,0.2)";
  for (let i = 0; i < 25; i++) {
    g.save();
    g.rotate((-(i / 25) * 360 - 120) * (Math.PI / 180));
    pill(g, radius - 34 * unit, -2.5 * unit, 30 * unit, 5 * unit);
    g.restore();
  }
  arc(g, "#ffffff", radius * 0.02, radius * 0.99, -90, 0.31 * 360);
  arc(g, "#ffffff", radius * 0.02, radius * 0.99, 90, 0.31 * 360);
  arc(g, "rgba(255,255,255,0.25)", radius * 0.12, radius * 0.94, 0, 0.15 * 360);
  arc(g, "rgba(255,255,255,0.25)", radius * 0.12, radius * 0.94, 180, 0.15 * 360);
  g.beginPath();
  g.arc(0, 0, 40 * unit * 0.8, 0, Math.PI * 2);
  g.lineWidth = Math.max(1.5, 4 * unit);
  g.strokeStyle = "#ffffff";
  g.stroke();
}
export { SPINNER_PINK };

function pill(g, x, y, w, h) {
  const r = h / 2;
  g.beginPath();
  g.moveTo(x + r, y);
  g.lineTo(x + w - r, y);
  g.arc(x + w - r, y + r, r, -Math.PI / 2, Math.PI / 2);
  g.lineTo(x + r, y + h);
  g.arc(x + r, y + r, r, Math.PI / 2, (3 * Math.PI) / 2);
  g.closePath();
  g.fill();
}

// An arc centred on `centre` degrees (0 = right, 90 = down), sweeping `sweep` degrees.
function arc(g, colour, width, radius, centre, sweep) {
  const from = ((centre - sweep / 2) * Math.PI) / 180;
  const to = ((centre + sweep / 2) * Math.PI) / 180;
  g.beginPath();
  g.arc(0, 0, radius, from, to);
  g.lineWidth = width;
  g.strokeStyle = colour;
  g.stroke();
}

// How big the spinner disc is `since` ms after its approach began (ArgonSpinnerDisc.cs:228-250): nothing
// until half the approach time, 0.2 by the spinner's time, then full size over the next half.
export function spinnerGrowth(since, preempt) {
  if (since < preempt / 2) return 0;
  if (since < preempt) return 0.2 * outQuint((since - preempt / 2) / (preempt / 4));
  return 0.2 + 0.8 * outQuint((since - preempt) / (preempt / 2));
}

// osu-framework's Easing.OutElasticHalf: overshoots a little, then settles at 1.
export function outElasticHalf(t) {
  const time = clamp01(t);
  return 2 ** (-10 * time) * Math.sin((0.5 * time - 0.075) * ((2 * Math.PI) / 0.3)) + 1;
}
