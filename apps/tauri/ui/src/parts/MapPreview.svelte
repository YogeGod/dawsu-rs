<script>
  import { untrack } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    AFTER_HIT, PLAYFIELD, approach, clamp01, ball, clock, comboColour, fit, opacity, pathLength, pathTo, reverseArrow, snake, visible,
  } from "../lib/preview.js";
  import InspectorHead from "./InspectorHead.svelte";

  // The map preview (step 1): osu!'s playfield at the playhead, in plain shapes. Shown from
  // View › Map preview; on or off is kept in this window only.
  let open = $state(true);
  let canvas = $state(null);
  let model = $state.raw(null);
  let error = $state("");

  const on = $derived(bridge.mapPreview);
  const path = $derived(bridge.scene?.path ?? null);
  const key = $derived(bridge.scene?.previewKey ?? null);

  // The objects come once per difficulty and again after a load, Save or revert (previewKey).
  $effect(() => {
    if (!on || !path) {
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
    void bridge.view;
    receivedAt = performance.now();
  });

  // One frame per display refresh while shown. While paused it paints only when the time, the
  // objects or the box change; a folded panel (0 wide) is not painted.
  $effect(() => {
    if (!on || !open || !canvas) return;
    let frame = 0;
    let time = null;
    let last = "";
    let lastModel;
    const draw = () => {
      frame = requestAnimationFrame(draw);
      if (!canvas.clientWidth) return;
      time = clock(bridge.view, receivedAt, performance.now(), time);
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
    // The playfield's faint border.
    g.strokeStyle = "rgba(255,255,255,0.12)";
    g.lineWidth = 1 / Math.max(place.scale, 0.01);
    g.strokeRect(0, 0, PLAYFIELD.width, PLAYFIELD.height);
    if (!map?.objects) return;
    const r = map.radius;
    g.font = `600 ${Math.round(r * 0.8)}px system-ui, sans-serif`;
    g.textAlign = "center";
    g.textBaseline = "middle";
    for (const object of visible(map, time)) {
      const alpha = opacity(map, object, time);
      const colour = comboColour(map, object);
      g.globalAlpha = alpha;
      if (object.kind === "spinner") drawSpinner(g, object, time);
      else if (object.kind === "slider") drawSlider(g, map, object, time, r, colour, alpha);
      else drawCircle(g, object.x, object.y, r, colour, object.combo, time >= object.time);
      const ring = approach(map, object, time);
      if (ring) {
        g.globalAlpha = ring.alpha;
        g.strokeStyle = colour;
        g.lineWidth = r * 0.08;
        g.beginPath();
        g.arc(object.x, object.y, r * ring.scale, 0, Math.PI * 2);
        g.stroke();
      }
    }
    g.globalAlpha = 1;
  }

  // A plain circle: combo colour inside a white ring, with its combo number. After its time it
  // turns white, as lazer's editor shows a hit circle that has been passed.
  function drawCircle(g, x, y, r, colour, number, hit) {
    g.beginPath();
    g.arc(x, y, r * 0.94, 0, Math.PI * 2);
    g.fillStyle = hit ? "#ffffff" : colour;
    g.fill();
    g.lineWidth = r * 0.12;
    g.strokeStyle = "#ffffff";
    g.stroke();
    if (number && !hit) {
      g.fillStyle = "#ffffff";
      g.fillText(String(number), x, y + r * 0.04);
    }
  }

  // A slider body drawn at full strength on its own layer, then laid on at the slider's opacity,
  // so the white border does not show through the fill while it fades.
  const layer = globalThis.document?.createElement("canvas");

  function drawSlider(g, map, object, time, r, colour, alpha) {
    const points = pathTo(object, snake(map, object, time) * pathLength(object));
    const body = target => {
      const trace = width => {
        target.beginPath();
        points.forEach((point, index) => (index ? target.lineTo(point.x, point.y) : target.moveTo(point.x, point.y)));
        target.lineWidth = width;
        target.lineCap = "round";
        target.lineJoin = "round";
        target.stroke();
      };
      // A white border round a darker fill of the combo colour.
      target.strokeStyle = "#ffffff";
      trace(r * 2);
      target.strokeStyle = shade(colour, 0.35);
      trace(r * 1.76);
    };
    const l = alpha < 1 && layer?.getContext("2d");
    if (l) {
      if (layer.width !== g.canvas.width || layer.height !== g.canvas.height) {
        layer.width = g.canvas.width;
        layer.height = g.canvas.height;
      }
      l.setTransform(1, 0, 0, 1, 0, 0);
      l.clearRect(0, 0, layer.width, layer.height);
      l.setTransform(g.getTransform());
      body(l);
      g.save();
      g.setTransform(1, 0, 0, 1, 0, 0);
      g.drawImage(layer, 0, 0);
      g.restore();
    } else {
      body(g);
    }
    const arrow = reverseArrow(object, time);
    if (arrow) drawArrow(g, arrow, r);
    // The head stays until its time, then fades out like a circle over AFTER_HIT.
    if (time < object.time + AFTER_HIT) {
      if (time >= object.time) g.globalAlpha = alpha * clamp01(1 - (time - object.time) / AFTER_HIT);
      drawCircle(g, object.x, object.y, r, colour, object.combo, time >= object.time);
      g.globalAlpha = alpha;
    }
    const at = ball(object, time);
    if (at) {
      g.beginPath();
      g.arc(at.x, at.y, r * 0.8, 0, Math.PI * 2);
      g.fillStyle = "#ffffff";
      g.fill();
    }
  }

  function drawArrow(g, { x, y, angle }, r) {
    g.save();
    g.translate(x, y);
    g.rotate(angle);
    g.beginPath();
    g.moveTo(r * 0.45, 0);
    g.lineTo(-r * 0.25, -r * 0.4);
    g.lineTo(-r * 0.25, r * 0.4);
    g.closePath();
    g.fillStyle = "#ffffff";
    g.fill();
    g.restore();
  }

  // A plain spinner in the playfield's centre: a ring, and an arc for the time left.
  function drawSpinner(g, object, time) {
    const [x, y, size] = [PLAYFIELD.width / 2, PLAYFIELD.height / 2, 150];
    g.lineWidth = 6;
    g.strokeStyle = "rgba(255,255,255,0.35)";
    g.beginPath();
    g.arc(x, y, size, 0, Math.PI * 2);
    g.stroke();
    const left = time < object.time ? 1 : Math.max(0, (object.end - time) / Math.max(1, object.end - object.time));
    g.strokeStyle = "#ffffff";
    g.beginPath();
    g.arc(x, y, size, -Math.PI / 2, -Math.PI / 2 + left * Math.PI * 2);
    g.stroke();
  }

  // A hex colour darkened to `amount` of its brightness.
  function shade(hex, amount) {
    const value = Number.parseInt(hex.slice(1), 16);
    const channel = shift => Math.round(((value >> shift) & 255) * amount);
    return `rgb(${channel(16)},${channel(8)},${channel(0)})`;
  }
</script>

{#if on}
  <div class="insp">
    <InspectorHead title="Map preview" body="inspector-preview-body" bind:open />
    <div id="inspector-preview-body" class="preview" hidden={!open}>
      {#if !path}
        <p class="insp-summary">Open a difficulty to see its preview.</p>
      {:else if model?.unsupported}
        <p class="insp-summary">The map preview shows osu!standard maps only.</p>
      {:else if error}
        <p class="insp-summary">{error}</p>
      {:else}
        <!-- A 4:3 box (padding, since the bundle targets WebViews without aspect-ratio). -->
        <div class="frame"><canvas bind:this={canvas} aria-label="Map preview at the playhead"></canvas></div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .frame {
    position: relative;
    padding-top: 75%;
  }
  .frame canvas {
    position: absolute;
    top: 0;
    left: 0;
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 4px;
    background: #0d0d0f;
  }
</style>
