"use strict";

const menuPopup = $("editor-menu");
const menuNames = ["file", "edit", "playback", "map", "view"];
let editorMenu = null;

function canEditFromMenu() {
  return !!scene && !view.busy && !drag;
}

function menuIcon(glyph) {
  const image = document.createElement("img");
  image.className = "button-icon";
  image.src = "studio-assets/icons/" + glyph + ".svg";
  image.alt = "";
  return image;
}

function menuItem(label, glyph, execute, {
  shortcut, enabled = canEditFromMenu, checked, radio = false,
} = {}) {
  const button = makeButton(label, () => {
    if (!enabled()) return;
    if (!checked) closeEditorMenu();
    execute();
    refreshEditorMenu();
  });
  button.tabIndex = -1;
  button.className = radio ? "menu-choice" : glyph ? "menu-item" : "menu-item menu-item-plain";
  button.setAttribute("role", radio ? "menuitemradio" : checked ? "menuitemcheckbox" : "menuitem");
  if (glyph) button.prepend(menuIcon(glyph));
  const chip = shortcut ? document.createElement("kbd") : null;
  if (chip) {
    chip.hidden = true;
    button.append(chip);
  }
  if (checked && !radio) {
    const indicator = document.createElement("span");
    indicator.className = "menu-switch";
    indicator.setAttribute("aria-hidden", "true");
    button.append(indicator);
  }
  editorMenu.items.push({ button, enabled, checked, shortcut, chip });
  return button;
}

function menuCommand(label, glyph, id, enabled) {
  return menuItem(label, glyph, () => command(id), { shortcut: id, enabled });
}

function menuChoices(label, choices, current, choose, enabled) {
  const group = document.createElement("div");
  group.className = "menu-choice-group";
  group.setAttribute("role", "group");
  group.setAttribute("aria-label", label);
  const heading = document.createElement("div");
  heading.className = "menu-choice-label";
  heading.append(document.createTextNode(label));
  const row = document.createElement("div");
  row.className = "menu-choices";
  for (const [value, text] of choices) {
    row.append(menuItem(text, null, () => choose(value), {
      enabled, radio: true, checked: () => current() === value,
    }));
  }
  group.append(heading, row);
  return group;
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

function menuItems(name, target) {
  const click = (id) => () => $(id).click();
  const idle = () => !view.busy && !drag;
  const selection = () => canEditFromMenu() && !!scene.selection?.count;
  const panel = (label, glyph, key, field) => menuItem(label, glyph, () =>
    act({ kind: "panel", panel: key, on: !view.panels?.[field] }), {
    checked: () => !!view.panels?.[field],
  });
  const toggle = (label, glyph, kind, field) => menuItem(label, glyph, () =>
    act({ kind, on: !view[field] }, false), { checked: () => !!view[field] });
  if (name === "lane") {
    const allowed = predicate => () => {
      const row = laneMenuRow(target);
      return row !== -1 && predicate(scene.lanes[row], row);
    };
    const onLane = (label, glyph, action, predicate) =>
      menuItem(label, glyph, () => laneMenuAction(target, action), { enabled: allowed(predicate) });
    return [
      menuItem("Rename…", null, () => renameLane(laneMenuRow(target)),
        { enabled: allowed(lane => !!lane.colourKey && !view.drumView?.enabled) }),
      menuItem("Move up", null, () => moveLane(laneMenuRow(target), -1),
        { enabled: allowed((_, row) => adjacentLaneForMove(row, -1) !== null) }),
      menuItem("Move down", null, () => moveLane(laneMenuRow(target), 1),
        { enabled: allowed((_, row) => adjacentLaneForMove(row, 1) !== null) }),
      onLane("Replace sample…", "FolderOpen", click("replace-sample"), lane => lane.canReplace),
      onLane("Lane colour…", null, click("lane-colour"), lane => lane.colourKey),
      onLane("Add numbered layer", "Drumstick", row => act({ kind: "lane", row, operation: "add_layer" }), lane => lane.canAddLayer),
      menuItem("Add lane…", null, click("add-lane"), { enabled: allowed(() => true) }),
      onLane("Delete lane…", "Eraser", click("delete-lane"), lane => lane.canDelete),
    ];
  }
  if (name === "tool") {
    return Object.entries(toolIcons).map(([tool, glyph]) =>
      menuItem(tool, glyph, () => {
        closeEditorMenu();
        act({ kind: "tool", name: tool });
      }, {
        shortcut: "tool" + tool,
        radio: true,
        checked: () => view.tool === tool,
      }));
  }
  const separator = () => {
    const line = document.createElement("div");
    line.className = "menu-separator";
    line.setAttribute("role", "separator");
    return line;
  };
  const flag = (label, glyph, kind, on) => menuItem(label, glyph, () => act({ kind, on: !on() }),
    { checked: () => !!on() });
  const control = (label, glyph, id, on) => menuItem(label, glyph, click(id), { checked: on });
  const step = offset => scene?.history?.[scene.historyPosition + offset];
  const named = (label, action) => action ? label + " · " + action : label;
  if (name === "file") {
    return [
      menuItem("Open map…", "FolderOpen", click("open"), { shortcut: "open", enabled: idle }),
      menuCommand("Import .osz…", "FolderOpen", "importOsz", idle),
      menuCommand("Open map folder", "FolderOpen", "openFolder"),
      separator(),
      menuCommand("Save", "Save", "save"),
      menuCommand("Revert to saved…", "Undo2", "revert"),
      separator(),
      menuItem("Start screen", "House", click("go-home")),
      menuItem("Folders…", "Settings", click("folders"), { enabled: idle }),
    ];
  }
  if (name === "map") {
    const difficulties = scene?.path ? Array.from($("difficulty").options ?? [], option => [option.value, option.text]) : [];
    return [
      ...difficulties.length ? [menuChoices("Difficulty", difficulties, () => scene?.path, path => {
        closeEditorMenu();
        openSiblingDifficulty(path);
      }), separator()] : [],
      menuItem("New hitsound difficulty…", "Drumstick", click("new"), { shortcut: "newDiff" }),
      separator(),
      menuItem("Generate hitsound difficulty…", "TrendingUp", click("generate"), { shortcut: "generate" }),
      menuItem("Copy hitsounds…", "Redo2", click("copy")),
      menuChoices("Generate export style", [["objects", "Per-object"]],
        () => "objects", () => {}, () => false),
      separator(),
      menuCommand("Add bookmark", null, "addBookmark"),
      menuCommand("Previous bookmark", null, "previousBookmark"),
      menuCommand("Next bookmark", null, "nextBookmark"),
      separator(),
      menuItem("Add lane…", "Drumstick", click("add-lane")),
      menuItem("Delete active lane…", "Eraser", click("delete-lane"),
        { enabled: () => canEditFromMenu() && !$("delete-lane").disabled }),
    ];
  }
  if (name === "edit") {
    return [
      menuCommand(named("Undo", step(-1)), "Undo2", "undo", () => canEditFromMenu() && !$("undo").disabled),
      menuCommand(named("Redo", step(0)), "Redo2", "redo", () => canEditFromMenu() && !$("redo").disabled),
      separator(),
      menuCommand("Cut", "Eraser", "cut", selection),
      menuCommand("Copy", "Redo2", "copy", selection),
      menuCommand("Paste at playhead", "Redo2", "paste"),
      menuCommand("Duplicate", "Redo2", "duplicate", selection),
      menuCommand("Delete", "Eraser", "delete", selection),
      menuCommand("Select all", "MousePointer2", "selectAll"),
      separator(),
      menuCommand("Quantize…", "Scan", "quantizeDialog", selection),
      menuCommand("Repeat…", "Redo2", "repeat", selection),
      menuCommand("Edit range…", "Scan", "editRange", () => canEditFromMenu() && !!scene.range),
      menuCommand("Scale by volume…", "TrendingUp", "scaleByVolume"),
      separator(),
      flag("Protect hitnormals from deletion", null, "protect", () => scene?.protect),
      flag("Audition when placing hits", null, "audition", () => view.audition),
    ];
  }
  if (name === "view") {
    return [
      panel("Browser", "PanelLeft", "browser", "browser"),
      panel("Inspector", "PanelRight", "inspector", "inspector"),
      panel("Volume panel", "TrendingUp", "controller", "controller"),
      panel("Lane names", "Scan", "lane_names", "laneNames"),
      separator(),
      control("Song waveform", null, "ruler-waveform", () => $("ruler-waveform").getAttribute("aria-pressed") === "true"),
      menuItem("Object lines", "Scan", () => act({ kind: "object_lines", on: !view.objectLines }),
        { checked: () => !!view.objectLines }),
      control("Used lanes only", null, "used-only", () => !!$("used-only").checked),
      menuItem("Collapse sample layers", "Scan", click("collapse-layers")),
      separator(),
      menuCommand("Zoom in", "Scan", "zoomIn"),
      menuCommand("Zoom out", "Scan", "zoomOut"),
      menuItem("Fit difficulty", "Scan", click("fit")),
      separator(),
      menuItem("Time signatures…", "Drumstick", click("time-signatures")),
      menuItem("Open drum map…", "Drumstick", () => requestDrumMap()),
      menuItem("Edit layout…", "Scan", () => dock?.editLayout()),
      menuItem("View settings…", "Settings", click("panel-view")),
      separator(),
      menuItem("Shortcuts", "Info", click("help"), { shortcut: "help", enabled: idle }),
    ];
  }
  if (name !== "playback") return [];
  const speeds = speedButtons.map(button =>
    [Number(button.dataset.playbackSpeed), button.textContent.replace("%", "")]);
  return [
    menuItem("Play / pause", "Play", () => act({ kind: "play" }), { shortcut: "play" }),
    menuItem("Stop", "Square", () => act({ kind: "stop" }), { shortcut: "stop" }),
    menuCommand("Go to start", "SkipBack", "start"),
    separator(),
    menuChoices("Start playing from", playbackStartChoices, () => view.playbackStart ?? "cursor",
      value => act({ kind: "playback_start", value }, false)),
    separator(),
    toggle("Return on stop", "Undo2", "return_on_stop", "returnOnStop"),
    toggle("Follow playhead", "Scan", "follow", "follow"),
    flag("Cycle between L and R", null, "cycle", () => view.cycle),
    separator(),
    menuChoices("Speed · pitch stays the same", speeds, () => view.speed, value => act({ kind: "speed", value })),
    separator(),
    toggle("Mute song", "VolumeX", "song_muted", "songMuted"),
    menuItem("Audio output…", "Settings", click("audio"), { enabled: idle }),
  ];
}

function closeEditorMenu(restore = true) {
  if (!editorMenu) return;
  const { trigger } = editorMenu;
  editorMenu = null;
  menuPopup.hidden = true;
  trigger.setAttribute("aria-expanded", "false");
  if (restore) trigger.focus();
}

function refreshEditorMenu() {
  if (!editorMenu) return;
  if (dialog.open) {
    closeEditorMenu(false);
    return;
  }
  for (const { button, enabled, checked } of editorMenu.items) {
    button.disabled = !enabled();
    if (checked) button.setAttribute("aria-checked", String(checked()));
  }
}

function openEditorMenu(name, last = false, point = null) {
  closeEditorMenu(false);
  const trigger = point?.trigger ?? $("menu-" + name);
  const menu = { name, trigger, items: [] };
  editorMenu = menu;
  menuPopup.replaceChildren(...menuItems(name, point?.target));
  if (point) {
    menuPopup.removeAttribute("aria-labelledby");
    menuPopup.setAttribute("aria-label", point.label);
  } else {
    menuPopup.removeAttribute("aria-label");
    menuPopup.setAttribute("aria-labelledby", trigger.id);
  }
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
  refreshEditorMenu();
  const items = menu.items.filter(item => !item.button.disabled);
  (last ? items[items.length - 1] : items[0])?.button.focus();

  invoke("editor_shortcuts").then(({ entries }) => {
    if (editorMenu !== menu) return;
    const bindings = new Map(entries.map(([id, , keys]) => [id, keys]));
    for (const { shortcut, chip } of menu.items) {
      if (!chip) continue;
      chip.textContent = bindings.get(shortcut)?.join(" / ") ?? "";
      chip.hidden = !chip.textContent;
    }
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
  const nodes = editorMenu.items.map(item => item.button).filter(button => !button.disabled);
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
