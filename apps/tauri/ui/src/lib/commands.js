// The editor's one command list: every menu row, the right-click tool picker and the lane menu,
// and every command in the Rust catalogue (crates/dawsu-application/src/commands.rs,
// crates/dawsu-editor/src/editing_commands.rs and tools.rs). menus.js reads it when a menu opens
// and EditorMenu.svelte draws the rows. No imports: apps/tauri/tests reads it without node_modules,
// and a Rust test in dawsu-tauri checks its catalogue ids against the real catalogue both ways.
//
// A row's fields:
// - command: the catalogue id the row stands for. Its keys show as the shortcut hint (read from
//   editor_shortcuts each time a menu opens, so the key preset and the mapper's own keys apply),
//   and the row runs it unless `run` says otherwise. Keys are never written here.
// - run: ["click", id] presses an existing button; ["act", action] sends an editor action;
//   ["call", name] runs a menus.js function; ["toggle", kind, path, light] flips a view or scene
//   field (light: a quick update, as app.js's act(action, false)); ["panel", key, field] shows or
//   folds or opens a panel (browser, inspector, controller through app.js's togglePanel); ["tool", name]; ["lane", op] and ["onLane", op, id] act on the right-clicked lane (["onLane", "call", name] runs a ["call"] function).
// - enabled: a rule name in menus.js (default "edit": a map open, no job running, no drag).
// - checked: a rule name for a switch row; a toggle or panel row is checked by its own field.
// - radio: a tool picker row; close: a checked row that still closes the menu.
// - history: Undo (-1) and Redo (0) name the edit they would change.
// - choices: a row of segmented choices; menus.js fills the source when the menu opens.

// The menu bar, left to right; index.html has a #menu-<name> button for each (Layout… moves them).
export const menuBar = ["file", "edit", "playback", "map", "view"];

export const items = [
  // File
  { id: "openMap", label: "Open map…", icon: "FolderOpen", command: "open", run: ["call", "openMap"], enabled: "idle" },
  { id: "importOsz", label: "Import .osz…", icon: "PackageOpen", command: "importOsz", enabled: "idle" },
  { id: "openFolder", label: "Open map folder", icon: "Folder", command: "openFolder" },
  { id: "save", label: "Save", icon: "Save", command: "save" },
  { id: "revert", label: "Revert to saved…", icon: "RotateCcw", command: "revert" },
  { id: "export", label: "Export…", icon: "Package", run: ["click", "export-entry"], enabled: "document" },
  { id: "startScreen", label: "Start screen", icon: "House", run: ["call", "goHome"] },
  { id: "folders", label: "Folders…", icon: "Folder", run: ["call", "openFolders"], enabled: "idle" },

  // Edit
  { id: "undo", label: "Undo", icon: "Undo2", command: "undo", enabled: "undo", history: -1 },
  { id: "redo", label: "Redo", icon: "Redo2", command: "redo", enabled: "redo", history: 0 },
  { id: "cut", label: "Cut", icon: "Scissors", command: "cut", enabled: "selection" },
  { id: "copy", label: "Copy", icon: "Copy", command: "copy", enabled: "selection" },
  { id: "paste", label: "Paste at playhead", icon: "Clipboard", command: "paste" },
  { id: "duplicate", label: "Duplicate", icon: "CopyPlus", command: "duplicate", enabled: "selection" },
  { id: "delete", label: "Delete", icon: "Trash2", command: "delete", enabled: "selection" },
  { id: "selectAll", label: "Select all", icon: "SquareDashedMousePointer", command: "selectAll" },
  { id: "quantize", label: "Quantize…", icon: "AlignHorizontalJustifyCenter", command: "quantizeDialog", enabled: "selection" },
  { id: "repeat", label: "Repeat…", icon: "Repeat2", command: "repeat", enabled: "selection" },
  { id: "editRange", label: "Edit range…", icon: "SquareDashed", command: "editRange", enabled: "range" },
  { id: "scaleByVolume", label: "Scale by volume…", icon: "ChartNoAxesColumn", command: "scaleByVolume" },
  { id: "placeSoundSet", label: "Place sound set…", run: ["call", "placeSoundSet"] },
  { id: "saveSoundSet", label: "Save selection as sound set…", run: ["call", "saveSoundSet"], enabled: "selection" },
  { id: "soundSetPlacement", label: "Sound set placement…", run: ["call", "openSoundSetPlacement"], enabled: "placement" },
  { id: "protect", label: "Protect hitnormals from deletion", icon: "Shield", run: ["toggle", "protect", "scene.protect"] },
  { id: "audition", label: "Audition when placing hits", icon: "Ear", run: ["toggle", "audition", "view.audition"] },

  // Playback
  { id: "play", label: "Play / pause", icon: "Play", command: "play", run: ["act", { kind: "play" }] },
  { id: "stop", label: "Stop", icon: "Square", command: "stop", run: ["act", { kind: "stop" }] },
  { id: "goToStart", label: "Go to start", icon: "SkipBack", command: "start" },
  { id: "goToEnd", label: "Go to end", icon: "SkipForward", run: ["call", "goToEnd"] },
  { id: "playbackStart", label: "Start playing from", choices: "playbackStart" },
  { id: "returnOnStop", label: "Return on stop", icon: "UndoDot", run: ["toggle", "return_on_stop", "view.returnOnStop", true] },
  { id: "follow", label: "Follow playhead", icon: "LocateFixed", run: ["toggle", "follow", "view.follow", true] },
  { id: "cycle", label: "Cycle between L and R", icon: "Repeat", command: "cycle", run: ["toggle", "cycle", "view.cycle"] },
  { id: "speed", label: "Speed · pitch stays the same", choices: "speed" },
  { id: "muteSong", label: "Mute song", icon: "VolumeX", run: ["toggle", "song_muted", "view.songMuted", true] },
  { id: "audioOutput", label: "Audio output…", icon: "Volume2", run: ["click", "audio"], enabled: "idle" },

  // Map
  { id: "difficulty", label: "Difficulty", choices: "difficulty", close: true },
  { id: "newDifficulty", label: "New hitsound difficulty…", icon: "FilePlus", command: "newDiff", run: ["click", "new"] },
  { id: "generate", label: "Generate hitsound difficulty…", icon: "Package", command: "generate", run: ["click", "generate"] },
  { id: "copyHitsounds", label: "Copy hitsounds…", icon: "Copy", run: ["click", "copy"] },
  { id: "drumDetection", label: "Drum hit detection…", icon: "Drum", run: ["call", "openDrumWindow"] },
  { id: "nodeGraph", label: "Node graph…", icon: "Layers", run: ["call", "openNodeGraphWindow"] },
  { id: "exportStyle", label: "Generate export style", choices: "exportStyle", enabled: "never" },
  { id: "addBookmark", label: "Add bookmark", icon: "BookmarkPlus", command: "addBookmark" },
  { id: "previousBookmark", label: "Previous bookmark", icon: "ArrowLeftToLine", command: "previousBookmark" },
  { id: "nextBookmark", label: "Next bookmark", icon: "ArrowRightToLine", command: "nextBookmark" },
  { id: "addLane", label: "Add lane…", icon: "Plus", run: ["click", "add-lane"] },
  { id: "deleteActiveLane", label: "Delete active lane…", icon: "Trash2", run: ["call", "deleteActiveLane"], enabled: "deleteLane" },

  // View
  { id: "browser", label: "Browser", icon: "PanelLeft", run: ["panel", "browser"] },
  { id: "inspector", label: "Inspector", icon: "PanelRight", run: ["panel", "inspector"] },
  { id: "volumePanel", label: "Volume and Mixer", icon: "ChartNoAxesColumn", run: ["panel", "controller"] },
  { id: "mapPreview", label: "Map preview", icon: "ScanEye", run: ["call", "toggleMapPreview"], checked: "mapPreview", enabled: "document" },
  { id: "laneNames", label: "Lane names", icon: "Tag", run: ["panel", "lane_names", "laneNames"] },
  { id: "songWaveform", label: "Song waveform", icon: "AudioWaveform", run: ["click", "ruler-waveform"], checked: "waveform" },
  { id: "objectLines", label: "Object lines", icon: "AlignHorizontalSpaceAround", run: ["toggle", "object_lines", "view.objectLines"] },
  { id: "usedOnly", label: "Used lanes only", icon: "ListFilter", run: ["click", "used-only"], checked: "usedOnly" },
  { id: "collapseLayers", label: "Collapse sample layers", icon: "ListCollapse", run: ["click", "collapse-layers"] },
  { id: "filterLanes", label: "Filter lanes…", icon: "Search", command: "filter" },
  { id: "tallerLanes", label: "Taller lanes", icon: "ChevronsUpDown", command: "rowsIn" },
  { id: "shorterLanes", label: "Shorter lanes", icon: "ChevronsDownUp", command: "rowsOut" },
  { id: "zoomIn", label: "Zoom in", icon: "ZoomIn", command: "zoomIn" },
  { id: "zoomOut", label: "Zoom out", icon: "ZoomOut", command: "zoomOut" },
  { id: "fit", label: "Fit difficulty", icon: "MoveHorizontal", run: ["click", "fit"] },
  { id: "timeSignatures", label: "Time signatures…", icon: "Music", run: ["click", "time-signatures"] },
  { id: "drumMap", label: "Open drum map…", icon: "Drum", run: ["call", "requestDrumMap"] },
  { id: "editLayout", label: "Edit layout…", icon: "LayoutPanelLeft", run: ["call", "editLayout"] },
  { id: "viewSettings", label: "View settings…", icon: "Settings", run: ["call", "openViewSettings"] },
  { id: "shortcuts", label: "Shortcuts", icon: "Keyboard", command: "help", run: ["call", "showHelp"], enabled: "idle" },

  // Right-click on the timeline (and the fourth tool pill): the seven tools, in TOOLS' order (tools.js).
  { id: "toolSelect", label: "Select", icon: "MousePointer2", command: "toolSelect", run: ["tool", "Select"], radio: true, close: true, checked: "tool" },
  { id: "toolDraw", label: "Draw", icon: "Drumstick", command: "toolDraw", run: ["tool", "Draw"], radio: true, close: true, checked: "tool" },
  { id: "toolErase", label: "Erase", icon: "Eraser", command: "toolErase", run: ["tool", "Erase"], radio: true, close: true, checked: "tool" },
  { id: "toolMute", label: "Mute", icon: "VolumeX", command: "toolMute", run: ["tool", "Mute"], radio: true, close: true, checked: "tool" },
  { id: "toolGrab", label: "Grab", icon: "Hand", command: "toolGrab", run: ["tool", "Grab"], radio: true, close: true, checked: "tool" },
  { id: "toolRange", label: "Range", icon: "Scan", command: "toolRange", run: ["tool", "Range"], radio: true, close: true, checked: "tool" },
  { id: "toolRamp", label: "Ramp", icon: "TrendingUp", command: "toolRamp", run: ["tool", "Ramp"], radio: true, close: true, checked: "tool" },

  // Right-click, Menu key or Shift+F10 on a lane header: the lane is activated (without audition) first.
  { id: "laneRename", label: "Rename…", run: ["lane", "rename"], enabled: "laneRename" },
  { id: "laneMoveUp", label: "Move up", run: ["lane", "up"], enabled: "laneUp" },
  { id: "laneMoveDown", label: "Move down", run: ["lane", "down"], enabled: "laneDown" },
  { id: "laneReplaceSample", label: "Replace sample…", icon: "FolderOpen", run: ["onLane", "click", "replace-sample"], enabled: "laneReplace" },
  { id: "laneColour", label: "Lane colour…", run: ["onLane", "click", "lane-colour"], enabled: "laneColour" },
  { id: "laneAddLayer", label: "Add numbered layer", icon: "Layers", command: "addNumberedLayer", run: ["onLane", "layer"], enabled: "laneLayer" },
  { id: "laneAdd", label: "Add lane…", icon: "Plus", run: ["click", "add-lane"], enabled: "lane" },
  { id: "laneDelete", label: "Delete lane…", icon: "Trash2", run: ["onLane", "call", "deleteActiveLane"], enabled: "laneDelete" },
];

// Where each row sits. "-" is a divider; one at the start or end of a menu, or next to another, is
// left out (Map has no Difficulty row, and so no divider under it, without a sibling difficulty).
export const menus = {
  file: ["openMap", "importOsz", "openFolder", "-", "save", "revert", "export", "-", "startScreen", "folders"],
  edit: [
    "undo", "redo", "-",
    "cut", "copy", "paste", "duplicate", "delete", "selectAll", "-",
    "quantize", "repeat", "editRange", "scaleByVolume", "placeSoundSet", "saveSoundSet", "soundSetPlacement", "-",
    "protect", "audition",
  ],
  playback: [
    "play", "stop", "goToStart", "goToEnd", "-",
    "playbackStart", "-",
    "returnOnStop", "follow", "cycle", "-",
    "speed", "-",
    "muteSong", "audioOutput",
  ],
  map: [
    "difficulty", "-",
    "newDifficulty", "-",
    "generate", "copyHitsounds", "drumDetection", "nodeGraph", "exportStyle", "-",
    "addBookmark", "previousBookmark", "nextBookmark", "-",
    "addLane", "deleteActiveLane",
  ],
  view: [
    "browser", "inspector", "volumePanel", "mapPreview", "laneNames", "-",
    "songWaveform", "objectLines", "usedOnly", "collapseLayers", "filterLanes", "tallerLanes", "shorterLanes", "-",
    "zoomIn", "zoomOut", "fit", "-",
    "timeSignatures", "drumMap", "editLayout", "viewSettings", "-",
    "shortcuts",
  ],
  tool: ["toolSelect", "toolDraw", "toolErase", "toolMute", "toolGrab", "toolRange", "toolRamp"],
  lane: ["laneRename", "laneMoveUp", "laneMoveDown", "laneReplaceSample", "laneColour", "laneAddLayer", "laneAdd", "laneDelete"],
};

// Catalogue commands no menu row stands for, so no menu shows their keys. They run from their keys
// only; the shortcut editor lists them all under their Rust description.
export const keyOnly = [
  "quantize", "volumeUp", "volumeDown", "volumeUpFine", "volumeDownFine", "jumpToSelection",
  "nudgeLeft", "nudgeRight", "gridLeft", "gridRight", "navUp", "navDown", "selectPrevious", "selectNext",
  "addPrevious", "addNext", "navAddUp", "navAddDown",
  "openHitsoundFolder", "reviewOutput", "first", "last", "deselect", "snap",
  "jumpLeft", "jumpRight", "setLeft", "setRight", "locatorsToSelection", "playLocatorRange", "removeBookmark",
  "seekBack", "seekBackFar", "seekForward", "seekForwardFar",
  "snap1", "snap2", "snap3", "snap4", "snap5", "snap6", "snap7", "snap8", "snap9",
];
