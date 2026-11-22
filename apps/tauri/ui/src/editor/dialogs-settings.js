// Settings, folders, audio output, view preferences, help search, and shortcut editor dialogs.
import { keyInput, playbackStartChoices, shortcutName } from "./helpers.js";

export function attachSettingsDialogs(ctx) {
  const { env, state, dom } = ctx;
  const { $, makeButton, paragraph, input, labeled } = dom;
  const { invoke } = env.window.__TAURI__.core;

  // File › Folders… and the start screen's Folders… button.
  function openFolders() {
    if (state.view.busy) return;
    const openFolder = (folder) => {
      ctx.closeModal();
      ctx.run(() => invoke("open_editor_folder", { folder }));
    };
    const chooseSongsFolder = () => {
      ctx.closeModal();
      ctx.run(async () => {
        const next = await invoke("pick_songs_folder");
        if (next) ctx.show(next);
      });
    };
    const chooseLibrary = () => {
      ctx.closeModal();
      ctx.run(async () => {
        const next = await invoke("pick_hitsound_library");
        if (!next) return;
        state.browserSource = "library";
        state.browserLibraryFolders = new Map();
        ctx.resetBrowserFolder();
        state.browserState = "";
        ctx.show(next);
      });
    };
    const actions = [["Cancel", () => ctx.closeModal()]];
    if (state.scene) {
      actions.push(["Open map folder", () => openFolder("map")],
        ["Open project folder", () => openFolder("project")]);
    }
    actions.push(["Choose Songs folder…", chooseSongsFolder]);
    if (state.view.hitsoundLibrary) actions.push(["Open hitsound library", () => openFolder("library")]);
    actions.push(["Choose hitsound library…", chooseLibrary]);
    ctx.modal("folders", "Folders", [], actions);
  }

  function openAudioOutput() {
    return ctx.run(async () => {
      const settings = await invoke("audio_outputs", { backend: null });
      const backend = env.document.createElement("select");
      const output = env.document.createElement("select");
      const buffer = input("number", settings.bufferMs ?? "", 10, 500);
      const current = paragraph("");
      const outputError = paragraph("");
      const locked = () => Boolean(state.view.playing || state.view.busy);
      let loadedBackend = null;

      function addOption(select, value, label) {
        const option = env.document.createElement("option");
        option.value = value;
        option.textContent = label;
        select.append(option);
      }
      function showDevices(config) {
        output.replaceChildren();
        addOption(output, "", config.backend === "asio" ? "Choose an ASIO driver" : "Device default");
        for (const device of config.devices) addOption(output, device.id, device.name);
        output.value = config.device ?? "";
        current.textContent = [config.currentBackend, config.current,
          config.effectiveBufferMs == null ? null : config.effectiveBufferMs + " ms effective buffer",
        ].filter(Boolean).join(" · ");
        const error = config.error ?? "";
        outputError.textContent = error;
        outputError.hidden = !error;
        loadedBackend = error ? null : config.backend;
        output.disabled = locked() || Boolean(error);
        applyOutput.disabled = output.disabled || (config.backend === "asio" && !output.value);
      }

      const applyOutput = makeButton("Apply output", () => {
        if (locked() || applyOutput.disabled || loadedBackend !== backend.value) return;
        const ms = buffer.value === "" ? null : Number(buffer.value);
        if (ms !== null && (!Number.isInteger(ms) || ms < 10 || ms > 500)) {
          buffer.reportValidity();
          return;
        }
        const action = { kind: "output_settings", backend: backend.value, device_id: output.value || null, ms };
        applyOutput.disabled = true;
        ctx.run(async () => {
          await ctx.update(action, false);
          if (!state.view.error && state.modalKind === "audio" && backend.value === action.backend) {
            showDevices(await invoke("audio_outputs", { backend: action.backend }));
          }
        });
      });
      output.addEventListener("change", () => {
        applyOutput.disabled = locked() || loadedBackend !== backend.value || (backend.value === "asio" && !output.value);
      });
      for (const choice of settings.available) addOption(backend, choice.id, choice.label);
      backend.value = settings.backend;
      backend.disabled = locked() || settings.available.length < 2;
      backend.addEventListener("change", () => {
        if (locked()) return;
        const requested = backend.value;
        loadedBackend = null;
        output.disabled = true;
        applyOutput.disabled = true;
        ctx.run(async () => {
          const config = await invoke("audio_outputs", { backend: requested });
          if (requested === backend.value && state.modalKind === "audio") showDevices(config);
        });
      });
      buffer.id = "audio-buffer";
      buffer.placeholder = "Device default";
      buffer.title = "Clear to use the device default buffer";
      buffer.disabled = locked();
      showDevices(settings);

      const song = input("range", state.view.song, 0, 1, 0.01);
      const main = input("range", state.view.main, 0.2, 1, 0.01);
      const mute = input("checkbox", "", 0, 0);
      mute.checked = state.view.songMuted;
      const offset = input("number", state.view.offset, -60000, 60000, 1);
      const start = env.document.createElement("select");
      for (const [value, label] of playbackStartChoices) {
        const option = env.document.createElement("option");
        option.value = value;
        option.textContent = label;
        start.append(option);
      }
      start.value = state.view.playbackStart ?? "cursor";
      start.addEventListener("change", () => ctx.act({ kind: "playback_start", value: start.value }, false));
      const returnOnStop = input("checkbox", "", 0, 0);
      returnOnStop.checked = state.view.returnOnStop;
      returnOnStop.addEventListener("change", () => ctx.act({ kind: "return_on_stop", on: returnOnStop.checked }, false));
      const gains = () => ctx.act({ kind: "gains", song: +song.value, main: +main.value }, false);
      song.addEventListener("change", gains);
      main.addEventListener("change", gains);
      mute.addEventListener("change", () => ctx.act({ kind: "song_muted", on: mute.checked }, false));
      offset.addEventListener("change", () => ctx.act({ kind: "offset", ms: offset.valueAsNumber || 0 }, false));
      ctx.modal("audio", "Audio", [
        current, labeled("Output backend", backend), labeled("Output device", output),
        labeled("Requested buffer (ms)", buffer), applyOutput, outputError,
        ...(state.view.playing ? [paragraph("Stop playback to change audio output.")] : []),
        labeled("Song volume", song), labeled("Master volume", main),
        labeled("Mute song", mute), labeled("Song offset (ms)", offset),
        labeled("Start from", start), labeled("Return on stop", returnOnStop),
      ], [["Done", () => ctx.closeModal(), true]]);
    });
  }

  // View › View settings….
  function openViewSettings() {
    if (!state.scene || !state.view.panels || state.view.busy) return;
    const options = [
      ["Inspector", "inspector", ctx.panelOn("inspector")],
      ["Volume and Mixer", "controller", state.view.panels.controller],
      ["Lane names", "lane_names", state.view.panels.laneNames],
      ["Browser", "browser", ctx.panelOn("browser")],
    ];
    const info = env.document.createElement("input");
    info.type = "checkbox";
    info.checked = state.showLaneInfo;
    info.disabled = !state.view.panels.laneNames;
    info.addEventListener("change", () => {
      state.showLaneInfo = info.checked;
      ctx.renderLanes();
      ctx.paintSoon();
    });
    const controls = options.map(([label, panel, on]) => {
      const checkbox = env.document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = on;
      checkbox.addEventListener("change", () => {
        if (panel === "lane_names") info.disabled = !checkbox.checked;
        if (panel === "lane_names") ctx.act({ kind: "panel", panel, on: checkbox.checked });
        else if (checkbox.checked !== ctx.panelOn(panel)) ctx.togglePanel(panel);
      });
      return ctx.labeled(label, checkbox);
    });
    const browserStyleSelect = env.document.createElement("select");
    for (const [value, label] of [["tree", "Folder tree · sidebar"], ["classic", "Classic · inspector"]]) {
      const option = env.document.createElement("option");
      option.value = value;
      option.textContent = label;
      browserStyleSelect.append(option);
    }
    browserStyleSelect.value = state.view.classicBrowser ? "classic" : "tree";
    browserStyleSelect.addEventListener("change", () => ctx.act({ kind: "browser_style", classic: browserStyleSelect.value === "classic" }));
    controls.push(ctx.labeled("Browser style", browserStyleSelect), ctx.labeled("Lane info (sample indexes)", info));
    ctx.modal("editor-view", "View", controls, [["Done", () => ctx.closeModal(), true]]);
  }

  async function showHelp() {
    const groups = await invoke("editor_help");
    const search = env.document.createElement("input");
    search.type = "search";
    search.value = "";
    const results = env.document.createElement("div");

    function renderMatches() {
      const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
      const content = [];
      for (const [title, entries] of groups) {
        const matches = entries.filter(([keys, description]) => {
          const text = [title, keys, description].join(" ").toLowerCase();
          return terms.every(term => text.includes(term));
        });
        if (!matches.length) continue;
        const heading = env.document.createElement("h3");
        heading.textContent = title;
        content.push(heading, ...matches.map(([keys, description]) =>
          paragraph(keys ? [keys, description].join(" — ") : description)));
      }
      results.replaceChildren(...(content.length ? content : [paragraph("No matching shortcuts or mouse actions.")]));
    }

    search.addEventListener("input", renderMatches);
    renderMatches();
    ctx.modal("help", "Shortcuts and mouse actions", [labeled("Search", search), results], [
      ["Customise shortcuts…", () => ctx.run(showShortcutEditor)],
      ["Close", () => ctx.closeModal(), true],
    ]);
    search.focus();
  }

  async function showShortcutEditor() {
    let shortcuts = await invoke("editor_shortcuts");
    // Command ids carry no group, so each row goes under the area F1 help lists it in.
    const groupOf = new Map();
    for (const [title, entries] of await invoke("editor_help")) {
      if (title.endsWith("(mouse)")) continue;
      for (const [, description] of entries) if (!groupOf.has(description)) groupOf.set(description, title);
    }
    // {id} while a row waits for a key; with chord, name and owner while it asks to take another command's key.
    let editing = null;
    let focusId = null;
    const rows = new Map();
    const search = env.document.createElement("input");
    search.type = "search";
    search.value = "";
    search.placeholder = "Search commands or keys";
    search.setAttribute("aria-label", "Search commands or keys");
    const presets = env.document.createElement("div");
    presets.className = "shortcut-presets";
    presets.setAttribute("role", "group");
    presets.setAttribute("aria-label", "Key preset");
    for (const [label, name] of [["osu! editor controls", "osu!"], ["DAW-like keys", "DAW"]]) {
      const button = makeButton(label, () => {
        if (shortcuts.preset !== name) refresh(() => ctx.update({ kind: "key_preset", name }));
      });
      button.dataset.preset = name;
      presets.append(button);
    }
    const toolbar = env.document.createElement("div");
    toolbar.className = "shortcut-toolbar";
    toolbar.append(presets, search);
    const note = paragraph("Click a command's keys, then press the key to add. It applies at once. Each preset keeps its own keys.");
    note.className = "shortcut-note";
    const table = env.document.createElement("div");
    table.className = "shortcut-table";
    const summary = env.document.createElement("p");
    summary.className = "dim";
    summary.setAttribute("role", "status");

    function keyChips(label) {
      const combo = env.document.createElement("span");
      combo.className = "key-combo";
      const [, ctrl, shift, alt, key] = /^(Ctrl\+)?(Shift\+)?(Alt\+)?(.+)$/.exec(label);
      [ctrl && "Ctrl", shift && "Shift", alt && "Alt", key].filter(Boolean).forEach((part, index) => {
        const chip = env.document.createElement("kbd");
        chip.textContent = part;
        combo.append(...(index ? ["+", chip] : [chip]));
      });
      return combo;
    }

    // Redraws the row being left and the row being edited; returns the edited row's control.
    function edit(id, next) {
      const previous = editing?.id;
      editing = next;
      if (previous && previous !== id) rows.get(previous)?.();
      return rows.get(id)();
    }

    function press(event, entry, capture) {
      if (["Control", "Meta", "Alt", "Shift"].includes(event.key) || event.repeat) return;
      const chord = keyInput(event);
      if (chord.key === "Tab" && !chord.ctrl && !chord.alt) return;
      event.preventDefault();
      const [id, , keys] = entry;
      // A plain Escape cancels waiting; it can't be assigned from the editor.
      if (chord.key === "Escape" && !chord.ctrl && !chord.shift && !chord.alt) return edit(id, null).focus();
      const name = shortcutName(chord);
      if (!name) {
        capture.textContent = "That key can't be a shortcut. Press another.";
        return;
      }
      if (keys.includes(name)) return edit(id, null).focus();
      const owner = shortcuts.entries.find(([, , other]) => other.includes(name));
      if (owner) return edit(id, { id, chord, name, owner }).focus();
      editing = null;
      focusId = id;
      refresh(() => ctx.update({ kind: "change_shortcut", id, chord }));
    }

    function fillRow(row, entry) {
      const [id, description, keys] = entry;
      const mine = editing?.id === id ? editing : null;
      row.className = "shortcut-row" + (mine ? (mine.owner ? " conflict" : " capturing") : "");
      const commandLabel = env.document.createElement("span");
      commandLabel.className = "shortcut-command";
      commandLabel.textContent = description;
      commandLabel.title = id;
      let target;
      if (mine?.owner) {
        const conflict = env.document.createElement("span");
        conflict.className = "shortcut-conflict";
        conflict.textContent = "Also " + mine.owner[1].replace(/^[A-Z](?=[a-z])/, (c) => c.toLowerCase()) + " · " + mine.name;
        target = makeButton("Move here", () => {
          editing = null;
          focusId = id;
          refresh(() => ctx.update({ kind: "change_shortcut", id, chord: mine.chord }));
        });
        target.setAttribute("aria-label", `Move ${mine.name} to ${description}`);
        row.replaceChildren(commandLabel, conflict, keyChips(mine.name), target, makeButton("Cancel", () => edit(id, null).focus()));
      } else if (mine) {
        target = env.document.createElement("span");
        target.className = "shortcut-capture";
        target.tabIndex = 0;
        target.setAttribute("role", "textbox");
        target.setAttribute("aria-label", "Press the new key for " + description);
        target.textContent = "Press keys… · Esc cancels";
        target.addEventListener("keydown", (event) => press(event, entry, target));
        target.addEventListener("blur", () => { if (editing === mine) edit(id, null); });
        row.replaceChildren(commandLabel, target);
      } else {
        target = env.document.createElement("button");
        target.className = "shortcut-keys";
        target.title = "Add a key";
        target.setAttribute("aria-label", `${description}: ${keys.join(", ") || "no keys"}. Add a key`);
        if (keys.length) target.append(...keys.map(keyChips));
        else target.textContent = "Unassigned";
        target.addEventListener("click", () => edit(id, { id }).focus());
        const clear = makeButton("Clear", () => {
          focusId = id;
          refresh(() => ctx.update({ kind: "change_shortcut", id, chord: null }));
        });
        clear.className = "shortcut-clear";
        clear.disabled = !keys.length;
        clear.setAttribute("aria-label", "Clear the keys of " + description);
        row.replaceChildren(commandLabel, target, clear);
      }
      return target;
    }

    function render() {
      for (const button of presets.children) button.setAttribute("aria-pressed", String(button.dataset.preset === shortcuts.preset));
      const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
      const groups = new Map([...new Set(groupOf.values())].map((title) => [title, []]));
      rows.clear();
      for (const entry of shortcuts.entries) {
        const [id, description, keys] = entry;
        const text = [id, description, ...keys].join(" ").toLowerCase();
        if (!terms.every((term) => text.includes(term))) continue;
        const row = env.document.createElement("div");
        row.dataset.command = id;
        rows.set(id, () => fillRow(row, entry));
        rows.get(id)();
        const title = groupOf.get(description) ?? "Other";
        if (!groups.has(title)) groups.set(title, []);
        groups.get(title).push(row);
      }
      const sections = [...groups].filter(([, list]) => list.length).map(([title, list]) => {
        const section = env.document.createElement("section");
        section.className = "shortcut-group";
        const heading = env.document.createElement("h3");
        heading.textContent = title;
        section.append(heading, ...list);
        return section;
      });
      const body = $("dialog-body");
      const top = body.scrollTop;
      table.replaceChildren(...(sections.length ? sections : [paragraph("No commands match.")]));
      body.scrollTop = top;
      if (focusId && rows.has(focusId)) rows.get(focusId)().focus();
      focusId = null;
    }

    function refresh(task) {
      return ctx.run(async () => {
        try {
          await task();
          if (state.modalKind !== "shortcut-editor" || state.view.error) return;
          shortcuts = await invoke("editor_shortcuts");
          summary.textContent = state.view.status ?? "";
          editing = null;
          render();
        } catch (error) {
          summary.textContent = String(error);
        }
      });
    }

    search.addEventListener("input", render);
    render();
    ctx.modal("shortcut-editor", "Customise shortcuts", [toolbar, note, table, summary], [
      ["Import…", () => refresh(async () => { if (await invoke("import_key_file")) await ctx.update(null, false); })],
      ["Export…", () => refresh(async () => { if (await invoke("export_key_file")) await ctx.update(null, false); })],
      ["Close", () => ctx.closeModal(), true],
    ]);
    $("dialog-body").scrollTop = 0;
    search.focus();
  }

  // Edit › Preferences…: the Svelte modal registers openPreferencesModal once it mounts and drops it
  // when it unmounts, so a modal that failed to mount falls back to the View dialog instead of
  // calling this function again through the global of the same name.
  function openPreferences(tab = "general") {
    const modal = env.openPreferencesModal ?? globalThis.openPreferencesModal;
    if (typeof modal === "function" && modal !== openPreferences) {
      try {
        modal(tab);
        return;
      } catch (error) {
        console.error(error);
      }
    }
    openViewSettings();
  }

  Object.assign(ctx, { openFolders, openAudioOutput, openViewSettings, openPreferences, showHelp, showShortcutEditor });
}
