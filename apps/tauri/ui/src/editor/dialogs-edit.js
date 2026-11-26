// Editing, lane, bookmark, difficulty switching, and time-signature dialogs.
import { clock, laneGrey } from "./helpers.js";
import { showToast } from "../lib/registry.js";
import { activeBarMs, rhythmPresets } from "../lib/rhythmGallery.js";

export function attachEditDialogs(ctx) {
  const { env, state, dom, elements } = ctx;
  const { $, makeButton, paragraph, input, labeled } = dom;
  const { invoke } = env.window.__TAURI__.core;
  const notifyToast = toast => (env !== globalThis ? env.window?.showToast?.(toast) : showToast(toast));

  // The Inspector's Lane colour (and the lane menu, by its id): opens on grey until the mapper picks a colour.
  function openLaneColour() {
    const path = state.scene?.path;
    const lane = state.scene?.lanes?.[state.view.activeLane];
    if (!lane?.colourKey || state.view.busy || state.drag) return;
    const picked = state.laneColours.get(lane.colourKey);
    // Apply writes only a colour the mapper chose, so an unpicked lane stays without one.
    let chosen = !!picked;
    const colourInput = env.document.createElement("input");
    colourInput.type = "color";
    colourInput.value = picked ?? laneGrey;
    const label = env.document.createElement("label");
    label.append("Custom colour", colourInput);
    const palette = env.document.createElement("div");
    palette.className = "tint-palette";
    palette.setAttribute("role", "radiogroup");
    palette.setAttribute("aria-label", "Lane colour");
    const preview = env.document.createElement("div");
    preview.className = "tint-preview";
    const caption = env.document.createElement("span");
    caption.textContent = lane.name;
    const sample = env.document.createElement("small");
    sample.textContent = lane.sample ?? "";
    caption.append(sample);
    preview.append(caption);
    for (let i = 0; i < 5; i++) preview.append(env.document.createElement("i"));
    const refresh = () => {
      preview.style.setProperty("--preview-colour", colourInput.value);
      for (const radio of palette.children) radio.checked = radio.value === colourInput.value;
    };
    const shades = [
      "d04932 db642b d78628 deab27 c9bc30 a6be32 70be3d 44ba4b 23b76c 21af91 24abb2 289dc9 388ada 5776d4 7464ce 945ac5 b751bb cc50a2",
      "b0412e b95728 b57326 bb9125 aa9f2c 8da12e 61a137 3d9e43 229b5e 20957c 239197 2686aa 3376b8 4c66b3 6457ae 7e4fa7 9b479e ac478a",
      "91392b 984b26 966124 9a7923 8d8429 76862b 538632 36833b 218150 1f7c68 21797e 24708d 2e6498 435794 554b90 6a458a 813f84 8f3e73",
      "74342a 794126 775125 7a6224 706a29 606b2a 476b2f 326935 226845 226456 236266 255c70 2c5378 3b4976 494173 583c6f 68386a 72385e",
    ];
    for (const shade of shades.join(" ").split(" ")) {
      const radio = env.document.createElement("input");
      radio.type = "radio";
      radio.name = "lane-swatch";
      radio.value = "#" + shade;
      radio.title = radio.value;
      radio.setAttribute("aria-label", radio.value);
      radio.style.backgroundColor = radio.value;
      radio.addEventListener("change", () => { colourInput.value = radio.value; chosen = true; refresh(); });
      palette.append(radio);
    }
    colourInput.addEventListener("input", () => { chosen = true; refresh(); });
    refresh();
    const change = (colour) => {
      ctx.closeModal();
      if (state.scene?.path !== path || state.view.busy || state.drag) return;
      const before = state.laneColours.get(lane.colourKey) ?? null;
      const after = colour ?? null;
      if (before !== after) {
        state.laneColourUndo.push({
          key: lane.colourKey,
          before,
          after,
          historyPos: state.scene?.history?.position ?? 0,
        });
        state.laneColourRedo = [];
      }
      if (colour) state.laneColours.set(lane.colourKey, colour);
      else state.laneColours.delete(lane.colourKey);
      ctx.renderActiveLane();
      ctx.renderLanes();
      ctx.paintSoon();
      ctx.saveLaneLayout(true);
    };
    const buttons = [["Cancel", () => ctx.closeModal()]];
    if (state.laneColours.has(lane.colourKey)) buttons.push(["Clear colour", () => change(null)]);
    buttons.push(["Apply", () => (chosen ? change(colourInput.value) : ctx.closeModal()), true]);
    ctx.modal("lane-colour", "Lane colour · " + lane.name, [palette, preview, label], buttons);
  }

  function openReplaceSample() {
    const target = ctx.sampleTarget();
    if (!target) return;
    const select = env.document.createElement("select");
    const samples = new Map();
    for (const [path, row] of ctx.mapsetSamples()) {
      samples.set(row, path);
      const option = env.document.createElement("option");
      option.value = String(row);
      option.textContent = path;
      select.append(option);
    }
    const chosen = () => samples.has(Number(select.value)) && select.value !== "" ? Number(select.value) : null;
    const assign = () => {
      const row = chosen();
      if (row === null) return;
      ctx.closeModal();
      ctx.act({ kind: "replace_sample", target, path: samples.get(row), mapset: true });
    };
    const browse = () => {
      ctx.closeModal();
      ctx.pickSample(target);
    };
    const available = samples.size > 0;
    ctx.modal("replace-sample", "Replace sample on " + state.scene.lanes[state.view.activeLane].name,
      available ? [labeled("Mapset sample", select)] : [], [
        ["Cancel", () => ctx.closeModal()],
        ...(available ? [["Audition", () => {
          const row = chosen();
          if (row !== null && state.scene?.path === target.map) {
            ctx.act({ kind: "lane", row, operation: "preview_sample" }, false);
          }
        }], ["Replace", assign, true]] : []),
        ["Browse…", browse, !available],
      ]);
  }

  // File › Open map… and Ctrl+O's fallback; the start screen's Open map… button.
  function openMap() {
    return ctx.run(async () => ctx.show(await invoke("pick_map")));
  }

  async function createAndOpenHitsoundDiff(siblingPath, reuseExisting = false, starterMode = null) {
    if (!siblingPath || state.view.busy) return;
    try {
      const created = await invoke("create_hitsound_diff", {
        siblingPath,
        reuseExisting: Boolean(reuseExisting),
        starterMode: starterMode || null,
      });
      if (created) {
        await ctx.act({ kind: "open", path: created });
      }
    } catch (error) {
      ctx.modal("error", "Could not create Hitsound diff", [paragraph(String(error))], [["OK", () => ctx.closeModal(), true]]);
    }
  }

  async function pickAndCreateHitsoundDiff() {
    if (state.view.busy) return;
    const picked = await invoke("pick_map_for_hitsound_diff");
    if (!picked) return;
    const parent = picked.replace(/[\\/][^\\/]+$/, "");
    if (parent && parent !== picked) {
      await openMapFolderPath(parent);
    } else {
      await createAndOpenHitsoundDiff(picked, true, null);
    }
  }

  async function openMapFolderPath(folderPath) {
    if (!folderPath || state.view.busy) return;
    try {
      const res = await invoke("inspect_mapset_folder", { path: folderPath });
      if (!res) return;
      if (res.kind === "open") {
        await ctx.act({ kind: "open", path: res.path });
        return;
      }
      if (res.kind === "choose" && Array.isArray(res.difficulties) && res.difficulties.length) {
        ctx.ui()?.openMapsetIntake?.(res);
      }
    } catch (error) {
      ctx.modal("error", "Could not open folder", [paragraph(String(error))], [["OK", () => ctx.closeModal(), true]]);
    }
  }

  async function openMapFolder() {
    if (state.view.busy) return;
    const folder = await invoke("pick_map_folder");
    if (folder) await openMapFolderPath(folder);
  }

  // File › Open from osu! and the start screen's Open from osu! button.
  function openFromOsu() {
    return ctx.run(async () => {
      try {
        const path = await invoke("open_from_osu");
        if (path) await ctx.act({ kind: "open", path });
      } catch (reason) {
        env.window?.showToast?.({
          tone: "warning",
          title: "Open from osu!",
          body: String(reason),
        });
      }
    });
  }

  // File › Start screen.
  function goHome() {
    if (!state.drag && !state.view.busy) ctx.act({ kind: "home" });
  }

  // File › Revert mapset to original…: puts existing .osu files in this mapset folder back the way
  // they were when dawsu! first opened it. Deleted difficulties stay deleted unless opted in.
  function confirmRevertToOriginal() {
    if (!state.scene || !state.view.hasDocument || !state.view.hasOriginalSnapshot || state.view.busy || state.drag) return;
    const map = state.scene.path;
    return ctx.run(async () => {
      let preview;
      try {
        preview = await invoke("mapset_revert_action", { action: { kind: "preview" } });
      } catch (reason) {
        notifyToast({
          tone: "warning",
          title: "Revert mapset to original",
          body: String(reason),
        });
        return;
      }
      if (state.scene?.path !== map || state.view.busy || state.drag) return;
      const deletedChecks = (preview.deletedFiles ?? []).map(name => {
        const box = input("checkbox", "", 0, 0);
        box.checked = false;
        return { name, box, row: labeled(`${name} was deleted since then`, box) };
      });
      const content = [
        paragraph(`Snapshot saved on ${preview.snapshotDate}. Every existing .osu file in this mapset folder goes back to how it was when dawsu! first opened it.`),
        paragraph("This undoes all changes since then, saved and unsaved, in every difficulty of this mapset. Undo can't bring them back."),
        paragraph("dawsu! saves a backup of the current files first, so nothing is lost for good."),
        ...(deletedChecks.length ? [
          paragraph("Deleted difficulties stay deleted unless you choose to bring them back:"),
          ...deletedChecks.map(entry => entry.row),
        ] : []),
      ];
      ctx.modal("revert-original", "Revert mapset to original?", content, [
        ["Cancel", () => ctx.closeModal(), true],
        ["Revert mapset", () => {
          const includeDeleted = deletedChecks.filter(entry => entry.box.checked).map(entry => entry.name);
          ctx.closeModal();
          ctx.run(async () => {
            if (state.scene?.path !== map || state.view.busy) return;
            try {
              await invoke("mapset_revert_action", { action: { kind: "revert", includeDeleted } });
              await ctx.update();
            } catch (reason) {
              notifyToast({
                tone: "error",
                title: "Revert mapset to original",
                body: String(reason),
              });
            }
          });
        }],
      ]);
    });
  }

  // File › Undo revert: restores existing .osu files from the newest pre-revert backup for this mapset.
  function undoMapsetRevert() {
    if (!state.scene || !state.view.hasDocument || !state.view.canUndoRevert || state.view.busy || state.drag) return;
    const map = state.scene.path;
    return ctx.run(async () => {
      if (state.scene?.path !== map || state.view.busy) return;
      try {
        await invoke("mapset_revert_action", { action: { kind: "undo" } });
        await ctx.update();
      } catch (reason) {
        notifyToast({
          tone: "error",
          title: "Undo revert",
          body: String(reason),
        });
      }
    });
  }

  function openSiblingDifficulty(path) {
    if (state.scene && !state.view.busy && !state.drag && path && path !== state.scene.path) {
      ctx.act({ kind: "open", path });
    }
  }

  function chooseSiblingDifficulty() {
    const ribbon = $("difficulty");
    if (!state.scene || !ribbon.getClientRects().length || !ribbon.options.length) return false;
    const currentPath = state.scene.path;
    const OptionCtor = env.Option ?? Option;
    const choices = env.document.createElement("select");
    choices.id = "difficulty-choice";
    const diffOptions = [...ribbon.options].filter(
      option => !option.value.startsWith("__"),
    );
    choices.size = Math.min(8, diffOptions.length);
    choices.setAttribute("aria-label", "Difficulty in the current mapset");
    choices.append(...diffOptions.map(option => new OptionCtor(option.text, option.value)));
    choices.value = currentPath;
    const open = () => {
      ctx.closeModal();
      if (state.scene?.path === currentPath) openSiblingDifficulty(choices.value);
    };
    choices.addEventListener("keydown", event => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      open();
    });
    choices.addEventListener("dblclick", open);
    ctx.modal("difficulty-chooser", "Switch difficulty", [choices], [
      ["Cancel", () => ctx.closeModal()],
      ["+ Create Hitsound diff", () => {
        ctx.closeModal();
        createAndOpenHitsoundDiff(currentPath, false);
      }],
      [state.scene.hs ? "Turn off HS mode" : "Turn on HS mode", () => {
        ctx.closeModal();
        ctx.act({ kind: "hs_mode", on: !state.scene.hs });
      }],
      ["Open", open, true],
    ]);
    choices.focus();
    return true;
  }

  function showTimelineEdit(kind) {
    if (state.modalKind === "timeline-edit" && elements.dialog.open) return;
    const path = state.scene.path;
    const cancel = () => { ctx.closeModal(); ctx.act({ kind: "cancel_timeline_edit" }); };
    const submit = (edit) => { ctx.closeModal(); ctx.act({ kind: "timeline_edit", path, edit: { kind, ...edit } }); };
    const fail = paragraph("");
    fail.setAttribute("role", "alert");
    const choice = (options, initial) => {
      const select = env.document.createElement("select");
      for (const [value, label] of options) {
        const option = env.document.createElement("option");
        option.value = value;
        option.textContent = label;
        select.append(option);
      }
      select.value = initial;
      return select;
    };
    const openEdit = (title, controls, apply, label = "Apply") =>
      ctx.modal("timeline-edit", title, [...controls, fail], [["Cancel", cancel], [label, apply, true]], cancel);
    if (kind === "quantize") {
      const strength = input("number", state.view.quantizeStrength ?? 100, 1, 100, "any");
      openEdit("Quantize", [labeled("Strength (%)", strength)], () => {
        const value = strength.valueAsNumber;
        if (Number.isFinite(value) && value >= 1 && value <= 100) {
          submit({ strength: value });
        } else fail.textContent = "Enter a strength from 1% to 100%.";
      }, "Quantize");
    } else if (kind === "repeat") {
      const copies = input("number", 1, 1, 64);
      const gap = input("number", state.scene.repeatGap ?? 1, 1, 1024);
      openEdit("Repeat", [labeled("Copies", copies), labeled("Gap in snap steps", gap)], () => {
        const n = copies.valueAsNumber, steps = gap.valueAsNumber;
        if (Number.isInteger(n) && n >= 1 && n <= 64 && Number.isInteger(steps) && steps >= 1 && steps <= 1024) {
          submit({ copies: n, gap_steps: steps });
        } else fail.textContent = "Enter 1–64 copies and a gap of 1–1024 steps.";
      }, "Repeat");
    } else if (kind === "pattern_stamp") {
      const presets = rhythmPresets(activeBarMs(state.scene));
      const count = input("number", 4, 1, 64);
      const step = input("number", +(activeBarMs(state.scene)).toFixed(3), 1, "", "any");
      const snapToObjects = input("checkbox", "", 0, 0);
      snapToObjects.checked = true;
      const quantizeVolumes = input("checkbox", "", 0, 0);
      const presetStrip = env.document.createElement("div");
      presetStrip.className = "rhythm-preset-strip";
      presetStrip.setAttribute("role", "group");
      presetStrip.setAttribute("aria-label", "Rhythm presets");
      const chipButtons = [];
      for (const preset of presets) {
        const chip = makeButton(preset.modalLabel, () => {
          count.value = String(preset.count);
          step.value = String(preset.stepMs);
          for (const btn of chipButtons) btn.setAttribute("aria-pressed", String(btn === chip));
        });
        chip.className = "rhythm-preset-chip";
        chip.title = preset.title;
        chip.setAttribute("aria-pressed", String(preset.id === "bar"));
        chipButtons.push(chip);
        presetStrip.append(chip);
      }
      openEdit("Pattern stamp", [
        presetStrip,
        labeled("Repeats", count),
        labeled("Step (ms)", step),
        labeled("Snap to gameplay objects", snapToObjects),
        labeled("Quantize volumes (35 / 70 / 100%)", quantizeVolumes),
      ], () => {
        const n = count.valueAsNumber, ms = step.valueAsNumber;
        if (Number.isInteger(n) && n >= 1 && n <= 64 && Number.isFinite(ms) && ms > 0) {
          ctx.closeModal();
          ctx.act({
            kind: "pattern_stamp",
            count: n,
            step_ms: ms,
            snap_to_objects: snapToObjects.checked,
            quantize_volumes: quantizeVolumes.checked,
          });
        } else fail.textContent = "Enter 1–64 repeats and a positive step in milliseconds.";
      }, "Stamp");
    } else if (kind === "range" && state.scene.range) {
      const operation = choice([["copy", "Copy"], ["move", "Move"], ["delete", "Delete"]], "copy");
      const offset = input("number", +(state.scene.range[1] - state.scene.range[0]).toFixed(3), "", "", "any");
      operation.addEventListener("change", () => { offset.disabled = operation.value === "delete"; });
      openEdit("Edit range", [labeled("Operation", operation), labeled("Offset (ms)", offset)], () => {
        const ms = operation.value === "delete" ? 0 : offset.valueAsNumber;
        if (Number.isFinite(ms)) {
          submit({ operation: operation.value, offset: ms, rows: state.markedRange?.path === path ? state.markedRange.rows : null });
        } else fail.textContent = "Enter a time offset in milliseconds.";
      });
    } else if (kind === "scale") {
      const scope = choice([["selected", "Selected hits"], ["selected_lanes", "Lanes of selected hits"],
        ["all", "All hits"]], state.scene.selection ? "selected" : "all");
      const low = input("number", 1, 0, 100);
      const high = input("number", 100, 0, 100);
      const percent = input("number", 100, 0, "", "any");
      openEdit("Scale by volume", [labeled("Applies to", scope), labeled("From volume (%)", low),
        labeled("To volume (%)", high), labeled("Scale by (%)", percent),
        paragraph("Every sound it applies to is scaled by the same factor, so their balance stays. A sound that reaches 1% or 100% stops there; a silent sound stays silent.")], () => {
        const min = low.valueAsNumber, max = high.valueAsNumber, scale = percent.valueAsNumber;
        if ([min, max].every(value => Number.isInteger(value) && value >= 0 && value <= 100) &&
            Number.isFinite(scale) && scale >= 0) {
          submit({ scope: scope.value, low: min, high: max, percent: scale });
        } else fail.textContent = "Enter volumes from 0% to 100% and a scale of at least 0%.";
      }, "Scale");
    }
  }

  function editBookmark(mark) {
    if (!state.scene || state.view.busy) return;
    const path = state.scene.path;
    const time = mark?.time ?? state.view.position;
    const label = input("text", mark?.label ?? "", 0, 0);
    const submit = () => {
      ctx.closeModal();
      ctx.act({ kind: mark ? "bookmark_edit" : "bookmark_add", path, time, label: label.value });
    };
    label.addEventListener("keydown", (event) => { if (event.key === "Enter") submit(); });
    ctx.modal("bookmark", mark ? "Rename bookmark" : "Add bookmark",
      [paragraph(clock(time)), labeled("Label", label)],
      [["Cancel", () => ctx.closeModal()], [mark ? "Rename" : "Add", submit, true]]);
    label.focus();
  }

  function openTimeSignatures() {
    if (!state.scene || state.view.busy) return;
    const path = state.scene.path;
    const field = env.document.createElement("textarea");
    field.rows = 6;
    field.cols = 28;
    field.setAttribute("aria-label", "Bar and time signature changes");
    field.placeholder = "33 7/8";
    field.value = (state.scene.timeSignatures ?? []).map(({ bar, numerator, denominator }) =>
      `${bar} ${numerator}/${denominator}`).join("\n");
    const error = paragraph("");
    error.setAttribute("role", "alert");
    ctx.modal("time-signatures", "Time signatures", [
      paragraph("Enter one change per line, such as 33 7/8 for 7/8 from bar 33. These changes affect the editor ruler."),
      field,
      error,
    ], [
      ["Cancel", () => ctx.closeModal()],
      ["Apply", async () => {
        if (state.scene?.path !== path) return ctx.closeModal();
        const value = [];
        for (const line of field.value.trim().split("\n").filter(entry => entry.trim())) {
          const parts = /^(\d+)\s+(\d+)\s*\/\s*(\d+)$/.exec(line.trim());
          const [bar, numerator, denominator] = parts?.slice(1).map(Number) ?? [];
          if (!bar || bar > 4294967295 || numerator < 1 || numerator > 32 || ![1, 2, 4, 8, 16].includes(denominator)) {
            error.textContent = "Use bar 1 or later, 1–32 beats, and a beat unit of 1, 2, 4, 8 or 16.";
            return;
          }
          value.push({ bar, numerator, denominator });
        }
        ctx.closeModal();
        await ctx.act({ kind: "time_signatures", map: path, value });
      }, true],
    ]);
  }

  Object.assign(ctx, {
    openLaneColour, openReplaceSample, openMap, openMapFolder, openMapFolderPath,
    createAndOpenHitsoundDiff, pickAndCreateHitsoundDiff, openFromOsu, goHome, confirmRevertToOriginal, undoMapsetRevert,
    openSiblingDifficulty, chooseSiblingDifficulty, showTimelineEdit,
    editBookmark, openTimeSignatures,
  });
}
