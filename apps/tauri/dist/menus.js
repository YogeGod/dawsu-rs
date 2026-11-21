"use strict";

const menuPopup = $("editor-menu");
// The bar's order; a test checks it against menuBar in ui/src/lib/commands.js and index.html.
const menuNames = ["file", "edit", "playback", "map", "view"];
let editorMenu = null;
let menuOpens = 0;

function canEditFromMenu() {
  return !!scene && !view.busy && !drag;
}

// Rules a row's `enabled` or `checked` names in ui/src/lib/commands.js; `target` is the right-clicked lane.
const laneAllowed = predicate => target => {
  const row = laneMenuRow(target);
  return row !== -1 && predicate(scene.lanes[row], row);
};
const menuRules = {
  edit: canEditFromMenu,
  idle: () => !view.busy && !drag,
  document: () => !!scene && !view.busy,
  selection: () => canEditFromMenu() && !!scene.selection?.count,
  range: () => canEditFromMenu() && !!scene.range,
  placement: () => canEditFromMenu() && scene.placement != null,
  undo: () => canEditFromMenu() && scene.historyPosition !== 0,
  redo: () => canEditFromMenu() && scene.historyPosition !== scene.history?.length,
  deleteLane: () => canEditFromMenu() && !!deletableLane(),
  never: () => false,
  lane: laneAllowed(() => true),
  laneRename: laneAllowed(lane => !!lane.colourKey && !view.drumView?.enabled),
  laneUp: laneAllowed((_, row) => adjacentLaneForMove(row, -1) !== null),
  laneDown: laneAllowed((_, row) => adjacentLaneForMove(row, 1) !== null),
  laneReplace: laneAllowed(lane => lane.canReplace),
  laneColour: laneAllowed(lane => lane.colourKey),
  laneLayer: laneAllowed(lane => lane.canAddLayer),
  laneDelete: laneAllowed(lane => lane.canDelete),
  waveform: () => $("ruler-waveform").getAttribute("aria-pressed") === "true",
  usedOnly: () => !!$("used-only").checked,
  mapPreview: () => !!window.dawsuUi?.mapPreviewOn?.(),
  nodeGraph: () => !!window.dawsuUi?.nodeGraphOn?.(),
  tool: (_, item) => view.tool === item.run[1],
};

// The segmented choice rows, filled when their menu opens.
const menuChoiceSources = {
  difficulty: {
    list: () => scene?.path ? Array.from($("difficulty").options ?? [], option => [option.value, option.text]) : [],
    current: () => scene?.path,
    choose: path => openSiblingDifficulty(path),
  },
  playbackStart: {
    list: () => playbackStartChoices,
    current: () => view.playbackStart ?? "cursor",
    choose: value => act({ kind: "playback_start", value }, false),
  },
  speed: {
    list: () => speedButtons.map(button => [Number(button.dataset.playbackSpeed), button.textContent.replace("%", "")]),
    current: () => view.speed,
    choose: value => act({ kind: "speed", value }),
  },
  exportStyle: { list: () => [["objects", "Per-object"]], current: () => "objects", choose: () => {} },
};

// What ["call", name] rows run.
const menuCalls = {
  placeSoundSet: () => placeSoundSet(),
  saveSoundSet: () => saveSoundSet(),
  openSoundSetPlacement: () => openSoundSetPlacement(),
  goToEnd: () => act({ kind: "seek", ms: view.duration }),
  openDrumWindow: () => invoke("drum_window_open").catch(error => { $("status").textContent = String(error); }),
  requestDrumMap: () => requestDrumMap(),
  editLayout: () => dock?.editLayout(),
  openMap: () => openMap(),
  goHome: () => goHome(),
  openFolders: () => openFolders(),
  openViewSettings: () => openViewSettings(),
  showHelp: () => run(showHelp),
  deleteActiveLane: () => deleteActiveLane(),
  toggleMapPreview: () => window.dawsuUi?.toggleMapPreview?.(),
  toggleNodeGraph: () => window.dawsuUi?.toggleNodeGraph?.(),
};

const menuField = path => path.split(".").reduce((value, key) => value?.[key], { view, scene });

function runMenuEntry({ item, target, value, source }) {
  if (source) {
    source.choose(value);
    return;
  }
  const [kind, first, second, third] = item.run ?? ["command", item.command];
  if (kind === "command") command(first);
  else if (kind === "click") $(first)?.click();
  else if (kind === "act") act(first);
  else if (kind === "toggle" && third) act({ kind: first, on: !menuField(second) }, false);
  else if (kind === "toggle") act({ kind: first, on: !menuField(second) });
  else if (kind === "panel" && first !== "lane_names") togglePanel(first);
  else if (kind === "panel") act({ kind: "panel", panel: first, on: !view.panels?.[second ?? first] });
  else if (kind === "tool") act({ kind: "tool", name: first });
  else if (kind === "call") menuCalls[first]();
  else if (kind === "lane" && first === "rename") renameLane(laneMenuRow(target));
  else if (kind === "lane") moveLane(laneMenuRow(target), first === "up" ? -1 : 1);
  else if (kind === "onLane") {
    laneMenuAction(target, row => first === "layer" ? act({ kind: "lane", row, operation: "add_layer" })
      : first === "call" ? menuCalls[second]() : $(second)?.click());
  }
}

// A switch, tool or choice row's state; undefined for a plain row.
function menuChecked({ item, target, value, source }) {
  if (source) return source.current() === value;
  if (item.checked) return !!menuRules[item.checked](target, item);
  const [kind, first, second] = item.run ?? [];
  if (kind === "toggle") return !!menuField(second);
  if (kind === "panel" && first !== "lane_names") return panelOn(first);
  if (kind === "panel") return !!view.panels?.[second ?? first];
  return undefined;
}

const menuEnabled = entry => !!menuRules[entry.item.enabled ?? "edit"](entry.target, entry.item);

// One menu's rows from the command list, with one entry per button in page order.
function menuEntries(name, target) {
  const list = window.dawsuUi?.commands;
  if (!list?.menus[name]) {
    $("status").textContent = "The menus could not load. Restart dawsu! to try again.";
    return null;
  }
  const byId = new Map(list.items.map(item => [item.id, item]));
  const rows = [];
  const entries = [];
  const entry = (item, extra) => {
    const added = { item, target, index: entries.length, keys: "", ...extra };
    entries.push(added);
    return added;
  };
  const step = offset => scene?.history?.[scene.historyPosition + offset];
  for (const id of list.menus[name]) {
    if (id === "-") {
      if (rows.length && rows[rows.length - 1].type !== "separator") rows.push({ type: "separator" });
      continue;
    }
    const item = byId.get(id);
    if (item.choices) {
      const source = menuChoiceSources[item.choices];
      const choices = source.list();
      if (choices.length) {
        rows.push({ type: "choices", label: item.label,
          choices: choices.map(([value, text]) => entry(item, { value, label: text, source })) });
      }
      continue;
    }
    const action = item.history === undefined ? null : step(item.history);
    rows.push({ type: "item", entry: entry(item, { label: action ? item.label + " · " + action : item.label }) });
  }
  while (rows.length && rows[rows.length - 1].type === "separator") rows.pop();
  return { rows, entries };
}

// The plain model EditorMenu.svelte draws; disabled and checked are worked out again on each refresh.
function menuModel(menu) {
  const plain = entry => {
    const radio = !!(entry.item.radio || entry.source);
    const checked = menuChecked(entry);
    entry.disabled = !menuEnabled(entry);
    return {
      index: entry.index, label: entry.label, icon: entry.source ? null : entry.item.icon ?? null,
      role: radio ? "menuitemradio" : checked !== undefined ? "menuitemcheckbox" : "menuitem",
      className: radio ? "menu-choice" : entry.item.icon ? "menu-item" : "menu-item menu-item-plain",
      checked, disabled: entry.disabled, chip: !!entry.item.command, keys: entry.keys,
      switch: checked !== undefined && !radio,
    };
  };
  return {
    open: menu.open,
    rows: menu.rows.map(row => row.type === "separator" ? row : row.type === "choices"
      ? { type: "choices", label: row.label, choices: row.choices.map(plain) }
      : { type: "item", item: plain(row.entry) }),
  };
}

// A click on a row: EditorMenu.svelte calls this with the row's index.
function menuAction(index) {
  const entry = editorMenu?.entries[index];
  if (!entry || !menuEnabled(entry)) return;
  if (entry.item.close || menuChecked(entry) === undefined) closeEditorMenu();
  runMenuEntry(entry);
  refreshEditorMenu();
}

// The open menu's enabled buttons, in page order.
function menuButtons() {
  if (!editorMenu) return [];
  const buttons = Array.from(menuPopup.querySelectorAll("button"));
  return editorMenu.entries.filter(entry => !entry.disabled).map(entry => buttons[entry.index]).filter(Boolean);
}

function laneMenuRow(target) {
  if (!canEditFromMenu() || scene.path !== target.map) return -1;
  const row = scene.lanes.findIndex(lane => lane.sourceKey === target.key);
  return lanePositions.has(row) ? row : -1;
}

function laneMenuAction(target, invokeAction) {
  run(async () => {
    const row = laneMenuRow(target);
    if (row === -1) return;
    await update({ kind: "lane", row, operation: "activate" }, false);
    if (laneMenuRow(target) === view.activeLane) invokeAction(row);
  });
}

function soundSetsUsable(map) {
  return scene?.path === map && !view.busy && !drag;
}

function chooseSoundSetLibrary(map, then) {
  closeModal();
  run(async () => {
    if (!soundSetsUsable(map)) return;
    const next = await invoke("pick_hitsound_library");
    if (!next) return;
    show(next);
    if (soundSetsUsable(map)) then();
  });
}

function browseSoundSet(map) {
  closeModal();
  run(async () => {
    if (!soundSetsUsable(map)) return;
    const next = await invoke("pick_sound_set", { map });
    if (next) show(next);
  });
}

function showSoundSetLibrary(map, library) {
  const browse = ["Browse…", () => browseSoundSet(map)];
  const chooseFolder = ["Choose library folder…", () => chooseSoundSetLibrary(map, placeSoundSet), true];
  if (!library.configured || library.error) {
    modal("sound-sets", "Place sound set",
      [paragraph(library.error ?? "Sound sets are kept in your hitsound library.")],
      [["Cancel", closeModal], browse, chooseFolder]);
    return;
  }
  if (library.reading) {
    modal("sound-sets", "Place sound set", [paragraph("Still reading the sound set library. Try again.")],
      [["Cancel", closeModal], browse, ["Try again", () => { closeModal(); placeSoundSet(); }, true]]);
    return;
  }
  if (!library.sets.length && !library.unreadable?.length) {
    modal("sound-sets", "Place sound set",
      [paragraph("The library has no sound sets yet. Use Edit > Save selection as sound set to add one.")],
      [["Cancel", closeModal], browse]);
    return;
  }
  const list = document.createElement("select");
  list.id = "sound-set-choice";
  list.size = Math.min(10, library.sets.length + (library.unreadable?.length ?? 0) + 2);
  list.setAttribute("aria-label", "Sound sets");
  const option = (value, text, disabled) => {
    const node = document.createElement("option");
    node.value = value;
    node.textContent = text;
    node.disabled = disabled;
    return node;
  };
  const group = (label, options) => {
    const node = document.createElement("optgroup");
    node.label = label;
    node.append(...options);
    return node;
  };
  const missingOption = set => Object.assign(option(set.id, "Not in library", true), { title: set.id });
  const setOption = set => set.missing ? missingOption(set) : option(set.id,
    set.name + " · " + set.sounds + (set.sounds === 1 ? " sound" : " sounds") +
      (set.duplicate ? " · in the library twice, remove one copy" : ""), set.duplicate);
  const used = library.sets.filter(set => set.inMap).map(setOption);
  const rest = [...library.sets.filter(set => !set.inMap).map(setOption),
    ...(library.unreadable ?? []).map(file => option("", "Could not read " + file, true))];
  list.append(...[used.length && group("In this map", used), rest.length && group("Library", rest)].filter(Boolean));
  const placeable = library.sets.filter(set => !set.duplicate && !set.missing);
  list.value = placeable[0]?.id ?? "";
  const message = paragraph(placeable.length ? "" : "None of these sound sets can be placed.");
  const place = () => {
    const set = placeable.find(item => item.id === list.value);
    if (!set) {
      message.textContent = placeable.length ? "Choose a sound set to place." : "None of these sound sets can be placed.";
      return;
    }
    closeModal();
    if (soundSetsUsable(map)) act({ kind: "place_library_sound_set", map, id: set.id });
  };
  list.addEventListener("keydown", event => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    place();
  });
  list.addEventListener("dblclick", event => {
    if (!event.target?.disabled) place();
  });
  modal("sound-sets", "Place sound set", [list, message], [["Cancel", closeModal], browse, ["Place", place, true]]);
  list.focus?.();
}

function placeSoundSet() {
  const map = scene.path;
  run(async () => {
    if (!soundSetsUsable(map)) return;
    const library = await invoke("sound_set_library");
    if (soundSetsUsable(map)) showSoundSetLibrary(map, library);
  });
}

function soundSetPlacementAction(map, id, op, extra = {}) {
  closeModal();
  if (soundSetsUsable(map)) act({ kind: "sound_set_placement", map, id, op, ...extra });
}

function showSoundSetPlacement(map, placement) {
  const name = paragraph(placement.name ?? placement.frozen ?? "Not in library");
  if (placement.name == null) name.title = placement.setId;
  const content = [name];
  if (placement.sounds) content.push(paragraph(placement.edited + " of " + placement.sounds + " sounds edited"));
  const message = paragraph(placement.frozen ?? "");
  const buttons = [["Close", closeModal], ["Unlink", () => soundSetPlacementAction(map, placement.id, "unlink")]];
  if (placement.edited) buttons.push(["Reset edits", () => soundSetPlacementAction(map, placement.id, "reset")]);
  if (placement.newer) {
    buttons.push(["Update", () => soundSetPlacementAction(map, placement.id, "update"), true]);
  } else if (!placement.frozen) {
    const field = input("number", String(placement.gain), 5, 100);
    content.push(labeled("Volume %", field));
    buttons.push(["Set volume", () => {
      const gain = Number(field.value);
      if (String(field.value).trim() === "" || !Number.isInteger(gain) || gain < 5) {
        message.textContent = "Volume must be at least 5%.";
        return;
      }
      if (gain > 100) {
        message.textContent = "Volume can be at most 100%.";
        return;
      }
      soundSetPlacementAction(map, placement.id, "gain", { gain });
    }, true]);
  }
  if (placement.name != null && (placement.frozen || !placement.newer)) content.push(message);
  modal("sound-set-placement", "Sound set placement", content, buttons);
}

function openSoundSetPlacement() {
  const map = scene.path;
  const id = scene.placement;
  run(async () => {
    if (!soundSetsUsable(map) || id == null) return;
    const placement = await invoke("sound_set_placement", { map, id });
    if (soundSetsUsable(map)) showSoundSetPlacement(map, placement);
  });
}

// Questions closed with Escape this open, by open and set id: they come back on the next open.
const dismissedSoundSetPrompts = new Set();
const soundSetPromptKey = prompt => prompt.open + ":" + prompt.setId;

function pendingSoundSetPrompt() {
  return (view.soundSetPrompts ?? []).find(prompt => !dismissedSoundSetPrompts.has(soundSetPromptKey(prompt)));
}

function showSoundSetPrompt(prompt) {
  const answer = update => { closeModal(); act({ kind: "sound_set_prompt", set_id: prompt.setId, update }); };
  const dismiss = () => { dismissedSoundSetPrompts.add(soundSetPromptKey(prompt)); closeModal(); };
  const count = prompt.placements + (prompt.placements === 1 ? " placement uses" : " placements use");
  const content = [paragraph(count + " an older version of " + prompt.name + ".")];
  if (prompt.editedKeepFiles) {
    content.push(paragraph(prompt.editedKeepFiles === 1 ? "1 edited sound keeps its own file."
      : prompt.editedKeepFiles + " edited sounds keep their own files."));
  }
  modal("sound-set-update", "Sound set changed", content,
    [["Keep", () => answer(false)], ["Update", () => answer(true), true]], dismiss);
}

function saveSoundSet() {
  const map = scene.path;
  const title = "Save selection as sound set";
  if (!view.hitsoundLibrary) {
    modal("save-sound-set", title, [paragraph("Sound sets are kept in your hitsound library.")],
      [["Cancel", closeModal], ["Choose library folder…", () => chooseSoundSetLibrary(map, saveSoundSet), true]]);
    return;
  }
  const field = input("text", "", "", "");
  field.maxLength = 128;
  const message = paragraph("");
  let saving = false;
  let saveButton = null;
  const busy = on => {
    saving = on;
    if (saveButton) {
      saveButton.disabled = on;
      saveButton.textContent = on ? "Saving…" : "Save";
    }
  };
  const save = () => {
    if (saving) return;
    const name = field.value.trim();
    if (!name) {
      message.textContent = "Name the sound set.";
      return;
    }
    busy(true);
    message.textContent = "";
    run(async () => {
      try {
        if (!soundSetsUsable(map)) return;
        const next = await invoke("save_sound_set", { map, name });
        if (next.nameTaken) {
          message.textContent = next.nameTaken;
          field.select?.();
          return;
        }
        closeModal();
        show(next);
      } catch (error) {
        message.textContent = String(error);
      } finally {
        busy(false);
      }
    });
  };
  field.addEventListener("keydown", event => {
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      save();
    }
  });
  modal("save-sound-set", title, [labeled("Name", field), message], [["Cancel", closeModal], ["Save", save, true]]);
  saveButton = Array.from($("dialog-actions").children ?? []).find(button => button.textContent === "Save") ?? null;
  field.focus?.();
}

function closeEditorMenu(restore = true) {
  if (!editorMenu) return;
  const { trigger } = editorMenu;
  editorMenu = null;
  menuPopup.hidden = true;
  window.dawsuUi?.menu?.(null);
  trigger.setAttribute("aria-expanded", "false");
  if (restore) trigger.focus();
}

function refreshEditorMenu() {
  if (!editorMenu) return;
  if (dialog.open) {
    closeEditorMenu(false);
    return;
  }
  // Hand the rows over only when something changed: show() refreshes on every editor update.
  const model = menuModel(editorMenu);
  const drawn = JSON.stringify(model);
  if (drawn === editorMenu.drawn) return;
  editorMenu.drawn = drawn;
  window.dawsuUi?.menu?.(model);
}

function openEditorMenu(name, last = false, point = null) {
  closeEditorMenu(false);
  const built = menuEntries(name, point?.target);
  if (!built) return;
  const trigger = point?.trigger ?? $("menu-" + name);
  const menu = { name, trigger, open: ++menuOpens, ...built };
  editorMenu = menu;
  if (point) {
    menuPopup.removeAttribute("aria-labelledby");
    menuPopup.setAttribute("aria-label", point.label);
  } else {
    menuPopup.removeAttribute("aria-label");
    menuPopup.setAttribute("aria-labelledby", trigger.id);
  }
  // The bundle draws the rows before this returns, so the popup can be measured and focused.
  refreshEditorMenu();
  if (editorMenu !== menu) return;
  menuPopup.hidden = false;
  trigger.setAttribute("aria-expanded", "true");
  const bounds = trigger.getBoundingClientRect();
  const left = point?.x ?? bounds.left;
  const availableHeight = window.innerHeight - 4;
  menuPopup.style.maxHeight = Math.max(0, availableHeight - (point ? 4 : bounds.bottom)) + "px";
  const top = point
    ? Math.max(4, Math.min(point.y, availableHeight - menuPopup.offsetHeight))
    : bounds.bottom;
  menuPopup.style.left = Math.max(4, Math.min(left, window.innerWidth - menuPopup.offsetWidth - 4)) + "px";
  menuPopup.style.top = top + "px";
  const buttons = menuButtons();
  (last ? buttons[buttons.length - 1] : buttons[0])?.focus();

  invoke("editor_shortcuts").then(({ entries }) => {
    if (editorMenu !== menu) return;
    const bindings = new Map(entries.map(([id, , keys]) => [id, keys]));
    for (const entry of menu.entries) {
      // A menu shows a command's first key; the Shortcuts list shows them all (Cycle has three).
      if (entry.item.command) entry.keys = bindings.get(entry.item.command)?.[0] ?? "";
    }
    refreshEditorMenu();
  }).catch(() => {});
}

$("grid").addEventListener("contextmenu", event => {
  if (!scene) return;
  event.preventDefault();
  if (canEditFromMenu() && !dialog.open) {
    openEditorMenu("tool", false, {
      x: event.clientX, y: event.clientY, trigger: $("grid"), label: "Timeline tools",
    });
  }
});

function openLaneMenu(event, keyboard = false) {
  const trigger = event.target.closest("[data-lane-row]");
  if (!trigger || !canEditFromMenu() || dialog.open) return;
  const row = Number(trigger.dataset.laneRow);
  const lane = scene.lanes[row];
  if (!lane?.sourceKey || !lanePositions.has(row)) return;
  event.preventDefault();
  const bounds = trigger.getBoundingClientRect();
  openEditorMenu("lane", false, {
    x: keyboard ? bounds.left : event.clientX,
    y: keyboard ? bounds.bottom : event.clientY,
    trigger, label: "Lane actions", target: { map: scene.path, key: lane.sourceKey },
  });
}

$("lanes").addEventListener("contextmenu", event => openLaneMenu(event));
$("lanes").addEventListener("keydown", event => {
  if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
    openLaneMenu(event, true);
  }
});

for (const name of menuNames) {
  const trigger = $("menu-" + name);
  trigger.addEventListener("click", () => {
    if (editorMenu?.name === name) closeEditorMenu();
    else openEditorMenu(name);
  });
  trigger.addEventListener("keydown", event => {
    if (!["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(event.key)) return;
    event.preventDefault();
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      openEditorMenu(name, event.key === "ArrowUp");
      return;
    }
    const offset = event.key === "ArrowLeft" ? -1 : 1;
    const next = menuNames[(menuNames.indexOf(name) + offset + menuNames.length) % menuNames.length];
    $("menu-" + next).focus();
  });
}

document.addEventListener("keydown", event => {
  if (!editorMenu || event.defaultPrevented) return;
  const key = event.key;
  if (key === "Tab") {
    closeEditorMenu();
    return;
  }
  if (!["Escape", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(key)) return;
  event.preventDefault();
  if (key === "Escape") {
    closeEditorMenu();
    return;
  }
  const nodes = menuButtons();
  if (!nodes.length) return;
  const index = nodes.indexOf(document.activeElement);
  const step = key === "ArrowUp" || key === "ArrowLeft" ? -1 : 1;
  let next = (index + step + nodes.length) % nodes.length;
  if (key === "Home" || (index < 0 && step > 0)) next = 0;
  if (key === "End" || (index < 0 && step < 0)) next = nodes.length - 1;
  nodes[next].focus();
}, true);

document.addEventListener("pointerdown", event => {
  if (editorMenu && !menuPopup.contains(event.target) && !menuNames.some(name => $("menu-" + name).contains(event.target))) closeEditorMenu(false);
}, true);
window.addEventListener("resize", () => closeEditorMenu(false));
window.refreshEditorMenu = refreshEditorMenu;
window.openEditorMenu = openEditorMenu;
