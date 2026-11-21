// Export-adjacent dialogs: Merge settings, Generate, New difficulty, and Copy hitsounds.
import { clock, nameOf } from "./helpers.js";

export function attachCopyDialogs(ctx) {
  const { env, state, dom } = ctx;
  const { makeButton, paragraph, input, labeled } = dom;
  const { invoke } = env.window.__TAURI__.core;

  async function openMergeSettings(combinations = [], regenerate = false, includeSliderAudio = false) {
    if (!state.scene || state.view.busy) return;
    const result = await invoke("editor_export_settings");
    if (!result || result[0] !== state.scene?.path) return;
    const [map, current] = result;
    const reviewedPairing = current.PairSamples;
    let value = current;
    const form = env.document.createElement("div");
    const status = paragraph("");
    status.setAttribute("role", "status");
    let priorityIndex = 0;
    const OptionCtor = env.Option ?? Option;
    function renderSettings() {
      const paired = env.document.createElement("input");
      paired.type = "checkbox";
      paired.checked = value.PairSamples;
      paired.addEventListener("change", () => {
        value.PairSamples = paired.checked;
        priorityIndex = 0;
        renderSettings();
      });
      const priority = env.document.createElement("select");
      priority.setAttribute("aria-label", "Sample slot priority");
      priority.replaceChildren(...value.Priorities.map((slot, i) => new OptionCtor((i + 1) + ". " + slot, i)));
      priority.value = String(priorityIndex);
      const move = step => {
        const from = Number(priority.value);
        const to = from + step;
        if (to < 0 || to >= value.Priorities.length) return;
        [value.Priorities[from], value.Priorities[to]] = [value.Priorities[to], value.Priorities[from]];
        priorityIndex = to;
        renderSettings();
      };
      const order = env.document.createElement("div");
      order.className = "row";
      order.append(priority, makeButton("Move up", () => move(-1)), makeButton("Move down", () => move(1)));
      const nodes = [
        labeled("Pair samples by hitsound role", paired),
        paragraph("Priority chooses the output sample slot. A merge rule can override the priority for its combination."),
        order,
      ];
      if (reviewedPairing === value.PairSamples && combinations.length) {
        const combo = env.document.createElement("select");
        combo.setAttribute("aria-label", "Sample combination");
        combo.replaceChildren(...combinations.map(({ layers, time }, i) =>
          new OptionCtor(layers.join(" + ") + " · " + clock(time), i)));
        combo.value = "0";
        const slot = env.document.createElement("select");
        slot.setAttribute("aria-label", "Override output slot");
        const displayRule = () => {
          const group = combinations[Number(combo.value)];
          slot.replaceChildren(new OptionCtor("Use priority", ""),
            ...value.Priorities.map(entry => new OptionCtor(entry, entry)));
          slot.value = value.Rules[group.key] ?? "";
        };
        combo.addEventListener("change", displayRule);
        slot.addEventListener("change", () => {
          const { key } = combinations[Number(combo.value)];
          if (slot.value) value.Rules[key] = slot.value;
          else delete value.Rules[key];
        });
        displayRule();
        nodes.push(labeled("Combination", combo), labeled("Output slot", slot));
      }
      form.replaceChildren(...nodes);
    }
    renderSettings();
    const load = async () => {
      try {
        const loaded = await invoke("pick_merge_preset");
        if (!loaded || state.modalKind !== "merge-settings" || state.scene?.path !== map) return;
        value = loaded;
        priorityIndex = 0;
        renderSettings();
        status.textContent = "Loaded " + Object.keys(value.Rules).length + " combination rules.";
      } catch (error) {
        status.textContent = String(error);
      }
    };
    const save = async () => {
      try {
        if (await invoke("save_merge_preset", { value })) status.textContent = "Preset saved.";
      } catch (error) {
        status.textContent = String(error);
      }
    };
    const apply = () => {
      ctx.closeModal();
      if (state.scene?.path !== map) return;
      ctx.run(async () => {
        await ctx.update({ kind: "export_settings", map, value });
        if (regenerate && !state.view.error && state.scene?.path === map) {
          await ctx.update({ kind: "generate", include_slider_audio: includeSliderAudio });
        }
      });
    };
    ctx.modal("merge-settings", "Merge settings", [form, status],
      [["Cancel", () => ctx.closeModal()], ["Load preset…", load], ["Save preset…", save], [regenerate ? "Review again" : "Apply", apply, true]]);
  }

  function openGenerate() {
    if (!state.scene || state.view.busy) return;
    const sliderAudio = input("checkbox", "", 0, 0);
    sliderAudio.checked = false;
    ctx.modal("generate", "Generate hitsound difficulty",
      [paragraph("Generate from " + nameOf(state.scene.path)), labeled("Include slider slide, whistle and tick audio", sliderAudio)],
      [["Cancel", () => ctx.closeModal()], ["Review output", () => {
        ctx.closeModal();
        ctx.act({ kind: "generate", include_slider_audio: sliderAudio.checked });
      }, true]]);
  }

  function openNewDifficulty() {
    const name = input("text", "", 0, 0);
    name.placeholder = "Difficulty name";
    const submit = () => {
      if (name.value.trim()) {
        ctx.closeModal();
        ctx.act({ kind: "new_difficulty", name: name.value.trim() });
      }
    };
    name.addEventListener("keydown", (event) => { if (event.key === "Enter") submit(); });
    ctx.modal("new", "New difficulty", [labeled("Name", name)], [["Cancel", () => ctx.closeModal()], ["Review output", submit, true]]);
    name.focus();
  }

  function openCopy() {
    const paths = state.scene.difficulties.filter((path) => path !== state.scene.path);
    if (!paths.length) {
      ctx.modal("copy", "Copy hitsounds", [paragraph("There are no other difficulties in this mapset.")], [["Close", () => ctx.closeModal(), true]]);
      return;
    }

    const checkboxes = paths.map(() => input("checkbox", "", 0, 0));
    const choices = paths.map((path, i) => labeled(nameOf(path), checkboxes[i]));
    const leniency = input("number", 5, 0, 1000, "any");
    const startIndex = input("number", 100, 2, 2147483647);
    const mode = env.document.createElement("select");
    for (const [value, label] of [
      ["overwrite_everything", "Overwrite everything"], ["overwrite_only_defined", "Keep unmatched hits"],
    ]) {
      const option = env.document.createElement("option");
      option.value = value;
      option.textContent = label;
      mode.append(option);
    }
    mode.value = "overwrite_everything";

    const check = (on) => {
      const box = input("checkbox", "", 0, 0);
      box.checked = on;
      return box;
    };
    const hitsounds = check(true);
    const laterWins = check(false);
    const bodies = check(true);
    const ticks = check(false);
    const slides = check(false);
    const sets = check(true);
    const volumes = check(true);
    const keep5 = check(true);
    const neverSilent = check(false);
    const storyboard = check(false);
    const satisfied = check(true);
    const whenever = check(false);
    const mute = check(false);
    const defaultDivisors = "1, 4, 3, 8, 6, 16, 12";
    const defaultMutedDivisors = "4, 3, 8, 6, 16, 12";
    const defaultMinLength = 0.5;
    const maxSampleIndex = 4294967295;
    const divisors = input("text", defaultDivisors, 0, 0);
    const mutedDivisors = input("text", defaultMutedDivisors, 0, 0);
    const minLength = input("number", defaultMinLength, 0, "", "any");
    const mutedIndex = input("number", "", 0, maxSampleIndex);
    mutedIndex.placeholder = "Keep";
    const mutedSet = env.document.createElement("select");
    for (const [value, label] of [["", "Keep"], ["normal", "Normal"], ["soft", "Soft"], ["drum", "Drum"]]) {
      const option = env.document.createElement("option");
      option.value = value;
      option.textContent = label;
      mutedSet.append(option);
    }
    mutedSet.value = "";

    const copySliderAudio = () => mode.value === "overwrite_only_defined" && hitsounds.checked && sets.checked;
    const updateEnabled = () => {
      bodies.disabled = !hitsounds.checked;
      ticks.disabled = slides.disabled = !copySliderAudio();
      keep5.disabled = !volumes.checked;
      satisfied.disabled = whenever.disabled = !storyboard.checked;
      for (const field of [divisors, mutedDivisors, minLength, mutedIndex, mutedSet]) {
        field.disabled = !mute.checked;
      }
    };
    for (const field of [mode, hitsounds, sets, volumes, storyboard, mute]) field.addEventListener("change", updateEnabled);
    updateEnabled();

    const positiveDivisors = (text) => {
      const tokens = text.split(/[,\s]+/).filter(Boolean);
      if (tokens.some((value) => !/^\d+$/.test(value) || Number(value) < 1 || Number(value) > 4294967295)) return null;
      return tokens.map(Number);
    };
    const error = paragraph("");
    error.setAttribute("role", "alert");
    const submit = () => {
      const targets = paths.filter((_, i) => checkboxes[i].checked);
      if (!targets.length) {
        error.textContent = "Select at least one target difficulty.";
        return;
      }
      const start = startIndex.valueAsNumber;
      if (!Number.isInteger(start) || start < 2 || start > 2147483647) {
        error.textContent = "The start index must be a whole number from 2 to 2147483647.";
        return;
      }
      const tolerance = leniency.valueAsNumber;
      if (!Number.isFinite(tolerance) || tolerance < 0 || tolerance > 1000) {
        error.textContent = "Enter a timing tolerance from 0 to 1000 ms.";
        return;
      }
      const beatDivisors = positiveDivisors(mute.checked ? divisors.value : defaultDivisors);
      const mutedBeats = positiveDivisors(mute.checked ? mutedDivisors.value : defaultMutedDivisors);
      const length = mute.checked ? minLength.valueAsNumber : defaultMinLength;
      const index = mute.checked && mutedIndex.value !== "" ? mutedIndex.valueAsNumber : null;
      if (!beatDivisors || !mutedBeats) {
        error.textContent = "Use positive whole numbers for beat divisors.";
        return;
      }
      if (!Number.isFinite(length) || length < 0) {
        error.textContent = "Enter a minimum slider length of 0 beats or more.";
        return;
      }
      if (index !== null && (!Number.isInteger(index) || index < 0 || index > maxSampleIndex)) {
        error.textContent = "Enter a valid custom sample index, or leave it blank to keep the existing index.";
        return;
      }
      ctx.closeModal();
      ctx.act({
        kind: "copy", targets, options: {
          mode: mode.value, leniency: tolerance,
          collision_policy: laterWins.checked ? "later_wins" : "merge",
          start_index: start, never_silent_hitnormals: neverSilent.checked,
          copy_hitsounds: hitsounds.checked, copy_body_hitsounds: bodies.checked, copy_sample_sets: sets.checked,
          copy_to_slider_ticks: copySliderAudio() && ticks.checked,
          copy_to_slider_slides: copySliderAudio() && slides.checked,
          copy_volumes: volumes.checked, always_preserve_5_volume: keep5.checked,
          copy_storyboarded_samples: storyboard.checked,
          ignore_hitsound_satisfied_samples: satisfied.checked, ignore_whenever_hitsound: whenever.checked,
          mute_slider_ends: mute.checked, beat_divisors: beatDivisors, muted_divisors: mutedBeats,
          min_length: length, muted_index: index, muted_sample_set: mute.checked && mutedSet.value ? mutedSet.value : null,
        },
      });
    };

    const details = env.document.createElement("details");
    const summary = env.document.createElement("summary");
    summary.textContent = "Advanced options";
    details.append(summary, labeled("Start index", startIndex), labeled("Never create silent hitnormals", neverSilent),
      labeled("Copy storyboard samples", storyboard),
      labeled("Add timed samples at slider ticks", ticks), labeled("Add timed samples during slides", slides),
      paragraph("Timed storyboard samples play independently of slider tracking and preserve the existing slider sounds."),
      paragraph("Enable Keep unmatched hits, Copy hitsounds, and Copy sample sets and indexes to add timed samples."),
      labeled("Skip samples a target hit already plays", satisfied), labeled("Skip samples at any hit", whenever),
      labeled("Mute unmatched slider and spinner ends", mute), labeled("Beat divisors (highest priority first)", divisors),
      labeled("Muted beat divisors", mutedDivisors), labeled("Minimum slider length (beats)", minLength),
      labeled("Muted sample index", mutedIndex), labeled("Muted sample set", mutedSet));
    ctx.modal("copy", "Copy hitsounds", [
      paragraph("Copy from " + nameOf(state.scene.path)),
      ...choices, labeled("Timing tolerance (ms)", leniency), labeled("Unmatched target hits", mode),
      labeled("Later moment wins on nearby hits", laterWins),
      labeled("Copy hitsounds", hitsounds), labeled("Copy slider-body sounds", bodies),
      labeled("Copy sample sets and indexes", sets), labeled("Copy volumes", volumes), labeled("Keep target hits at 5%", keep5),
      details, error,
    ], [["Cancel", () => ctx.closeModal()], ["Review output", submit, true]]);
  }

  Object.assign(ctx, { openMergeSettings, openGenerate, openNewDifficulty, openCopy });
}
