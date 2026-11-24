<script>
  import { untrack } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import * as argon from "../lib/argon.js";
  import {
    AFTER_HIT, PLAYFIELD, approach, arrowsShown, ballAt, bodyOpacity, clamp01, clock, comboColour, fit, pathLength, pathTo,
    pieceOpacity, pointAt, snake, ticksShown, visible,
  } from "../lib/preview.js";

  // The map preview's playfield: osu!'s objects at the playhead, in lazer's default (Argon) look.
  // The Inspector's panel (MapPreview) and the preview's own window (PreviewWindow) both draw it.
  // `view` is the editor's position (position, playing, speed, duration); `path` and `previewKey`
  // name the open difficulty; nothing is asked for or drawn while `active` is false. `fill` fills
  // the box given; otherwise the playfield sits in a 4:3 box as wide as its parent.
  let { view = null, path = null, previewKey = null, active = true, fill = false, skinId = "argon" } = $props();
  let canvas = $state(null);
  let model = $state.raw(null);
  let skinData = $state.raw(null);
  let skinError = $state("");
  let error = $state("");
  const key = $derived(previewKey);

  // Decode a data-URL skin payload into HTMLImageElement sprites + tinted hitcircle cache.
  function decodeSkin(raw) {
    if (!raw || typeof Image === "undefined") return null;
    const loadImg = src => {
      if (!src) return null;
      const img = new Image();
      img.src = src;
      return img;
    };
    const decoded = {
      hitcircle: loadImg(raw.hitcircle),
      overlay: loadImg(raw.overlay),
      sliderEndCircle: loadImg(raw.sliderEndCircle),
      sliderEndOverlay: loadImg(raw.sliderEndOverlay),
      hasCustomEndCircle: Boolean(raw.sliderEndCircle),
      approachCircle: loadImg(raw.approachCircle),
      reverseArrow: loadImg(raw.reverseArrow),
      digits: (raw.digits ?? []).map(loadImg),
      digitHd: raw.digitHd !== false,
      overlap: typeof raw.overlap === "number" ? raw.overlap : -2,
      overlayAboveNumber: raw.overlayAboveNumber !== false,
      comboColours: Array.isArray(raw.comboColours) && raw.comboColours.length ? raw.comboColours : null,
      sliderBorder: raw.sliderBorder || null,
      sliderTrack: raw.sliderTrack || null,
      tinted: new Map(),
      tintedEnd: new Map(),
      tintedApproach: new Map(),
      version: 0,
    };
    const all = [decoded.hitcircle, decoded.overlay, decoded.approachCircle, decoded.reverseArrow, ...decoded.digits].filter(Boolean);
    for (const img of all) {
      img.onload = () => { decoded.version++; };
    }
    return decoded;
  }

  function tintSprite(cache, sprite, colour) {
    if (!sprite || !sprite.complete || !sprite.naturalWidth) return null;
    const hit = cache.get(colour);
    if (hit) return hit;
    const off = globalThis.document?.createElement("canvas");
    if (!off) return sprite;
    off.width = sprite.naturalWidth;
    off.height = sprite.naturalHeight;
    const octx = off.getContext("2d");
    if (!octx) return sprite;
    octx.drawImage(sprite, 0, 0);
    octx.globalCompositeOperation = "multiply";
    octx.fillStyle = colour;
    octx.fillRect(0, 0, off.width, off.height);
    octx.globalCompositeOperation = "destination-in";
    octx.drawImage(sprite, 0, 0);
    cache.set(colour, off);
    return off;
  }

  function skinComboColour(map, object) {
    if (skinData?.comboColours?.length) {
      const idx = (object.colour ?? 0) % skinData.comboColours.length;
      return skinData.comboColours[idx];
    }
    return comboColour(map, object);
  }

  $effect(() => {
    const id = skinId || "argon";
    if (!active || id === "argon") {
      skinData = null;
      skinError = "";
      return;
    }
    let current = true;
    invoke("preview_skin_load", { skinId: id })
      .then(raw => {
        if (!current) return;
        skinData = decodeSkin(raw);
        skinError = "";
      })
      .catch(reason => {
        if (!current) return;
        skinData = null;
        skinError = String(reason);
      });
    return () => { current = false; };
  });

  // The objects come once per difficulty and again after a load, Save or revert (previewKey).
  $effect(() => {
    if (!active || !path) {
      model = null;
      return;
    }
    let current = true;
    void key;
    // A new difficulty: drop the last one's objects rather than draw them at the new playhead.
    // Read without tracking: the answer below sets `model`, which must not ask again.
    if (untrack(() => model?.path) !== path) {
      model = null;
      error = "";
    }
    invoke("map_preview_objects", { map: path })
      .then(next => { if (current) { model = next; error = ""; } })
      .catch(reason => { if (current) { model = null; error = String(reason); } });
    return () => { current = false; };
  });

  // The editor's position, and when it arrived, for the clock between updates.
  let receivedAt = 0;
  $effect(() => {
    void view;
    receivedAt = performance.now();
  });

  // One frame per display refresh while shown. While paused it paints only when the time, the
  // objects or the box change; a folded panel (0 wide) is not painted.
  $effect(() => {
    if (!active || !canvas) return;
    let frame = 0;
    let time = null;
    let last = "";
    let lastModel;
    const draw = () => {
      frame = requestAnimationFrame(draw);
      if (!canvas.clientWidth) return;
      time = clock(view, receivedAt, performance.now(), time);
      const state = [time, canvas.clientWidth, canvas.clientHeight, globalThis.devicePixelRatio, skinId, skinData?.version ?? 0].join();
      if (state === last && lastModel === model) return;
      last = state;
      lastModel = model;
      try {
        paint(canvas, model, time);
      } catch (reason) {
        console.error(reason);
      }
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  });

  function paint(target, map, time) {
    const ratio = globalThis.devicePixelRatio || 1;
    const width = target.clientWidth;
    const height = target.clientHeight;
    if (target.width !== Math.round(width * ratio) || target.height !== Math.round(height * ratio)) {
      target.width = Math.round(width * ratio);
      target.height = Math.round(height * ratio);
    }
    const g = target.getContext("2d");
    if (!g) return;
    g.setTransform(ratio, 0, 0, ratio, 0, 0);
    g.clearRect(0, 0, width, height);
    const place = fit(width, height);
    g.translate(place.x, place.y);
    g.scale(place.scale, place.scale);
    // The playfield, a shade lighter than the box, with a faint border.
    g.fillStyle = "#121218";
    g.fillRect(0, 0, PLAYFIELD.width, PLAYFIELD.height);
    g.strokeStyle = "rgba(255,255,255,0.14)";
    g.lineWidth = 1 / Math.max(place.scale, 0.01);
    g.strokeRect(0, 0, PLAYFIELD.width, PLAYFIELD.height);
    if (!map?.objects) return;
    const unit = map.radius / (argon.DIAMETER / 2);
    drawFollowPoints(g, map, time, unit);
    const shown = visible(map, time);
    for (const object of shown) {
      const colour = skinComboColour(map, object);
      if (object.kind === "spinner") drawSpinner(g, map, object, time);
      else if (object.kind === "slider") drawSlider(g, map, object, time, unit, colour);
      else drawCircle(g, map, object, time, unit, colour);
    }
    // Approach circles over everything, as lazer proxies them above the objects.
    for (const object of shown) {
      const ring = approach(map, object, time);
      if (!ring) continue;
      g.globalAlpha = ring.alpha;
      const colour = skinComboColour(map, object);
      at(g, object, unit, () => {
        const sprite = skinData?.approachCircle && tintSprite(skinData.tintedApproach, skinData.approachCircle, colour);
        if (sprite) {
          const size = argon.DIAMETER * ring.scale;
          g.drawImage(sprite, -size / 2, -size / 2, size, size);
        } else {
          argon.approachCircle(g, colour, ring.scale);
        }
      });
    }
    g.globalAlpha = 1;
  }

  // osu! follow points between consecutive objects in the same combo (FollowPointConnection.cs):
  // spaced every 32 playfield units starting 32 units from each endpoint, fading in before the next object.
  function drawFollowPoints(g, map, time, unit) {
    const objs = map.objects;
    if (!objs || objs.length < 2) return;
    const spacing = 32;
    const preempt = Math.min(800, map.preempt || 600);
    const fadeIn = Math.min(400, map.fadeIn || 240);
    const dotRadius = Math.max(2.2, 4 * unit);
    for (let i = 0; i < objs.length - 1; i++) {
      const prev = objs[i];
      const next = objs[i + 1];
      if (prev.kind === "spinner" || next.kind === "spinner") continue;
      if (next.combo === 1) continue; // new combo breaks follow points
      const startTime = prev.end ?? prev.time;
      const endTime = next.time;
      const duration = endTime - startTime;
      if (duration <= 0) continue;
      if (time < startTime - preempt || time > endTime + 150) continue;
      const endPos = prev.kind === "slider" && prev.path?.length >= 2
        ? ballAt(prev, prev.end)
        : { x: prev.x, y: prev.y };
      const dx = next.x - endPos.x;
      const dy = next.y - endPos.y;
      const dist = Math.hypot(dx, dy);
      if (dist <= spacing * 2) continue;
      for (let d = spacing * 1.5; d < dist - spacing * 1.5; d += spacing) {
        const fraction = d / dist;
        const pointStart = startTime + fraction * duration - preempt;
        const pointEnd = startTime + fraction * duration;
        if (time < pointStart || time > pointEnd + fadeIn * 0.5) continue;
        let alpha = 1;
        if (time < pointStart + fadeIn) alpha = clamp01((time - pointStart) / Math.max(1, fadeIn));
        else if (time > pointEnd) alpha = clamp01(1 - (time - pointEnd) / Math.max(1, fadeIn * 0.5));
        if (alpha <= 0.01) continue;
        const px = endPos.x + dx * fraction;
        const py = endPos.y + dy * fraction;
        g.globalAlpha = alpha * 0.72;
        g.fillStyle = "#ffffff";
        g.beginPath();
        g.arc(px, py, dotRadius, 0, Math.PI * 2);
        g.fill();
      }
    }
    g.globalAlpha = 1;
  }

  // Draws in lazer's object units (a circle is 128 across) centred on a playfield point.
  function at(g, point, unit, draw, size = 1) {
    g.save();
    g.translate(point.x, point.y);
    g.scale(unit * size, unit * size);
    draw();
    g.restore();
  }

  function drawSkinDigits(g, skin, number) {
    if (!(number > 0)) return;
    const chars = String(number).split("");
    const imgs = chars.map(ch => skin.digits[Number(ch)]).filter(img => img && img.complete && img.naturalWidth > 0);
    if (imgs.length !== chars.length) {
      // Fallback: mathematically centered text when skin omits digit sprites.
      const text = String(number);
      g.fillStyle = "#ffffff";
      g.font = "700 44px system-ui, sans-serif";
      g.textAlign = "center";
      g.textBaseline = "middle";
      const metrics = g.measureText?.(text);
      const yOffset = metrics && Number.isFinite(metrics.actualBoundingBoxAscent) && Number.isFinite(metrics.actualBoundingBoxDescent)
        ? (metrics.actualBoundingBoxAscent - metrics.actualBoundingBoxDescent) / 2
        : 0;
      g.fillText(text, 0, yOffset);
      return;
    }
    // Normalize @2x sprites to osu! 1x object coordinates (128 diameter), then center horizontally and vertically.
    const scale = skin.digitHd ? 0.5 : 1.0;
    const widths = imgs.map(img => img.naturalWidth * scale);
    const heights = imgs.map(img => img.naturalHeight * scale);
    const totalW = widths.reduce((sum, w) => sum + w, 0) - skin.overlap * (imgs.length - 1);
    let x = -totalW / 2;
    for (let i = 0; i < imgs.length; i++) {
      g.drawImage(imgs[i], x, -heights[i] / 2, widths[i], heights[i]);
      x += widths[i] - skin.overlap;
    }
  }

  // A circle or a slider head: the combo colour until its time, then white, fading over 700 ms.
  function drawPiece(g, map, object, time, unit, colour, outerFill) {
    const alpha = pieceOpacity(map, object, time);
    if (alpha <= 0) return;
    g.globalAlpha = alpha;
    const passed = time > object.time;
    const tint = passed ? "#ffffff" : colour;
    at(g, object, unit, () => {
      if (skinData && (skinData.hitcircle?.complete || skinData.overlay?.complete)) {
        const d = argon.DIAMETER;
        const base = tintSprite(skinData.tinted, skinData.hitcircle, tint);
        if (base) g.drawImage(base, -d / 2, -d / 2, d, d);
        if (!skinData.overlayAboveNumber && skinData.overlay?.complete && skinData.overlay.naturalWidth) {
          g.drawImage(skinData.overlay, -d / 2, -d / 2, d, d);
        }
        drawSkinDigits(g, skinData, object.combo);
        if (skinData.overlayAboveNumber && skinData.overlay?.complete && skinData.overlay.naturalWidth) {
          g.drawImage(skinData.overlay, -d / 2, -d / 2, d, d);
        }
      } else {
        argon.circle(g, tint, object.combo, outerFill);
      }
    });
  }

  function drawCircle(g, map, object, time, unit, colour) {
    drawPiece(g, map, object, time, unit, colour, true);
    if (time < object.time) return;
    g.globalAlpha = 1;
    at(g, object, unit, () => argon.hitMarker(g, colour, (time - object.time) / AFTER_HIT));
  }

  // A slider body drawn at full strength on its own layer, then laid on at the body's opacity,
  // so its border does not show through the fill while it fades.
  const layer = globalThis.document?.createElement("canvas");

  function drawSlider(g, map, object, time, unit, colour) {
    const alpha = bodyOpacity(map, object, time);
    const snaked = snake(map, object, time);
    if (alpha > 0 && snaked > 0) {
      const points = pathTo(object, snaked * pathLength(object));
      const l = alpha < 1 && layer?.getContext("2d");
      if (l) {
        if (layer.width !== g.canvas.width || layer.height !== g.canvas.height) {
          layer.width = g.canvas.width;
          layer.height = g.canvas.height;
        }
        l.setTransform(1, 0, 0, 1, 0, 0);
        l.clearRect(0, 0, layer.width, layer.height);
        l.setTransform(g.getTransform());
        argon.sliderBody(l, points, unit, colour);
        g.save();
        g.globalAlpha = alpha;
        g.setTransform(1, 0, 0, 1, 0, 0);
        g.drawImage(layer, 0, 0);
        g.restore();
      } else {
        g.globalAlpha = alpha;
        argon.sliderBody(g, points, unit, colour);
      }
    }
    for (const tick of ticksShown(map, object, time)) {
      g.globalAlpha = clamp01(tick.since / 150);
      at(g, tick, unit, () => argon.tick(g, colour), 0.5 + 0.5 * argon.outElasticHalf(tick.since / 600));
    }
    for (const arrow of arrowsShown(map, object, time)) {
      g.globalAlpha = clamp01(arrow.since / arrow.fade);
      at(g, arrow, unit, () => {
        g.rotate(arrow.angle);
        argon.reverseArrow(g, colour, arrow.since);
      });
    }
    drawPiece(g, map, object, time, unit, colour, false);
    // Slider end circle: in Classic/Legacy skins, draw sliderendcircle (+ overlay) if provided,
    // otherwise fall back to hitcircle + hitcircleoverlay (matching osu!lazer LegacySliderTail).
    if (skinData && snaked >= 0.99 && alpha > 0 && (object.path?.length ?? 0) >= 4) {
      const tail = pointAt(object, pathLength(object));
      g.globalAlpha = alpha;
      at(g, tail, unit, () => {
        const d = argon.DIAMETER;
        const endBaseImg = skinData.hasCustomEndCircle ? skinData.sliderEndCircle : skinData.hitcircle;
        const endOverImg = skinData.hasCustomEndCircle ? skinData.sliderEndOverlay : skinData.overlay;
        const base = endBaseImg && tintSprite(skinData.hasCustomEndCircle ? skinData.tintedEnd : skinData.tinted, endBaseImg, colour);
        if (base && base.width > 1 && base.height > 1) g.drawImage(base, -d / 2, -d / 2, d, d);
        if (endOverImg?.complete && endOverImg.naturalWidth > 1 && endOverImg.naturalHeight > 1) {
          g.drawImage(endOverImg, -d / 2, -d / 2, d, d);
        }
      });
    }
    if (time >= object.time && time <= object.end) {
      const ballPlace = ballAt(object, time);
      const ahead = ballAt(object, time + 1);
      const behind = ballAt(object, time - 1);
      // lazer's ball shows at once (DrawableSlider: Ball.FadeIn()).
      g.globalAlpha = 1;
      at(g, ballPlace, unit, () => argon.sliderBall(g, colour, Math.atan2(ahead.y - behind.y, ahead.x - behind.x)));
    }
  }

  // Argon's spinner disc in the playfield's centre, growing in over the approach time.
  function drawSpinner(g, map, object, time) {
    const alpha = bodyOpacity(map, object, time);
    const radius = PLAYFIELD.height * 0.5 * argon.spinnerGrowth(time - (object.time - map.preempt), map.preempt);
    if (alpha <= 0 || radius < 1) return;
    g.globalAlpha = alpha;
    g.save();
    g.translate(PLAYFIELD.width / 2, PLAYFIELD.height / 2);
    argon.spinner(g, radius);
    g.restore();
  }
</script>

{#if !path}
  <p class="insp-summary">Open a difficulty to see its preview.</p>
{:else if model?.unsupported}
  <p class="insp-summary">The map preview shows osu!standard maps only.</p>
{:else if error}
  <p class="insp-summary">{error}</p>
{:else}
  <!-- A 4:3 box (padding, since the bundle targets WebViews without aspect-ratio), or the whole box. -->
  <div class="frame" class:fill><canvas bind:this={canvas} aria-label="Map preview at the playhead"></canvas></div>
{/if}

<style>
  .frame {
    position: relative;
    padding-top: 75%;
  }
  .frame.fill {
    padding-top: 0;
    width: 100%;
    height: 100%;
  }
  .frame canvas {
    position: absolute;
    top: 0;
    left: 0;
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 4px;
    background: #0b0b0f;
  }
  .frame.fill canvas {
    border-radius: 0;
  }
</style>
