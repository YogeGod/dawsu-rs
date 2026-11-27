// Every feature of dawsu! (the editor in apps/tauri + crates), what it should do, where that rule
// comes from, and how the robot mapper checks it.
//
// IDs F001-F088 are FEATURE-COVERAGE.md's rows; N001+ are the features added since that ledger's last
// full check (NOW.md, the code and the Nov 24-27 audits). "truth" is the source a check is judged
// against: osu!lazer at ppy/osu@131fe72 for osu! behaviour (file:line), otherwise the project rule or
// ADR. "manual" says why a robot cannot check it (hardware, the osu! client, listening, network).
//
// Robot steps name the ids they check with `covers`; `node tools/feature-check.mjs --features`
// prints this list with the steps that cover each row.

const L = "osu!lazer@131fe72";
export const FEATURES = [
  // Map files
  { id: "F001", area: "Map files", feature: "Open a .osu; drop a .osu or .osz", intended: "The difficulty opens with one lane per sound it plays and one hit per sound moment; the lanes and hits match what osu! plays from the file.", truth: `${L} ConvertHitObjectParser.cs:543-569, LegacyDecoder.cs:192-203` },
  { id: "F002", area: "Map files", feature: "Recent maps", intended: "An opened map is listed on the start screen; the practice mapset never is.", truth: "dawsu! rule (ADR 0066 start screen)" },
  { id: "F003", area: "Map files", feature: "Import .osz", intended: "The archive unpacks into the Songs folder and opens; nothing else is written.", truth: "dawsu! rule", manual: "native file picker; covered by the every-input robot's picker answers" },
  { id: "F004", area: "Map files", feature: "New hitsound difficulty", intended: "Creates one new .osu in the mapset with no objects, Version [Hitsounds], BeatmapID 0 and the source's timing lines, then opens it.", truth: "crates/dawsu-hitsounding/src/generate.rs:161-185" },
  { id: "F005", area: "Map files", feature: "Revert to last saved", intended: "After a confirmation, the difficulty reloads from disk; unsaved hits are gone, saved ones back.", truth: "dawsu! rule" },
  { id: "F006", area: "Map files", feature: "Open map / hitsound / project folder", intended: "Opens the folder in the system file manager.", truth: "dawsu! rule", manual: "opens Explorer; the robot cannot read Explorer's window" },
  { id: "F007", area: "Map files", feature: "Sample files added to the folder show up as lanes", intended: "A new .wav/.ogg/.mp3 in the mapset is offered by Add lane and becomes a lane.", truth: "dawsu! rule" },
  { id: "F008", area: "Map files", feature: "Difficulty list (switch, asks about unsaved edits)", intended: "Switching difficulty with unsaved edits asks first; Cancel keeps the edits.", truth: "dawsu! rule" },
  // osu! connection
  { id: "F009", area: "osu! connection", feature: "Open map from osu!", intended: "Opens the map osu!stable has open, or the one lazer exported with Edit externally.", truth: "dawsu! rule", manual: "needs osu! running" },
  { id: "F010", area: "osu! connection", feature: "Live reload when osu!'s editor saves", intended: "A change on disk reloads the map and keeps unsaved hitsound edits.", truth: "dawsu! rule", manual: "needs osu!'s editor; external change is checked in Rust tests" },
  { id: "F011", area: "osu! connection", feature: "Reload osu!'s editor after Save", intended: "osu! reloads the saved difficulty.", truth: "dawsu! rule", manual: "needs osu! running" },
  // Lanes
  { id: "F012", area: "Lanes", feature: "Sample lanes for hitnormal, whistle, finish, clap; default sound first", intended: "Each set shows hitnormal first, then whistle, finish, clap; every object plays a hitnormal.", truth: `${L} ConvertHitObjectParser.cs:547-566` },
  { id: "F013", area: "Lanes", feature: "Sample-set folders (Normal, Soft, Drum), M and S, reorderable", intended: "Lanes group by set; a folder's M mutes and S solos all its lanes.", truth: `${L} ConvertHitObjectParser.cs:197-238 (three banks)` },
  { id: "F014", area: "Lanes", feature: "Numbered layers (+ on a lane)", intended: "+ adds the next custom index of that sound; it saves as that index (file name with the number).", truth: `${L} ConvertHitObjectParser.cs:639-646, HitSampleInfo.cs:89-99` },
  { id: "F015", area: "Lanes", feature: "Add, delete, filter lanes; hide lane names", intended: "Add lane adds a lane, Delete removes it (asks when it has hits), Filter shows only matching lanes.", truth: "dawsu! rule" },
  { id: "F016", area: "Lanes", feature: "Active lane and sample replacement", intended: "Replace sample swaps the lane's file; the map's hits stay.", truth: "dawsu! rule", manual: "native file picker" },
  { id: "F017", area: "Lanes", feature: "Mute, solo, audition per lane; Song row", intended: "M and S change only what plays in dawsu!, never the saved file.", truth: "dawsu! rule" },
  { id: "F018", area: "Lanes", feature: "Collapse sample layers", intended: "Numbered layers fold into their base lane row and unfold again; hits are unchanged.", truth: "dawsu! rule" },
  { id: "F019", area: "Lanes", feature: "Used lanes only", intended: "Only lanes with hits are shown; turning it off shows the rest again.", truth: "dawsu! rule" },
  // Hit editing
  { id: "F020", area: "Hit editing", feature: "Tools: Select, Grab, Draw, Erase, Mute, Ramp, Range", intended: "Draw adds a hit at the snapped time, Erase removes one, Mute silences one in dawsu! only, Range selects a time range, Ramp shapes volumes.", truth: "dawsu! rule (Cubase tools)" },
  { id: "F021", area: "Hit editing", feature: "Cut, copy, paste at playhead, duplicate, delete, select all", intended: "Copy+Paste puts the copied hits at the playhead keeping their spacing; Duplicate places them right after; Select all selects every hit.", truth: "dawsu! rule" },
  { id: "F022", area: "Hit editing", feature: "Quantize, repeat, edit range, scale by volume", intended: "Quantize moves selected hits onto the snap grid; Repeat copies them n times; Scale by volume changes volumes and never goes under 5%.", truth: `${L} ControlPointInfo.cs:169 (closest snapped time); project rule (5%)` },
  { id: "F023", area: "Hit editing", feature: "Insert volume for new hits", intended: "A new hit takes the Insert volume; the field refuses values under 5%.", truth: "project rule (5%)" },
  { id: "F024", area: "Hit editing", feature: "Protect hitnormals from deletion", intended: "With protect on, Delete leaves hitnormals on objects in place.", truth: `${L} ConvertHitObjectParser.cs:547-553 (an object always plays a hitnormal)` },
  { id: "F025", area: "Hit editing", feature: "Slider body sounds", intended: "Slider slide and whistle body sounds save into the slider's own fields.", truth: `${L} HitObject.cs:205-217` },
  { id: "F026", area: "Hit editing", feature: "Undo, redo, History panel", intended: "Each edit is one undo step; Undo and Redo restore the exact hits; History lists them.", truth: "dawsu! rule" },
  { id: "F027", area: "Hit editing", feature: "Drum hit detection", intended: "Finds kick and snare onsets on the beat grid and offers them as hits.", truth: "dawsu! rule", manual: "needs a real song and listening" },
  { id: "F028", area: "Hit editing", feature: "Live: play lanes from the keyboard or MIDI", intended: "Keys A–L (or MIDI notes) play their lane.", truth: "3.1.4 parity", manual: "needs listening or a MIDI device" },
  { id: "F029", area: "Hit editing", feature: "Record taps from the playhead", intended: "Taps while playing become hits, as one undo step.", truth: "3.1.4 parity", manual: "needs playback" },
  // Timeline navigation
  { id: "F030", area: "Timeline", feature: "Snap on/off; Grid, Grid relative, Objects", intended: "A click lands on the closest grid line of its red line's section (or on an object in Objects mode).", truth: `${L} ControlPointInfo.cs:169-186; crates/dawsu-editor/src/tempo.rs:198-209` },
  { id: "F031", area: "Timeline", feature: "Beat divisions 1/n and 2/1-4/1", intended: "The snap slider's stops 1,2,3,4,6,8,12,16 set the grid spacing to a beat divided by n.", truth: `${L} BindableBeatDivisor.cs:17` },
  { id: "F032", area: "Timeline", feature: "Zoom, Fit and follow playback", intended: "Zoom changes px per ms; Fit shows the whole song from 0.", truth: "dawsu! rule" },
  { id: "F033", area: "Timeline", feature: "Overview strip", intended: "Shows the whole difficulty; a click moves the view there.", truth: "dawsu! rule" },
  { id: "F034", area: "Timeline", feature: "Bookmarks with labels", intended: "Add bookmark at the playhead; Next/Previous move the playhead to it.", truth: `${L} LegacyBeatmapDecoder.cs:317-318 ([Editor] Bookmarks)` },
  { id: "F035", area: "Timeline", feature: "L/R locators and cycle", intended: "Locators to selection sets L and R to the selection's first and last time.", truth: "dawsu! rule (Cubase)" },
  { id: "F036", area: "Timeline", feature: "Time signatures (editor-only)", intended: "Changes bar lines in dawsu! only; the .osu is unchanged.", truth: `${L} LegacyBeatmapDecoder.cs:496-498 (the map's meter stays)` },
  { id: "F037", area: "Timeline", feature: "Object lines and warning markers", intended: "Lines at every object; a warning where a hit has no object.", truth: "dawsu! rule" },
  { id: "F038", area: "Timeline", feature: "Z/V jump to the first and last object", intended: "Go to start puts the playhead at 0; Go to end at the end.", truth: "dawsu! rule" },
  { id: "F039", area: "Timeline", feature: "Snap colouring", intended: "Hits are coloured by their beat division, as lazer colours ticks.", truth: `${L} BindableBeatDivisor.cs:104 (GetColourFor)` },
  { id: "F040", area: "Timeline", feature: "Position display", intended: "Shows time, bar.beat.tick, tempo, signature and snap of the playhead.", truth: "dawsu! rule" },
  { id: "F041", area: "Timeline", feature: "Show/hide Inspector, Lane info, Volume, Drums view", intended: "Each panel folds and shows again.", truth: "dawsu! rule" },
  { id: "F042", area: "Timeline", feature: "Map preview window", intended: "Draws circles, sliders, spinners with the chosen skin and lazer's stacking.", truth: `${L} OsuBeatmapProcessor.cs:63-69 (stacking)` },
  // Volume
  { id: "F043", area: "Volume", feature: "Volume controller: one bar per circle or slider end", intended: "One bar per sound moment; dragging changes that moment's volume.", truth: "dawsu! rule" },
  { id: "F044", area: "Volume", feature: "Drag, Alt-paint, Shift-ramp, Ramp tool", intended: "Every way of changing volume stops at 5%.", truth: "project rule (5%), PR #21" },
  { id: "F045", area: "Volume", feature: "Ramp shapes", intended: "A ramp sets volumes rising or falling across the selection by the chosen shape.", truth: "dawsu! rule" },
  { id: "F046", area: "Volume", feature: "Volume colours on hits", intended: "Quiet hits blue, loud ones red.", truth: "dawsu! rule", manual: "colour judgement; the gate checks the canvas draws" },
  { id: "F047", area: "Volume", feature: "Selection card: relative or set-all volume, start time", intended: "Set-all gives every selected hit the typed volume; typing under 5% gives 5%.", truth: "project rule (5%)" },
  // Map output
  { id: "F048", area: "Map output", feature: "Save .osu with a backup outside the mapset", intended: "Only the edited objects' lines change; the saved file plays exactly what the editor showed; no file is left in the mapset that the map does not play.", truth: `${L} ConvertHitObjectParser.cs:543-569; project rule (no stray files)` },
  { id: "F049", area: "Map output", feature: "Volume floor on save", intended: "Every changed hit is written at 5% or more.", truth: `project rule; ${L} DrawableHitObject.cs:186 (lazer plays 5% minimum)` },
  { id: "F050", area: "Map output", feature: "HS difficulty mode", intended: "In a [Hitsounds] difficulty each hit time becomes one circle carrying its sounds; reopening shows the same hits.", truth: "FEATURE-COVERAGE F050; ADR 0067" },
  { id: "F051", area: "Map output", feature: "Mixed sample indexes and sets on one object", intended: "A sound a circle's line cannot hold is written as a mixed sample file the map plays.", truth: `${L} ConvertHitObjectParser.cs:197-238 (one bank pair and index per object)` },
  { id: "F052", area: "Map output", feature: "Save problems, warning badge, Next problem", intended: "Problems are listed before Save writes.", truth: "dawsu! rule" },
  { id: "F053", area: "Map output", feature: "Generate hitsound diff, with review", intended: "The generated [Hitsounds] difficulty plays the same sounds at the same times as the source.", truth: "dawsu! rule; lazer rules for 'same sounds'" },
  { id: "F054", area: "Map output", feature: "Export settings: merge rules and presets", intended: "Merge settings change how mixes are written; Export window lists the files.", truth: "dawsu! rule" },
  { id: "F055", area: "Map output", feature: "Copy hitsounds to other difficulties", intended: "A target difficulty keeps its objects and positions and plays the source's sounds where their times match.", truth: "dawsu! rule (3.1.4 Copy)" },
  // Sound sets
  { id: "F056", area: "Sound sets", feature: "Save and apply sound sets", intended: "A saved set placed at the playhead adds the same hits there.", truth: "dawsu! rule" },
  { id: "F057", area: "Sound sets", feature: "Role-preserving snapshots", intended: "A set keeps which lane each sound came from.", truth: "dawsu! rule", manual: "covered by Rust tests" },
  { id: "F058", area: "Sound sets", feature: "Sound sets panel", intended: "Lists saved sets.", truth: "dawsu! rule" },
  // Drums
  { id: "F059", area: "Drums", feature: "Drum Map Setup, .dawsudrum presets", intended: "A preset maps drum notes to lanes.", truth: "drum-map-presets/README.md" },
  { id: "F060", area: "Drums", feature: "Map lanes from the drum instrument", intended: "Note names become lanes.", truth: "dawsu! rule", manual: "needs a VST3 drum instrument" },
  { id: "F061", area: "Drums", feature: "Drum instrument: VST3 or SoundFont", intended: "Loads and plays the instrument.", truth: "dawsu! rule", manual: "needs a VST3 or .sf2 and listening" },
  { id: "F062", area: "Drums", feature: "Live Drums VST playback", intended: "Hits play through the VST while playing.", truth: "dawsu! rule", manual: "listening" },
  { id: "F063", area: "Drums", feature: "Render drums to WAV", intended: "Renders once per input (hash-checked cache) into app data, never into the mapset.", truth: "improvements-audit 2026-11-24", manual: "needs a VST3" },
  // Playback
  { id: "F064", area: "Playback", feature: "Transport: play, stop, go to start, jump to L/R", intended: "Play moves the playhead forward; Stop stops it; Go to start puts it at 0.", truth: "dawsu! rule" },
  { id: "F065", area: "Playback", feature: "Speed 25-150% without pitch change", intended: "The speed changes only playback.", truth: "dawsu! rule" },
  { id: "F066", area: "Playback", feature: "Return on stop, start from", intended: "With Return on stop, Stop puts the playhead back where Play started.", truth: "dawsu! rule (Cubase)" },
  { id: "F067", area: "Playback", feature: "Song volume, mute, offset (not saved into the map)", intended: "None of these change the .osu.", truth: "dawsu! rule" },
  { id: "F068", area: "Playback", feature: "Global volume (never below 20%)", intended: "The main level never goes under 20%.", truth: "dawsu! rule" },
  { id: "F069", area: "Playback", feature: "Song waveform by frequency", intended: "The waveform draws once the song is decoded.", truth: "dawsu! rule" },
  // Window and settings
  { id: "F070", area: "Window", feature: "Start screen", intended: "Shows Open map, practice mapset, Ready checklist; the house button returns to it.", truth: "ADR 0066" },
  { id: "F071", area: "Window", feature: "Ribbon groups", intended: "Labels shrink in narrow windows; Save keeps its label.", truth: "dawsu! rule", manual: "layout judgement (screenshots)" },
  { id: "F072", area: "Window", feature: "Menus show icons, shortcuts, switches", intended: "Each row shows its shortcut from the active key preset.", truth: "apps/tauri/ui/src/lib/commands.js" },
  { id: "F073", area: "Window", feature: "Top bar shows the .osu name", intended: "The open file's name shows; its path on hover.", truth: "dawsu! rule" },
  { id: "F074", area: "Shortcuts", feature: "Key presets: DAW-like or osu!", intended: "The chosen preset changes the keys menus show and run.", truth: "dawsu! rule" },
  { id: "F075", area: "Shortcuts", feature: "First-launch preset choice", intended: "A fresh profile asks once.", truth: "dawsu! rule" },
  { id: "F076", area: "Shortcuts", feature: "Shortcut customisation, .dawsukeys", intended: "A changed key runs the command; reset restores the preset.", truth: "dawsu! rule" },
  { id: "F077", area: "Preferences", feature: "Songs folder", intended: "Chosen folder is used for Open from osu! and the library.", truth: "dawsu! rule", manual: "native folder picker" },
  { id: "F078", area: "Preferences", feature: "Audio output and buffer", intended: "The chosen device plays.", truth: "vault decision Nov 21 (WASAPI)", manual: "listening" },
  { id: "F079", area: "Preferences", feature: "File browser style", intended: "Classic or folder tree.", truth: "dawsu! rule" },
  { id: "F080", area: "Preferences", feature: "Colour schemes", intended: "Five schemes, Slate default; the choice persists after restart.", truth: "dawsu! rule" },
  { id: "F081", area: "Preferences", feature: "preferences.json read on first start", intended: "An existing file is read.", truth: "dawsu! rule", manual: "Rust tests" },
  { id: "F082", area: "Help", feature: "F1 list of every shortcut and mouse action", intended: "Lists every command with its keys; search filters it.", truth: "dawsu! rule" },
  { id: "F083", area: "Help", feature: "Quick start and full tour", intended: "The tour opens and can be finished or skipped.", truth: "dawsu! rule" },
  { id: "F084", area: "Help", feature: "Check for updates", intended: "Asks the release server; never installs without asking.", truth: "dawsu! rule", manual: "network and installer" },
  { id: "F085", area: "Data", feature: "Editor project files", intended: "Lane layout, hits with no object, HS mode, locators and settings survive a reopen; project files live in app data, never in the mapset.", truth: "dawsu! rule; project rule (no stray files)" },
  { id: "F086", area: "Data", feature: ".dawsukeys files", intended: "Export then import gives the same keys.", truth: "dawsu! rule", manual: "native file picker" },
  { id: "F087", area: "Data", feature: ".dawsuks keysound projects", intended: "Save and reopen keeps tracks and notes.", truth: "dawsu! rule", manual: "native file picker" },
  { id: "F088", area: "Data", feature: ".dawsudrum, drum-maps/, vst-renders/ in app data", intended: "Never written into the mapset.", truth: "dawsu! rule" },
  // Added since the ledger's last full check
  { id: "N001", area: "Since v1.1", feature: "Bundled practice mapset", intended: "Opens a practice copy outside Songs; never listed in recent maps.", truth: "NOW.md" },
  { id: "N002", area: "Since v1.1", feature: "Ready checklist on the start screen", intended: "Each item turns done live when set.", truth: "NOW.md" },
  { id: "N003", area: "Since v1.1", feature: "Help menu (tour, shortcuts, wiki, FAQ, bug report, crash logs)", intended: "Every row is present and enabled.", truth: "commands.js:126-131" },
  { id: "N004", area: "Since v1.1", feature: "Export review .osu diff", intended: "The Export window shows the lines Save would change.", truth: "NOW.md" },
  { id: "N005", area: "Since v1.1", feature: "Keysounding piano roll on the shared ruler", intended: "Notes snap to the editor's snap, draw/erase/undo work, hits are untouched.", truth: "FEATURE-COVERAGE keysounding section" },
  { id: "N006", area: "Since v1.1", feature: "Skinned Song · objects row", intended: "Draws the skin picked for the map preview.", truth: "NOW.md" },
  { id: "N007", area: "Since v1.1", feature: "Mixer: lane faders, mute, solo", intended: "A fader changes the lane's playback gain only; the saved .osu does not change.", truth: "dawsu! rule (mixer is listening only)" },
  { id: "N008", area: "Since v1.1", feature: "Real-time sound engine", intended: "Plays the song and hits in time.", truth: "dawsu! rule", manual: "listening" },
  { id: "N009", area: "Since v1.1", feature: "VST3 host and plugin windows", intended: "Loads a plugin, keeps its state for the session.", truth: "improvements-audit 2026-11-24", manual: "needs a VST3" },
  { id: "N010", area: "Since v1.1", feature: "SoundFont (.sf2) and SFZ instruments", intended: "Keysound tracks play the chosen preset.", truth: "FEATURE-COVERAGE extra roadmap", manual: "needs a SoundFont and listening" },
  { id: "N011", area: "Since v1.1", feature: "Shared Base layer across difficulties", intended: "Capture counts the moments; Apply all asks first; the Base lives in app data.", truth: "NOW.md" },
  { id: "N012", area: "Since v1.1", feature: "Revert mapset to original / Undo revert", intended: "Restores every file to the first snapshot; Undo revert brings the edits back.", truth: "improvements-audit 2026-11-24" },
  { id: "N013", area: "Since v1.1", feature: "Command palette (Ctrl+K)", intended: "Typing a command's name and Enter runs it.", truth: "commands.js:229-247" },
  { id: "N014", area: "Since v1.1", feature: "Pattern stamp, clone variant, alternate variants", intended: "Stamp repeats the selection n times at the step.", truth: "dawsu! rule" },
  { id: "N015", area: "Since v1.1", feature: "Stem separation and drum stem monitor", intended: "Separates drums into app data, never the mapset.", truth: "dawsu! rule", manual: "needs the ONNX model and minutes of CPU" },
  { id: "N016", area: "Since v1.1", feature: "Crash recovery and autosave", intended: "After a crash, Restore reopens the last edit as unsaved.", truth: "FEATURE-COVERAGE intro", manual: "two-process check in apps/tauri tests" },
  { id: "N017", area: "Since v1.1", feature: "Second save after a slider hitsound change (issue #132)", intended: "A second Save after changing a slider's hitsound succeeds and writes the change.", truth: "issue #132" },
  { id: "N018", area: "Since v1.1", feature: "Lane colour wheel (Ctrl+scroll on lane strip)", intended: "Changes only the lane's colour.", truth: "PR #105" },
];

// What the merge gate (tools/everyday-check.mjs) already checks, by its step names. Those steps
// check that a screen opens and answers; the rows above say what the longer robots judge.
export const GATE = {
  F015: "Add a lane", F032: "Zoom and scroll the timeline", F041: "Fold and show the panels",
  F042: "Open and close the map preview window", F054: "Open and close the Export window",
  F059: "Open and close the drum window", F076: "Open and close the shortcut editor", F070: "Start with no map and see the start screen",
  N001: "Try the bundled practice mapset from the start screen", N005: "Keysounds mode: the roll …",
  N006: "The objects row draws the osu! skin picked for the map preview", N011: "Shared Base: capture shows the moment count, Apply all asks first",
};

/** The features with the robot steps that cover them, given steps' { name, covers } lists. */
export function coverage(steps) {
  return FEATURES.map(feature => ({ ...feature, gate: GATE[feature.id] ?? null,
    steps: steps.filter(step => step.covers?.includes(feature.id)).map(step => step.name) }));
}

/** The list as a Markdown table. */
export function markdown(steps) {
  const rows = coverage(steps);
  const cell = text => String(text ?? "").replaceAll("|", "\\|");
  const checked = rows.filter(row => row.steps.length).length;
  const gateOnly = rows.filter(row => !row.steps.length && row.gate).length;
  const manual = rows.filter(row => !row.steps.length && !row.gate && row.manual).length;
  return [
    "# dawsu! features, intended behaviour and robot coverage",
    "",
    `Generated by \`node tools/feature-check.mjs --features --markdown\` from tools/robot/features.mjs. ${rows.length} features: ` +
      `${checked} judged by a robot step, ${gateOnly} only opened by the merge gate, ${manual} need a person or hardware, ` +
      `${rows.length - checked - gateOnly - manual} not checked yet.`,
    "",
    "| ID | Area | Feature | Should do | Truth | Robot check |",
    "| --- | --- | --- | --- | --- | --- |",
    ...rows.map(row => `| ${row.id} | ${cell(row.area)} | ${cell(row.feature)} | ${cell(row.intended)} | ${cell(row.truth)} | ` +
      `${cell(row.steps.length ? row.steps.join("; ") : row.gate ? `Gate opens it: ${row.gate}` : row.manual ? `By hand: ${row.manual}` : "Not yet")} |`),
    "",
  ].join("\n");
}
