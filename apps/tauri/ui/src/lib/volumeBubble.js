// Pure helpers for the inline Hit Volume lane bubble (hover readout + double-click typing).

export const VOLUME_MIN = 5;
export const VOLUME_MAX = 100;

/**
 * Parses a typed volume string (e.g. "65", "65%", " 3 ") into an integer in [5, 100],
 * or null if the text is not a finite number.
 */
export function parseVolumeInput(raw) {
  if (raw == null) return null;
  const cleaned = String(raw).trim().replace(/%$/, "").trim();
  if (!cleaned || !/^-?\d+(\.\d+)?$/.test(cleaned)) return null;
  const num = Number(cleaned);
  if (!Number.isFinite(num)) return null;
  return Math.max(VOLUME_MIN, Math.min(VOLUME_MAX, Math.round(num)));
}

/**
 * Formats a millisecond timestamp as `m:ss.mmm` for the volume bubble readout.
 */
export function formatVolumeTime(ms) {
  const safe = Math.max(0, Math.round(Number(ms) || 0));
  const mins = Math.floor(safe / 60000);
  const secs = Math.floor((safe % 60000) / 1000);
  const millis = safe % 1000;
  return `${mins}:${String(secs).padStart(2, "0")}.${String(millis).padStart(3, "0")}`;
}

/**
 * Places the floating volume bubble above the bar's top, or flips below when too close to the
 * top of the window, clamping horizontally inside the viewport.
 */
export function placeVolumeBubble({ barX, barTopY, viewportWidth = 1440, margin = 64 }) {
  const x = Math.max(margin, Math.min(Math.max(margin, viewportWidth - margin), Math.round(barX)));
  const below = barTopY < 56;
  const y = Math.round(below ? barTopY + 14 : barTopY - 10);
  return { x, y, below };
}

/**
 * Builds the floating volume bubble model for the moment nearest `ms` (within 5 ms) on `#controller`.
 */
export function volumeBubbleAtMoment(scene, ms, rect, { editing = false, viewportWidth = 1440 } = {}) {
  if (!scene || !Number.isFinite(ms) || !rect || !(rect.width > 0) || !(rect.height > 0)) return null;
  let best = null;
  let bestDist = 5.0001;
  for (const moment of scene.moments ?? []) {
    const time = moment[0];
    const dist = Math.abs(time - ms);
    if (dist < bestDist) {
      bestDist = dist;
      best = moment;
    }
  }
  if (!best) return null;
  const time = best[0];
  const volume = Math.max(VOLUME_MIN, Math.min(VOLUME_MAX, Math.round(Number(best[1]) || 100)));
  const relX = (time - scene.viewStart) * scene.zoom;
  if (relX < -24 || relX > rect.width + 24) return null;
  const barX = rect.left + relX;
  const barHeight = Math.max(4, (volume / 100) * Math.max(1, rect.height - 8));
  const barTopY = rect.bottom - barHeight;
  const pos = placeVolumeBubble({ barX, barTopY, viewportWidth });
  return {
    key: `${time}:${volume}:${editing ? "edit" : "hover"}`,
    ms: time,
    volume,
    timeLabel: formatVolumeTime(time),
    x: pos.x,
    y: pos.y,
    below: pos.below,
    editing: !!editing,
  };
}
