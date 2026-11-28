// Core backend RPC loop, top-level view synchronization, frame chrome, and Home lists.
import { nameOf, recentMapName } from "./helpers.js";
import { callAction, showToast } from "../lib/registry.js";
import { markPracticeTried } from "../lib/startScreen.js";
import { sameSoundSetPrompts, sameUiView } from "../lib/view.js";
import { parseSpectrumFrame } from "../lib/spectrum.js";
import { osuReloadFailedToast, osuReloadToast } from "../lib/tapRecord.js";

function folderOfPath(path) {
  const normalized = String(path ?? "");
  const cut = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  return cut > 0 ? normalized.slice(0, cut) : "";
}

export function attachIpc(ctx) {
  const { env, state, dom, elements } = ctx;
  const { $, makeButton, paragraph } = dom;
  const { invoke } = env.window.__TAURI__.core;
  const notifyToast = toast => (env !== globalThis ? env.window?.showToast?.(toast) : showToast(toast));

  let lastStatusNotice = "";

  function setStatusNotice(text) {
    lastStatusNotice = text || "";
    if (ctx.ui()?.statusNotice) {
      ctx.ui().statusNotice(text);
      return;
    }
    const node = $("status");
    if (!node) return;
    const shown = text || state.view?.fault || state.view?.status || (state.view?.busy ? "Working…" : "");
    node.textContent = shown;
    node.title = shown;
  }

  function run(task) {
    state.requests++;
    let failed = false;
    const result = state.pending.then(task).catch((error) => {
      failed = true;
      setStatusNotice(String(error));
      (env.console ?? console).error(error);
    }).finally(() => {
      state.requests--;
      if (failed && state.requests === 0 && !state.drag && state.pendingDrawHits?.length) {
        state.pendingDrawHits = [];
        ctx.paintSoon?.();
      }
      // A wheel zoom or pan held back while the editor answered goes now.
      ctx.flushWheel?.();
    });
    state.pending = result;
    return result;
  }

  let lastActionKind = null;
  let waveformFetchKey = null;

  function toUint8View(buf) {
    if (!buf) return null;
    if (buf instanceof Uint8Array) return buf;
    if (ArrayBuffer.isView(buf)) {
      return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    }
    if (typeof buf.byteLength === "number" &&
        (buf instanceof ArrayBuffer || Object.prototype.toString.call(buf) === "[object ArrayBuffer]")) {
      return new Uint8Array(buf, 0, buf.byteLength);
    }
    if (Array.isArray(buf) && buf.length >= 64) {
      return Uint8Array.from(buf);
    }
    return null;
  }

  function toFloat32Waveform(buf) {
    if (!buf) return new Float32Array(0);
    if (ArrayBuffer.isView(buf)) {
      return new Float32Array(buf.buffer, buf.byteOffset, Math.floor(buf.byteLength / 4));
    }
    if (typeof buf.byteLength === "number" &&
        (buf instanceof ArrayBuffer || Object.prototype.toString.call(buf) === "[object ArrayBuffer]")) {
      return new Float32Array(buf, 0, Math.floor(buf.byteLength / 4));
    }
    return new Float32Array(0);
  }

  function decodeWaveformPayload(buf) {
    const bytes = toUint8View(buf);
    if (bytes && bytes.byteLength >= 12 &&
        bytes[0] === 0x44 && bytes[1] === 0x57 && bytes[2] === 0x46 && bytes[3] === 0x50) {
      const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      const version = dv.getUint16(4, true);
      const levelCount = dv.getUint16(6, true);
      const strideFloats = dv.getUint32(8, true);
      if (version === 1 && strideFloats === 6 && levelCount > 0 && bytes.byteLength >= 12 + levelCount * 8) {
        const metas = [];
        for (let i = 0; i < levelCount; i++) {
          const off = 12 + i * 8;
          metas.push({ count: dv.getUint32(off, true), strideBins: dv.getUint32(off + 4, true) });
        }
        let cursor = 12 + levelCount * 8;
        const aligned = (bytes.byteOffset % 4) === 0;
        const levels = [];
        for (const meta of metas) {
          const floatLen = meta.count * 6;
          const floatBytes = floatLen * 4;
          const maskBytes = ((Math.ceil(meta.count / 8) + 3) >> 2) << 2;
          if (cursor + floatBytes + maskBytes > bytes.byteLength) break;
          let bins;
          if (aligned) {
            bins = new Float32Array(bytes.buffer, bytes.byteOffset + cursor, floatLen);
          } else {
            const copy = bytes.slice(cursor, cursor + floatBytes);
            bins = new Float32Array(copy.buffer, 0, floatLen);
          }
          const clipMask = bytes.subarray(cursor + floatBytes, cursor + floatBytes + maskBytes);
          cursor += floatBytes + maskBytes;
          levels.push({ count: meta.count, strideBins: meta.strideBins, bins, clipMask });
        }
        if (levels.length > 0) {
          return {
            waveform: levels[0].bins,
            pyramid: { baseCount: levels[0].count, levels },
          };
        }
      }
    }
    return { waveform: toFloat32Waveform(buf), pyramid: null };
  }

  function parseTelemetryFrame(payload) {
    const bytes = toUint8View(payload);
    if (!bytes || bytes.byteLength < 64 || bytes[0] !== 0x01) return null;
    const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const flags = dv.getUint8(1);
    const meterNum = dv.getUint16(2, true);
    const meterDen = dv.getUint16(4, true);
    const subTick = dv.getUint16(6, true);
    const stateRev = typeof dv.getBigUint64 === "function"
      ? Number(dv.getBigUint64(8, true))
      : dv.getUint32(8, true) + dv.getUint32(12, true) * 4294967296;
    const position = dv.getFloat64(16, true);
    const bpmCenti = dv.getUint32(24, true);
    const bar = dv.getInt32(28, true);
    const beat = dv.getUint32(32, true);
    const masterL = dv.getFloat32(36, true);
    const masterR = dv.getFloat32(40, true);
    const song = dv.getFloat32(44, true);
    const hitsounds = dv.getFloat32(48, true);
    const rmsL = dv.getFloat32(52, true);
    const rmsR = dv.getFloat32(56, true);
    const phaseCorr = dv.getFloat32(60, true);
    return {
      version: 1,
      playing: (flags & 1) !== 0,
      busy: (flags & 2) !== 0,
      hasDocument: (flags & 4) !== 0,
      songLoading: (flags & 8) !== 0,
      stemLoading: (flags & 16) !== 0,
      meterNum,
      meterDen,
      subTick,
      stateRev,
      position,
      bpmCenti,
      bar,
      beat,
      masterL,
      masterR,
      master: Math.max(masterL, masterR),
      song,
      hitsounds,
      rmsL,
      rmsR,
      phaseCorr,
    };
  }

  function applyTelemetryFrame(frame) {
    const perf = env.window?.performance ?? env.performance ?? performance;
    state.telemetryConnected = true;
    state.lastTelemetryAt = perf.now();
    if (state.view.fault) return;

    const before = state.view;
    const needsStatePull =
      (state.lastTelemetryStateRev !== null && frame.stateRev !== state.lastTelemetryStateRev) ||
      Boolean(before.playing) !== frame.playing ||
      Boolean(before.busy) !== frame.busy ||
      Boolean(before.songLoading) !== frame.songLoading ||
      Boolean(before.stemLoading) !== frame.stemLoading ||
      Boolean(before.hasDocument) !== frame.hasDocument;

    if (state.scene && frame.hasDocument) {
      const drawn = ctx.playheadPosition();
      let position = frame.position;
      let heldPendingSeek = false;
      if (state.pendingSeek && state.pendingSeek.path === state.scene.path) {
        const scrubbing = state.rulerDrag?.mode === "scrub" || Boolean(state.rulerSeekTarget || state.rulerSeekDrain);
        const tolerance = frame.playing ? 250 : 1;
        const arrived = Number.isFinite(position) && Math.abs(position - state.pendingSeek.ms) <= tolerance;
        const expired = perf.now() - state.pendingSeek.at > 400;
        if (!scrubbing && (arrived || expired)) {
          state.pendingSeek = null;
        } else {
          position = state.pendingSeek.ms;
          heldPendingSeek = true;
        }
      }
      const prevMusical = before.musicalPosition;
      const snapSuffix = prevMusical ? prevMusical.split(" · ")[3] : null;
      const subStr = String(Math.min(479, Math.max(0, frame.subTick))).padStart(3, "0");
      const bpmText = frame.bpmCenti > 0 ? (frame.bpmCenti / 100).toFixed(1) : null;
      const musicalPosition = (bpmText && frame.meterNum > 0 && frame.meterDen > 0)
        ? `${frame.bar}.${frame.beat}.${subStr} · ${bpmText} BPM · ${frame.meterNum}/${frame.meterDen}${snapSuffix ? ` · ${snapSuffix}` : ""}`
        : prevMusical;

      state.view = {
        ...before,
        position,
        playing: frame.playing,
        busy: frame.busy,
        bpmCenti: frame.bpmCenti,
        meterNum: frame.meterNum,
        meterDen: frame.meterDen,
        bar: frame.bar,
        beat: frame.beat,
        subTick: frame.subTick,
        musicalPosition,
        peaks: {
          master: frame.master,
          masterL: frame.masterL,
          masterR: frame.masterR,
          song: frame.song,
          hitsounds: frame.hitsounds,
          rmsL: frame.rmsL,
          rmsR: frame.rmsR,
          phaseCorr: frame.phaseCorr,
        },
      };
      if (!heldPendingSeek) ctx.anchorPlayhead(drawn);
      ctx.followBookmarkPick();
      ctx.followPlayhead();
      ctx.paintSoon(true);
      if (state.view.playing) ctx.animatePlayback();
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
    }

    if (needsStatePull && !state.requests && !state.refreshing && !state.telemetryPullPending) {
      state.telemetryPullPending = true;
      const drawSeq = state.drawHitSeq ?? 0;
      ctx.run(() => ctx.update(null, false, drawSeq)).finally(() => {
        state.telemetryPullPending = false;
        if (state.quitAfterSave && !state.view.busy && !state.view.saveProblemReview) {
          state.quitAfterSave = false;
          if (!state.view.dirty && !state.view.error) ctx.run(() => invoke("close_editor", { discard: false }));
        }
      });
    }
  }

  function startTelemetry() {
    const ChannelCtor = env.window?.__TAURI__?.core?.Channel;
    if (typeof ChannelCtor !== "function") return null;
    const channel = new ChannelCtor();
    channel.onmessage = (payload) => {
      const bytes = toUint8View(payload);
      if (!bytes || bytes.byteLength < 4) return;
      if (bytes[0] === 0x01) {
        const frame = parseTelemetryFrame(bytes);
        if (frame) applyTelemetryFrame(frame);
      } else if (bytes[0] === 0x02) {
        const spec = parseSpectrumFrame(bytes);
        if (spec) ctx.ui()?.setSpectrum?.(spec);
      }
    };
    invoke("subscribe_telemetry", { channel }).catch((error) => {
      state.telemetryConnected = false;
      (env.console ?? console).error(error);
    });
    return channel;
  }

  function ensureWaveform(scene) {
    if (!scene || scene.waveform != null || !scene.path || scene.previewKey == null) return;
    const rev = state.view?.waveformRevision ?? 0;
    const stem = state.view?.stemActive ? 1 : 0;
    const key = `${scene.path}\0${scene.previewKey}\0${rev}\0${stem}`;
    if (waveformFetchKey === key) return;
    waveformFetchKey = key;
    invoke("editor_waveform").then((buf) => {
      const { waveform, pyramid } = decodeWaveformPayload(buf);
      if (state.scene && state.scene.path === scene.path && state.scene.previewKey === scene.previewKey &&
          (state.view?.waveformRevision ?? 0) === rev && (state.view?.stemActive ? 1 : 0) === stem) {
        state.scene.waveform = waveform;
        state.scene.waveformPyramid = pyramid;
        ctx.paintSoon();
      }
    }).catch((error) => {
      if (waveformFetchKey === key) waveformFetchKey = null;
      (env.console ?? console).error(error);
    });
  }

  async function update(action, full = true, drawSeq = state.drawHitSeq ?? 0) {
    const request = !action && !full
      ? { scene: false, action: null, tickRev: state.view?.tickRev ?? null }
      : { scene: full, action: action ?? null };
    let next = await invoke("editor_view", request);
    if (!next.fault && !full && (state.view.busy !== next.busy || (!next.tick && (state.view.hasDocument !== next.hasDocument ||
        state.view.laneCount !== next.laneCount || state.view.showHome !== next.showHome ||
        !!state.view.songLoading !== !!next.songLoading ||
        !!state.view.stemLoading !== !!next.stemLoading ||
        !!state.view.stemLoaded !== !!next.stemLoaded ||
        !!state.view.stemActive !== !!next.stemActive ||
        (state.view.waveformRevision ?? 0) !== (next.waveformRevision ?? 0))))) {
      // The editor names a Previous or Next bookmark step once, in the first answer.
      const step = next.bookmarkStep;
      next = await invoke("editor_view", { scene: true, action: null });
      if (!next.fault) next.bookmarkStep ??= step;
    }
    // The editor sends a map's static data (timing sections, waveform, objects, combos) once. A page
    // that never got it (reloaded, or another caller took that frame) asks for it again.
    const kept = state.scene?.sections && state.scene.path === next.scene?.path &&
      state.scene.previewKey === next.scene?.previewKey;
    if (!next.fault && next.scene && !next.scene.sections && !kept) {
      const step = next.bookmarkStep;
      next = await invoke("editor_view", { scene: true, action: null, fresh: true });
      if (!next.fault) next.bookmarkStep ??= step;
    }
    lastActionKind = action?.kind ?? null;
    ctx.show(next, drawSeq);
    if (next.shortcutHelp) await ctx.showHelp();
    if (next.shortcutOpen && !next.busy && !ctx.chooseSiblingDifficulty()) ctx.show(await invoke("pick_map"));
    if (!next.busy) {
      switch (next.shortcutAction) {
        case "filter": {
          const filter = $("lane-filter");
          const panel = filter.closest("[popover]");
          if (panel && !panel.matches(":popover-open")) panel.showPopover();
          filter.focus();
          break;
        }
        case "newDiff":
          ctx.openNewDifficulty();
          break;
        case "openFolder":
          await invoke("open_editor_folder", { folder: "map" });
          break;
        case "importOsz":
          ctx.show(await invoke("pick_map", { archiveOnly: true }));
          break;
        case "openHitsoundFolder":
          if (next.hitsoundLibrary) await invoke("open_editor_folder", { folder: "library" });
          else ctx.openFolders();
          break;
        case "reviewOutput":
          await ctx.openMergeSettings();
          break;
      }
    }
  }

  const TOOL_COMMAND_NAMES = {
    toolSelect: "Select",
    toolDraw: "Draw",
    toolErase: "Erase",
    toolMute: "Mute",
    toolGrab: "Grab",
    toolRange: "Range",
    toolRamp: "Ramp",
  };

  function optimisticToolFor(action) {
    if (!action) return null;
    if (action.kind === "tool" && typeof action.name === "string") return action.name;
    if (action.kind === "command" && Object.hasOwn(TOOL_COMMAND_NAMES, action.id)) return TOOL_COMMAND_NAMES[action.id];
    if (action.kind === "key" && !action.ctrl && !action.shift && !action.alt) {
      const osu = state.view?.keyPreset === "osu!";
      if (action.key === "1") return "Select";
      if (action.key === "2") return osu ? "Draw" : "Grab";
      if (action.key === "3" && !osu) return "Draw";
      if (action.key === "5") return "Mute";
      if (action.key === "6") return "Ramp";
    }
    return null;
  }

  function tryLaneColourHistory(action) {
    if (!state.scene || state.view.busy || state.drag) return false;
    const isUndo = (action?.kind === "command" && action.id === "undo")
      || (action?.kind === "key" && action.ctrl && !action.shift && !action.alt && String(action.key).toLowerCase() === "z");
    const isRedo = (action?.kind === "command" && action.id === "redo")
      || (action?.kind === "key" && action.ctrl && !action.alt && (
        (String(action.key).toLowerCase() === "y" && !action.shift)
        || (String(action.key).toLowerCase() === "z" && action.shift)
      ));
    const currentPos = state.scene?.history?.position ?? 0;
    if (isUndo && state.laneColourUndo?.length) {
      const top = state.laneColourUndo[state.laneColourUndo.length - 1];
      if (top.historyPos === currentPos) {
        state.laneColourUndo.pop();
        state.laneColourRedo.push(top);
        if (top.before) state.laneColours.set(top.key, top.before);
        else state.laneColours.delete(top.key);
        ctx.renderActiveLane();
        ctx.renderLanes();
        ctx.paintSoon();
        ctx.saveLaneLayout(true);
        return true;
      }
    }
    if (isRedo && state.laneColourRedo?.length) {
      const top = state.laneColourRedo[state.laneColourRedo.length - 1];
      if (top.historyPos === currentPos) {
        state.laneColourRedo.pop();
        state.laneColourUndo.push(top);
        if (top.after) state.laneColours.set(top.key, top.after);
        else state.laneColours.delete(top.key);
        ctx.renderActiveLane();
        ctx.renderLanes();
        ctx.paintSoon();
        ctx.saveLaneLayout(true);
        return true;
      }
    }
    return false;
  }

  function act(action, full = true) {
    if (tryLaneColourHistory(action)) return Promise.resolve();
    const optimisticTool = optimisticToolFor(action);
    if (optimisticTool && state.scene && !state.view.busy && !state.view.fault && state.view.tool !== optimisticTool) {
      state.view = { ...state.view, tool: optimisticTool };
      ctx.refreshToolCursor();
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
    }
    const requestFull = action?.kind === "tool" ? false : full;
    if (requestFull) state.pendingFullRequests = (state.pendingFullRequests ?? 0) + 1;
    const drawSeq = state.drawHitSeq ?? 0;
    return ctx.run(async () => {
      let effectiveFull = requestFull;
      if (requestFull) {
        const remaining = state.pendingFullRequests;
        state.pendingFullRequests = Math.max(0, remaining - 1);
        // When another full-scene action is already queued behind a rapid pointer edit,
        // skip building and parsing a full 174 KB+ scene for this intermediate step.
        if (remaining > 1 && action?.kind === "pointer") effectiveFull = false;
      }
      await ctx.update(action, effectiveFull, drawSeq);
    });
  }

  const command = (id) => ctx.act({ kind: "command", id });
  const toggle = (id, kind) => $(id).addEventListener("change", () => ctx.act({ kind, on: $(id).checked }));

  // The Svelte parts mount during bundle load, so a part counts as missing only after the page has loaded.
  function missingUiParts() {
    if (env.document.readyState !== "complete") return "";
    return env.document.querySelectorAll("[data-ui-mount]:not([data-ui-ready])").length
      ? "Part of the editor did not load" : "";
  }

  // The map chip in the ribbon. Map metadata comes from the .osu file, so it is set as text only.
  function renderFrame(path) {
    $("map-title").textContent = state.scene?.mapTitle || (path ? nameOf(path).replace(/\.osu$/i, "") : "");
    $("map-version").textContent = state.scene?.version ?? "";
    $("map-version").hidden = !state.scene?.version;
    const hsBadge = $("map-hs-badge");
    if (hsBadge) hsBadge.hidden = !state.scene?.hs;
    $("difficulty").title = path;
  }

  // Save and the Export window's write run in the background and finish here, in a later view:
  // a toast that goes by itself says so. A failed write keeps its error and gets no toast. The
  // last message told is kept, so a view still busy after the write (a sweep) can't swallow it.
  function announceWrite(before, after, hadMap) {
    const status = after.status ?? "";
    const title = /^Saved\b/.test(status) ? "Saved" : /^Wrote \d+ reviewed files?\b/.test(status) ? "Files written" : null;
    if (!title) {
      state.announcedWrite = null;
      return;
    }
    if (!hadMap || after.error) return;
    // The same message again is news only when the map became saved with it (a second Save).
    if (status === state.announcedWrite && !(before.dirty && !after.dirty)) return;
    state.announcedWrite = status;
    if (title === "Saved" && after.isLazerExternalEdit) {
      try {
        notifyToast({
          tone: "success",
          title: "Saved to osu!lazer",
          body: `${status} Click Finish in osu!lazer to apply your changes.`,
        });
      } catch (error) { (env.console ?? console).error(error); }
      return;
    }
    try { notifyToast({ tone: "success", title, body: status }); } catch (error) { (env.console ?? console).error(error); }
    // The reload runs after the Saved toast; only a reload that did not happen raises a second one.
    if (title === "Saved" && after.reloadOsuAfterSave) {
      invoke("reload_osu_editor").then(
        (res) => {
          if (res && res.status !== "reloaded_stable" && res.status !== "lazer_external_edit") notifyToast(osuReloadToast(res));
        },
        (error) => notifyToast(osuReloadFailedToast(error)),
      ).catch((error) => (env.console ?? console).error(error));
    }
  }

  function samePollBase(before, next) {
    return Boolean(
      state.scene && !next.scene && lastActionKind === null &&
      !next.busy && !before.busy && !!before.songLoading === !!next.songLoading &&
      !!before.stemLoading === !!next.stemLoading && !!before.stemLoaded === !!next.stemLoaded &&
      !!before.stemActive === !!next.stemActive &&
      (before.waveformRevision ?? 0) === (next.waveformRevision ?? 0) &&
      next.hasDocument && !next.showHome &&
      !!before.dirty === !!next.dirty && (before.fault ?? null) === (next.fault ?? null) &&
      before.left === next.left && before.right === next.right && before.cycle === next.cycle &&
      before.activeLane === next.activeLane && before.tool === next.tool &&
      before.follow === next.follow && before.offset === next.offset &&
      before.songMuted === next.songMuted && before.songSolo === next.songSolo &&
      before.hitsoundsMuted === next.hitsoundsMuted && before.hitsoundsSolo === next.hitsoundsSolo &&
      before.song === next.song && before.main === next.main && before.hitsounds === next.hitsounds &&
      !next.revert && !next.review && !next.editDialog && !next.saveProblemReview
    );
  }

  function show(next, drawSeq = state.drawHitSeq ?? 0) {
    if (next.fault) {
      state.pendingDrawHits = [];
      // A new object, so the Svelte parts see the fault and disable.
      state.view = { ...state.view, fault: next.fault };
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
      setStatusNotice(next.fault);
      const close = () => ctx.run(() => invoke("close_editor", { discard: false }));
      ctx.modal("fault", "Editor stopped", [paragraph(next.fault)], [["Close dawsu!", close, true]], close);
      return;
    }
    const previousTool = state.view.tool;
    const wasBusy = state.view.busy;
    const before = state.view;
    const hadMap = !!state.scene;
    // Where the line is now, from the view before this one.
    const drawn = ctx.playheadPosition();
    let heldPendingSeek = false;
    if (state.pendingSeek && state.scene && state.pendingSeek.path === state.scene.path && !next.scene) {
      const scrubbing = state.rulerDrag?.mode === "scrub" || Boolean(state.rulerSeekTarget || state.rulerSeekDrain);
      const tolerance = next.playing ? 250 : 1;
      const arrived = Number.isFinite(next.position) && Math.abs(next.position - state.pendingSeek.ms) <= tolerance;
      const expired = (env.window?.performance ?? performance).now() - state.pendingSeek.at > 400;
      if (!scrubbing && (arrived || expired)) {
        state.pendingSeek = null;
      } else {
        // Ignore stale in-flight poll/seek positions while our optimistic seek is still settling.
        next = { ...next, position: state.pendingSeek.ms };
        heldPendingSeek = true;
      }
    } else if (state.pendingSeek && next.scene && next.scene.path !== state.pendingSeek.path) {
      state.pendingSeek = null;
    }
    if (typeof next.stateRev === "number") state.lastTelemetryStateRev = next.stateRev;
    if (next.tick === true) {
      const { tick: _tick, ...tickFields } = next;
      const merged = { ...state.view, ...tickFields };
      if (state.scene && before.playing === merged.playing) {
        state.view = merged;
        if (!heldPendingSeek) ctx.anchorPlayhead(drawn);
        ctx.followBookmarkPick();
        ctx.followPlayhead();
        ctx.paintSoon(true);
        if (state.view.playing) ctx.animatePlayback();
        try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
        lastActionKind = null;
        return;
      }
      next = merged;
    }
    const waveformChanged = (Boolean(before.songLoading) && !next.songLoading) ||
      (Boolean(before.stemLoading) && !next.stemLoading) ||
      (Boolean(before.stemActive) !== Boolean(next.stemActive)) ||
      ((before.waveformRevision ?? 0) !== (next.waveformRevision ?? 0));
    if (waveformChanged) waveformFetchKey = null;
    if (next.recovery === undefined && !next.hasDocument && before.recovery) {
      next = { ...next, recovery: before.recovery };
    }
    state.view = next;
    if (!heldPendingSeek) ctx.anchorPlayhead(drawn);
    if (state.view.tool !== previousTool) ctx.refreshToolCursor();
    ctx.setQuasimodePreset?.(state.view.keyPreset);
    if (!state.view.hasDocument || state.view.showHome) {
      state.scene = null;
      state.pendingDrawHits = [];
      state.selectionVolumeScope = "hits";
      state.selectionLanesOnly = false;
      waveformFetchKey = null;
    }
    if (state.view.scene) {
      const loaded = state.view.scene.path !== state.scene?.path || (wasBusy && state.view.status === "Difficulty loaded");
      if (loaded) {
        state.pendingDrawHits = [];
        state.overviewDrag = null;
        state.overviewTarget = null;
        state.rulerSeekTarget = null;
        state.pendingSeek = null;
        state.usedLaneKeys = null;
        state.markedRange = null;
        state.selectionLaneKeys.clear();
        state.selectionLaneAnchor = null;
        state.selectionVolumeScope = typeof state.view.volumeScope === "string" ? state.view.volumeScope : "hits";
        state.selectionLanesOnly = state.selectionVolumeScope === "selected";
        state.laneFilterText = "";
      } else if (!state.drag && typeof state.view.volumeScope === "string") {
        state.selectionVolumeScope = state.view.volumeScope;
        state.selectionLanesOnly = state.selectionVolumeScope === "selected";
      }
      if (loaded || !state.layoutWrites) ctx.hydrateLaneLayout(state.view.scene.laneLayout);
      if (loaded) {
        ctx.ui()?.laneOptions?.(false, "", state.groupBySampleSet !== false);
        const zeroLanes = (state.view.scene.laneOrder ?? state.view.scene.lanes ?? []).length === 0;
        if (state.view.scene.starterMode === "custom_only" && zeroLanes && state.autoOpenedBrowserFor !== state.view.scene.path) {
          state.autoOpenedBrowserFor = state.view.scene.path;
          const hasMapsetSamples = (state.view.scene.lanes ?? []).some(l => Boolean(l?.sample));
          if (!hasMapsetSamples && state.view.hitsoundLibrary) {
            state.browserSource = "library";
          }
          if (state.view.panels?.browser === false) {
            ctx.togglePanel?.("browser");
          }
        }
      }
      // A lane that starts showing (a new layer from +, Ctrl+L or the lane menu, one Add lane shows
      // again, an Undo of −) shows at once: its family unfolds.
      const shownRows = incoming => incoming.laneOrder ?? incoming.lanes?.map((_, row) => row) ?? [];
      const before = !loaded && state.scene ? new Set(shownRows(state.scene)) : null;
      const unfolded = before ? shownRows(state.view.scene).filter(row => !before.has(row))
        .filter(row => state.collapsedFamilies.delete(state.view.scene.lanes[row]?.family)) : [];
      // Centre keeps the playhead locked in the middle while playing, paused, seeking, or zooming.
      const centring = !loaded && state.scene && state.view.follow && ctx.ui()?.centre?.() && elements.grid.clientWidth > 0;
      if (centring) state.view.scene.viewStart = ctx.playheadPosition() - elements.grid.clientWidth / state.view.scene.zoom / 2;
      // Static map data (waveform, gameplay objects, combos, timing sections) is omitted on
      // incremental edits for the same loaded document so clicks and drags stay fast. It is kept on
      // the "Difficulty loaded" frame too: the editor sent it once, in the frame before.
      if (state.scene && state.view.scene.path === state.scene.path &&
          state.view.scene.previewKey === state.scene.previewKey) {
        if (!waveformChanged) {
          state.view.scene.waveform ??= state.scene.waveform;
          state.view.scene.waveformPyramid ??= state.scene.waveformPyramid;
        }
        state.view.scene.gameplay ??= state.scene.gameplay;
        state.view.scene.combos ??= state.scene.combos;
        state.view.scene.sections ??= state.scene.sections;
      }
      state.scene = state.view.scene;
      if (state.pendingDrawHits?.length) {
        state.pendingDrawHits = state.pendingDrawHits.filter(hit => (hit.seq ?? 0) > drawSeq);
      }
      ensureWaveform(state.scene);
      if (unfolded.length) ctx.saveLaneLayout();
    } else if (waveformChanged && state.scene) {
      state.scene.waveform = undefined;
      state.scene.waveformPyramid = undefined;
      ensureWaveform(state.scene);
    }
    const pollBase = samePollBase(before, next);
    // Fast path for pure 12-Hz playback polls: skip DOM/menu/dialog churn when only playhead/peaks moved.
    const purePlaybackPoll = pollBase && Boolean(
      before.playing && next.playing &&
      (before.outputProblems?.length ?? 0) === (next.outputProblems?.length ?? 0)
    );
    if (purePlaybackPoll) {
      ctx.followBookmarkPick();
      ctx.followPlayhead();
      ctx.paintSoon(true);
      ctx.animatePlayback();
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
      return;
    }
    // Fast path for 1-Hz idle polls while paused: skip DOM/menu/dialog/canvas work when nothing changed.
    const idlePoll = pollBase && Boolean(
      !before.playing && !next.playing &&
      sameUiView(before, next) &&
      Object.is(before.position, next.position) &&
      (before.musicalPosition ?? null) === (next.musicalPosition ?? null) &&
      (before.error ?? null) === (next.error ?? null) &&
      (before.rampShape ?? "Linear") === (next.rampShape ?? "Linear") &&
      !!before.hasOriginalSnapshot === !!next.hasOriginalSnapshot &&
      !!before.canUndoRevert === !!next.canUndoRevert &&
      sameSoundSetPrompts(before.soundSetPrompts, next.soundSetPrompts) &&
      !before.revert && !before.review && !before.editDialog && !before.saveProblemReview &&
      !before.pendingHome && !next.pendingHome &&
      !before.pendingOpen && !next.pendingOpen &&
      !before.saveReview && !next.saveReview &&
      !before.firstLaunch && !next.firstLaunch &&
      next.bookmarkStep == null
    );
    if (idlePoll) {
      const hadBookmarkPick = state.bookmarkPick;
      ctx.followBookmarkPick();
      if (hadBookmarkPick && !state.bookmarkPick) ctx.renderActiveLane();
      if (state.rulerPointer || state.rulerDrag) ctx.refreshRulerCursor();
      try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
      const missing = missingUiParts();
      if (missing || lastStatusNotice) setStatusNotice(missing);
      return;
    }

    ctx.followBookmarkPick();
    if (state.rulerPointer || state.rulerDrag) ctx.refreshRulerCursor();
    $("home").hidden = !!state.scene;
    $("workspace").hidden = !state.scene;
    const path = state.scene?.path ?? "";
    ctx.renderFrame(path);
    $("editor-header").dataset.empty = String(!state.scene);
    const problems = state.view.outputProblems ?? [];
    const listKey = JSON.stringify(problems);
    if (state.outputProblemList !== listKey) {
      state.outputProblemList = listKey;
      state.outputProblemIndex = -1;
      state.rulerWarningMarkers = [];
      elements.ruler.title = "";
      if (state.modalKind === "output-problem") ctx.closeModal();
    }
    $("save-problem-badge").hidden = problems.length === 0;
    $("save-problem-badge").textContent = problems.length ? String(problems.length) : "";
    // One wording for the count: the badge, Save's name and the status bar.
    const cannotSave = problems.length + (problems.length === 1 ? " moment" : " moments") + " can't be saved";
    $("save-problem-badge").title = cannotSave;
    // Save's name says what it holds: unsaved changes and the moments Save can't write. Save is
    // icon-only below 1440 px, so its tooltip says the same.
    const save = $("save");
    const saveName = "Save" + (state.scene && state.view.dirty ? " (unsaved changes)" : "") + (problems.length ? ", " + cannotSave : "");
    save.dataset.dirty = String(!!state.scene && !!state.view.dirty);
    save.setAttribute("aria-label", saveName);
    save.title = saveName;
    const docDisabled = !state.scene || !!state.view.busy;
    for (const button of env.document.querySelectorAll("[data-doc]")) {
      if (button.disabled !== docDisabled) button.disabled = docDisabled;
    }
    const keysoundsActive = Boolean($("editor-arrange")?.classList?.contains?.("keysounds-active"));
    $("samples-view").ariaPressed = String(!keysoundsActive && !state.view.drumView?.enabled);
    $("drums-view").ariaPressed = String(!keysoundsActive && !!state.view.drumView?.enabled);
    const keysoundsViewBtn = $("keysounds-view");
    if (keysoundsViewBtn) keysoundsViewBtn.ariaPressed = String(keysoundsActive);
    $("drums-view").title = state.view.drumView?.name ?? "Open a drum map";
    ctx.syncDrumVst3Button?.();
    $("ramp-shape").value = state.view.rampShape ?? "Linear";
    $("song-mute").ariaPressed = String(!!state.view.songMuted);
    $("song-solo").ariaPressed = String(!!state.view.songSolo);
    const stemButton = $("song-stem");
    if (stemButton) {
      const stemLoaded = !!state.view.stemLoaded;
      const stemLoading = !!state.view.stemLoading;
      const stemActive = !!state.view.stemActive;
      const stemName = state.view.stemName || "stem";
      stemButton.ariaPressed = String(stemActive);
      stemButton.dataset.loaded = String(stemLoaded);
      if (stemLoading) {
        stemButton.textContent = "Stem…";
        stemButton.title = `Loading drum stem (${stemName})…`;
        stemButton.setAttribute("aria-label", `Loading drum stem ${stemName}`);
      } else if (stemLoaded) {
        stemButton.textContent = stemActive ? "Stem" : "Song";
        stemButton.title = stemActive
          ? `Monitoring stem: ${stemName} — click or press T for Song (right-click to clear)`
          : `Monitoring song (stem ready: ${stemName}) — click or press T for Stem (right-click to clear)`;
        stemButton.setAttribute("aria-label", stemActive ? `Monitoring stem ${stemName}` : `Monitoring song, switch to stem ${stemName}`);
      } else {
        stemButton.textContent = "+ Stem";
        stemButton.title = "Attach a drum stem audio file (T toggles Song / Stem)";
        stemButton.setAttribute("aria-label", "Attach drum stem");
      }
    }

    ctx.renderPanelLayout();
    if (state.view.scene) ctx.renderScene();
    else ctx.renderSelection();
    if (state.scene) ctx.followPlayhead();
    ctx.refreshLaneToolbar();
    ctx.refreshHitCard();
    ctx.renderDialogs();
    const toolOnlyUpdate = !next.scene && lastActionKind === "tool";
    const playheadOnlyUpdate = !next.scene &&
      (lastActionKind === "seek" || lastActionKind === "ruler_locators") &&
      before.left === state.view.left &&
      before.right === state.view.right &&
      before.cycle === state.view.cycle &&
      before.offset === state.view.offset;
    lastActionKind = null;
    if (!toolOnlyUpdate && !playheadOnlyUpdate) {
      try {
        if (env !== globalThis) env.window.refreshEditorMenu?.();
        else callAction("refreshEditorMenu");
      } catch (error) { (env.console ?? console).error(error); }
    }
    // A playback poll or playhead-only seek without a scene moved only the playhead: the cached canvas layers still hold.
    ctx.paintSoon(Boolean((state.view.playing && !next.scene) || playheadOnlyUpdate));
    if (state.scene && state.view.playing) ctx.animatePlayback();
    if (state.view?.isPracticeMapset && state.scene?.path) {
      const folder = folderOfPath(state.scene.path);
      if (folder && folder !== state.announcedPracticeFolder) {
        state.announcedPracticeFolder = folder;
        const storage = (() => {
          try {
            return env !== globalThis ? env.localStorage : globalThis.localStorage;
          } catch {
            return null;
          }
        })();
        markPracticeTried(storage);
        notifyToast({
          tone: "info",
          title: "Practice mapset opened",
          body: "Editing a safe temporary 2-diff sandbox — your osu! Songs folder is untouched.",
        });
      }
    }
    try { ctx.ui()?.update?.(state.view, state.scene); } catch (error) { (env.console ?? console).error(error); }
    setStatusNotice(missingUiParts());
    announceWrite(before, state.view, hadMap);
  }

  Object.assign(ctx, {
    invoke, setStatusNotice, run, update, act, command, toggle,
    missingUiParts, renderFrame, show, startTelemetry, parseTelemetryFrame,
  });
}
