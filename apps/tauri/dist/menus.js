"use strict";

const menuBar = $("editor-menus");
const menuPopup = $("editor-menu");
const menuNames = ["map", "edit", "view", "playback"];
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
  button.className = radio ? "menu-choice" : "menu-item";
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

function menuChoices(label, glyph, choices, current, choose, enabled) {
  const group = document.createElement("div");
  group.className = "menu-choice-group";
  group.setAttribute("role", "group");
  group.setAttribute("aria-label", label);
  const heading = document.createElement("div");
  heading.className = "menu-choice-label";
  heading.append(menuIcon(glyph), document.createTextNode(label));
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

function menuItems(name) {
  const click = (id) => () => $(id).click();
  const idle = () => !view.busy && !drag;
  const selection = () => canEditFromMenu() && !!scene.selection?.count;
  const panel = (label, glyph, key, field) => menuItem(label, glyph, () =>
    act({ kind: "panel", panel: key, on: !view.panels?.[field] }), {
    checked: () => !!view.panels?.[field],
  });
  const toggle = (label, glyph, kind, field) => menuItem(label, glyph, () =>
    act({ kind, on: !view[field] }, false), { checked: () => !!view[field] });
  if (name === "map") {
    return [
      menuItem("Open map…", "FolderOpen", click("open"), { shortcut: "open", enabled: idle }),
      menuItem("Folders…", "FolderOpen", click("folders"), { enabled: idle }),
      menuItem("Save", "Save", () => act({ kind: "review_save" }), { shortcut: "save" }),
      menuCommand("Revert…", "Undo2", "revert"),
      menuItem("New hitsound difficulty…", "Drumstick", click("new")),
      menuItem("Generate hitsound difficulty…", "TrendingUp", () => act({ kind: "generate" }), { shortcut: "generate" }),
      menuItem("Copy hitsounds…", "Redo2", click("copy")),
      menuChoices("Generate export style", "Scan", [["objects", "Per-object"]],
        () => "objects", () => {}, () => false),
    ];
  }
  if (name === "edit") {
    return [
      menuCommand("Undo", "Undo2", "undo", () => canEditFromMenu() && !$("undo").disabled),
      menuCommand("Redo", "Redo2", "redo", () => canEditFromMenu() && !$("redo").disabled),
      menuCommand("Cut", "Eraser", "cut", selection),
      menuCommand("Copy", "Redo2", "copy", selection),
      menuCommand("Paste at playhead", "Redo2", "paste"),
      menuCommand("Duplicate", "Redo2", "duplicate", selection),
      menuCommand("Delete", "Eraser", "delete", selection),
      menuCommand("Select all", "MousePointer2", "selectAll"),
      menuCommand("Quantize…", "Scan", "quantizeDialog", selection),
      menuCommand("Repeat…", "Redo2", "repeat", selection),
      menuCommand("Edit range…", "Scan", "editRange", () => canEditFromMenu() && !!scene.range),
      menuCommand("Scale by volume…", "TrendingUp", "scaleByVolume"),
    ];
  }
  if (name === "view") {
    return [
      menuCommand("Zoom in", "Scan", "zoomIn"),
      menuCommand("Zoom out", "Scan", "zoomOut"),
      menuItem("Fit difficulty", "Scan", click("fit")),
      menuItem("Collapse sample layers", "Scan", click("collapse-layers")),
      menuItem("Object lines", "Scan", () => act({ kind: "object_lines", on: !view.objectLines }),
        { checked: () => !!view.objectLines }),
      panel("Inspector", "Scan", "inspector", "inspector"),
      panel("Volume controller", "TrendingUp", "controller", "controller"),
      panel("Lane names", "Scan", "lane_names", "laneNames"),
      panel("File browser", "FolderOpen", "browser", "browser"),
      menuItem("Time signatures…", "Drumstick", click("time-signatures")),
      menuItem("View settings…", "Scan", click("panel-view")),
      menuItem("Shortcuts and help", "MousePointer2", click("help"), { shortcut: "help", enabled: idle }),
    ];
  }
  const speeds = [...$("speed").options].map(option => [Number(option.value), option.textContent]);
  return [
    menuItem("Play or pause", "Play", () => act({ kind: "play" }), { shortcut: "play" }),
    menuItem("Stop", "Square", () => act({ kind: "stop" }), { shortcut: "stop" }),
    menuChoices("Start playing from", "Play", playbackStartChoices, () => view.playbackStart ?? "cursor",
      value => act({ kind: "playback_start", value }, false)),
    toggle("Return on stop", "Undo2", "return_on_stop", "returnOnStop"),
    toggle("Follow playback", "Scan", "follow", "follow"),
    menuChoices("Speed", "TrendingUp", speeds, () => view.speed, value => act({ kind: "speed", value })),
    toggle("Mute song", "VolumeX", "song_muted", "songMuted"),
    menuItem("Audio settings…", "TrendingUp", click("audio"), { enabled: idle }),
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

function openEditorMenu(name, last = false) {
  closeEditorMenu(false);
  const trigger = $("menu-" + name);
  const menu = { name, trigger, items: [] };
  editorMenu = menu;
  menuPopup.replaceChildren(...menuItems(name));
  menuPopup.setAttribute("aria-labelledby", trigger.id);
  menuPopup.hidden = false;
  trigger.setAttribute("aria-expanded", "true");
  const bounds = trigger.getBoundingClientRect();
  menuPopup.style.left = Math.max(4, Math.min(bounds.left, window.innerWidth - menuPopup.offsetWidth - 4)) + "px";
  menuPopup.style.top = bounds.bottom + "px";
  menuPopup.style.maxHeight = Math.max(0, window.innerHeight - bounds.bottom - 4) + "px";
  refreshEditorMenu();
  const items = menu.items.filter(item => !item.button.disabled);
  (last ? items.at(-1) : items[0])?.button.focus();

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
  if (editorMenu && !menuPopup.contains(event.target) && !menuBar.contains(event.target)) closeEditorMenu(false);
}, true);
window.addEventListener("resize", () => closeEditorMenu(false));
window.refreshEditorMenu = refreshEditorMenu;
