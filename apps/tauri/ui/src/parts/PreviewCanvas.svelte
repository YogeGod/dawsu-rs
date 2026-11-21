<script>
  import { untrack } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import * as argon from "../lib/argon.js";
  import {
    AFTER_HIT, PLAYFIELD, approach, arrowsShown, ballAt, bodyOpacity, clamp01, clock, comboColour, fit, pathLength, pathTo,
    pieceOpacity, snake, ticksShown, visible,
  } from "../lib/preview.js";

  // The map preview's playfield: osu!'s objects at the playhead, in lazer's default (Argon) look.
  // The Inspector's panel (MapPreview) and the preview's own window (PreviewWindow) both draw it.
  // `view` is the editor's position (position, playing, speed, duration); `path` and `previewKey`
  // name the open difficulty; nothing is asked for or drawn while `active` is false. `fill` fills
  // the box given; otherwise the playfield sits in a 4:3 box as wide as its parent.
  let { view = null, path = null, previewKey = null, active = true, fill = false } = $props();
  let canvas = $state(null);
  let model = $state.raw(null);
  let error = $state("");
  const key = $derived(previewKey);

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
      const state = [time, canvas.clientWidth, canvas.clientHeight, globalThis.devicePixelRatio].join();
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
    const shown = visible(map, time);
    for (const object of shown) {
      const colour = comboColour(map, object);
      if (object.kind === "spinner") drawSpinner(g, map, object, time);
      else if (object.kind === "slider") drawSlider(g, map, object, time, unit, colour);
      else drawCircle(g, map, object, time, unit, colour);
    }
    // Approach circles over everything, as lazer proxies them above the objects.
    for (const object of shown) {
      const ring = approach(map, object, time);
      if (!ring) continue;
      g.globalAlpha = ring.alpha;
      at(g, object, unit, () => argon.approachCircle(g, comboColour(map, object), ring.scale));
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

  // A circle or a slider head: the combo colour until its time, then white, fading over 700 ms.
  function drawPiece(g, map, object, time, unit, colour, outerFill) {
    const alpha = pieceOpacity(map, object, time);
    if (alpha <= 0) return;
    g.globalAlpha = alpha;
    const passed = time > object.time;
    at(g, object, unit, () => argon.circle(g, passed ? "#ffffff" : colour, object.combo, outerFill));
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
