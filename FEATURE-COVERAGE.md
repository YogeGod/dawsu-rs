# Full feature target and current checkpoint

Date: 2026-11-17. The user explicitly requested every planned feature, including Later. The full request is **unfinished**. No row is accepted as fully verified in this checkpoint.

## Available for the first test

Real `.osu` opening and `.osz` import; difficulty switching with unsaved-edit protection; real sample lanes; basic seven-tool input; selection/copy/cut/paste/duplicate/delete; undo/redo surviving save; grid/relative/object snap and beat divisions; volume editing and colouring; lane mute/solo/audition; waveform; output-clock transport and cycle; prepared pitch-preserving playback speeds; basic preferences and recent maps; command-catalogue input/help; safe save and reviewed New/Generate/Copy output.

These features still need native-window and listening checks. Some are partial implementations of larger parity rows. Sample-mix rendering is not implemented, and an operation needing it is refused before any map write.

Tauri crash recovery passed the two-process Windows check. The start screen offered the recovery copy, Restore reopened the last completed edit as unsaved, and Save archived the copy. [Windows relaunch check](apps/tauri/tests/recovery-relaunch.mjs)

## Missing from this build

The node graph and classic detection parity; Live and Record/MIDI; Keysound mode and `.dawsuks`; VST3, SoundFont/SFZ and drum rendering; mixer/effect returns; stem splitting; osu! client link/live reload; map preview; licensed sound packs/map search; installer/updater and cross-platform runtime proof. Remaining M3 work includes complete project/legacy persistence, sound sets, sample replacement/watching, movable tabs, full command/key customisation, full timeline navigation/markers, export settings and sample-mix materialisation.

## Planned parity capabilities

“Basic code” means only a first path is present, not acceptance of the whole capability. “Not delivered” is explicit. Other rows remain unaccepted and must be checked against the actual implementation, not inferred from an underlying M2 library.

| ID | Area | Capability | Current coverage |
| --- | --- | --- | --- |
| F001 | Hitsound – Map files | Open a `.osu`; drop a `.osu` or `.osz` on the window | Basic code; native check pending |
| F002 | Hitsound – Map files | Recent maps | Basic code; native check pending |
| F003 | Hitsound – Map files | Import `.osz` | Basic code; native check pending |
| F004 | Hitsound – Map files | New hitsound diff | Basic code; native check pending |
| F005 | Hitsound – Map files | Revert to last saved | Confirmation and saved-file reload covered by automated workflow test. |
| F006 | Hitsound – Map files | Open map folder / hitsound folder / project folder | Not accepted as complete; partial or pending |
| F007 | Hitsound – Map files | Sample files added to the folder show up as lanes | Not accepted as complete; partial or pending |
| F008 | Hitsound – Map files | Difficulty list on the ribbon (switch difficulty, asks about unsaved edits) | Not accepted as complete; partial or pending |
| F009 | Hitsound – osu! connection | Open map from osu! (osu!stable from memory, osu!lazer after Edit externally) | Not delivered |
| F010 | Hitsound – osu! connection | Live reload when osu!'s editor saves, keeping unsaved hitsound edits | Not delivered |
| F011 | Hitsound – osu! connection | Reload osu!'s editor after Save | Not delivered |
| F012 | Hitsound – Lanes | Sample lanes for hitnormal, whistle, finish and clap, default sound first | Basic code; native check pending |
| F013 | Hitsound – Lanes | Sample-set folders (Normal, Soft, Drum) with M and S, reorderable headers | Not accepted as complete; partial or pending |
| F014 | Hitsound – Lanes | Numbered layers (+ on a lane) | Not accepted as complete; partial or pending |
| F015 | Hitsound – Lanes | Add, delete and filter lanes; hide lane names | Not accepted as complete; partial or pending |
| F016 | Hitsound – Lanes | Active lane and sample replacement (file browser or drop) | Not accepted as complete; partial or pending |
| F017 | Hitsound – Lanes | Mute, solo and audition per lane, plus the Song audio row | Not accepted as complete; partial or pending |
| F018 | Hitsound – Lanes | Collapse sample layers | Not accepted as complete; partial or pending |
| F019 | Hitsound – Lanes | Only lanes with hits ("Used only") | Basic code; native check pending |
| F020 | Hitsound – Hit editing | Tools: Select, Grab, Draw, Erase, Mute, Ramp, Range | Not accepted as complete; partial or pending |
| F021 | Hitsound – Hit editing | Cut, copy, paste at playhead, duplicate, delete, select all | Basic code; native check pending |
| F022 | Hitsound – Hit editing | Quantize, repeat, edit range, scale by volume | Not accepted as complete; partial or pending |
| F023 | Hitsound – Hit editing | Insert volume for new hits | Basic code; native check pending |
| F024 | Hitsound – Hit editing | Protect hitnormals from deletion | Basic code; native check pending |
| F025 | Hitsound – Hit editing | Slider body sounds | Not accepted as complete; partial or pending |
| F026 | Hitsound – Hit editing | Undo, redo and the History panel | Volume edit and History restore verified on Windows. |
| F027 | Hitsound – Hit editing | Drum hit detection (kick/snare, beat grid, stem option) | Not delivered |
| F028 | Hitsound – Hit editing | Live: play lanes from the computer keyboard or MIDI **(3.1.4)** | Not delivered |
| F029 | Hitsound – Hit editing | Record taps from the playhead, as one undo step **(3.1.4)** | Not delivered |
| F030 | Hitsound – Timeline navigation | Snap on/off and snap modes: Grid, Grid relative, Objects | Basic code; native check pending |
| F031 | Hitsound – Timeline navigation | Beat divisions 1/n and 2/1–4/1 | Basic code; native check pending |
| F032 | Hitsound – Timeline navigation | Zoom, Fit and follow playback | Not accepted as complete; partial or pending |
| F033 | Hitsound – Timeline navigation | Overview strip | Overview navigation verified on Windows and in automated tests. |
| F034 | Hitsound – Timeline navigation | Bookmarks with labels | Not accepted as complete; partial or pending |
| F035 | Hitsound – Timeline navigation | L/R locators and cycle | Locator markers and cycle controls present. Native check pending. |
| F036 | Hitsound – Timeline navigation | Time signatures (editor-only) | Not accepted as complete; partial or pending |
| F037 | Hitsound – Timeline navigation | Object lines and warning markers | Object lines added. Warning markers pending. |
| F038 | Hitsound – Timeline navigation | Z/V jumps to the first and last object | Basic code; native check pending |
| F039 | Hitsound – Timeline navigation | Snap colouring | Not accepted as complete; partial or pending |
| F040 | Hitsound – Timeline navigation | Position display: time, bar.beat.tick, tempo, time signature and snap | Position and tempo display added. Native check pending. |
| F041 | Hitsound – Timeline navigation | Show and hide the Inspector, Lane info, volume Controller and Drums view | Not accepted as complete; partial or pending |
| F042 | Hitsound – Timeline navigation | Map preview window (circles, sliders, spinners, skins, stacking) | Not delivered |
| F043 | Hitsound – Volume control | Volume controller: one bar per circle or slider end | Not accepted as complete; partial or pending |
| F044 | Hitsound – Volume control | Drag, Alt-drag to paint, Shift-drag or Ramp tool for ramps | Not accepted as complete; partial or pending |
| F045 | Hitsound – Volume control | Ramp shapes | Not accepted as complete; partial or pending |
| F046 | Hitsound – Volume control | Volume colours on hits (blue → purple → red) | Basic code; native check pending |
| F047 | Hitsound – Volume control | Selection card: relative or set-all volume, start time | Not accepted as complete; partial or pending |
| F048 | Hitsound – Map output | Save `.osu` with a backup outside the mapset (native sound combinations first, mixing only where needed) | Not accepted as complete; partial or pending |
| F049 | Hitsound – Map output | Volume floor on save: changed hits written at 5% or more (3.1.3) | Not accepted as complete; partial or pending |
| F050 | Hitsound – Map output | HS difficulty mode | Not accepted as complete; partial or pending |
| F051 | Hitsound – Map output | Mixed sample indexes and sets on one object | Not accepted as complete; partial or pending |
| F052 | Hitsound – Map output | Save/export problems, warning badge and Next problem | Not accepted as complete; partial or pending |
| F053 | Hitsound – Map output | Generate hitsound diff, with review output | Not accepted as complete; partial or pending |
| F054 | Hitsound – Map output | Export settings: merge rules and presets | Not accepted as complete; partial or pending |
| F055 | Hitsound – Map output | Copy hitsounds to other difficulties | Not accepted as complete; partial or pending |
| F056 | Hitsound – Sound sets | Save and apply sound sets | Not accepted as complete; partial or pending |
| F057 | Hitsound – Sound sets | Role-preserving snapshots in the mapset | Not accepted as complete; partial or pending |
| F058 | Hitsound – Sound sets | Sound sets panel in the inspector | Not accepted as complete; partial or pending |
| F059 | Hitsound – Drums | Drum Map Setup and portable `.dawsudrum` presets (Yoge Expanded / Simple Kit) | Not accepted as complete; partial or pending |
| F060 | Hitsound – Drums | Map lanes from the drum instrument (GGD exports, note names, optional chokes) | Not delivered |
| F061 | Hitsound – Drums | Drum instrument: a VST3 or a SoundFont kit | Not delivered |
| F062 | Hitsound – Drums | Live Drums VST playback | Not delivered |
| F063 | Hitsound – Drums | Render drums to WAV (hash-checked cache) | Not delivered |
| F064 | Hitsound – Playback | Transport: play, stop, go to start, jump to L/R | Basic code; native check pending |
| F065 | Hitsound – Playback | Playback speed 25–150% without pitch change | Basic code; native check pending |
| F066 | Hitsound – Playback | Return on stop, start from | Basic code; native check pending |
| F067 | Hitsound – Playback | Song volume, mute and offset (not saved into the map) | Song offset survives reopen and imports from legacy projects. Physical audio check pending. |
| F068 | Hitsound – Playback | Global volume (never below 20%) | Not accepted as complete; partial or pending |
| F069 | Hitsound – Playback | Song waveform coloured by frequency | Not accepted as complete; partial or pending |
| F070 | Start screen | Start screen (recent maps, mode choice, house button back to it) | Not accepted as complete; partial or pending |
| F071 | Window, ribbon and menus | Ribbon groups (Map, History, Tool, Grid, Drums, View) whose labels shrink in narrow windows; Save keeps its label | Not accepted as complete; partial or pending |
| F072 | Window, ribbon and menus | Menus show each item's icon and shortcut, on/off settings as switches, and Export style, Start playing from and Speed as rows of buttons | Basic code; native check pending |
| F073 | Window, ribbon and menus | Top bar shows the open `.osu` file name, its path on hover, and "…" for a long name | Basic code; native check pending |
| F074 | Shortcuts | Key presets: DAW-like (from Cubase `Key Commands.xml`) or osu! | Basic code; native check pending |
| F075 | Shortcuts | First-launch preset choice | Basic code; native check pending |
| F076 | Shortcuts | Shortcut customisation: key changes, filters, `.dawsukeys` import/export | Not accepted as complete; partial or pending |
| F077 | Preferences | Songs folder | Not accepted as complete; partial or pending |
| F078 | Preferences | Audio output (WASAPI, ASIO, DirectSound, MME) and buffer | Not accepted as complete; partial or pending |
| F079 | Preferences | File browser style: Classic or Folder tree | Not accepted as complete; partial or pending |
| F080 | Preferences | Colour schemes (five, Slate default), accent, grid and ruler colours | Not accepted as complete; partial or pending |
| F081 | Preferences | Existing `preferences.json` read on first start | Not accepted as complete; partial or pending |
| F082 | Help and updates | F1: searchable list of every shortcut and mouse action | Not accepted as complete; partial or pending |
| F083 | Help and updates | Quick start and full interface tour | Not delivered |
| F084 | Help and updates | Check for updates; download and restart | Not accepted as complete; partial or pending |
| F085 | Data compatibility (not a feature area, but needed for parity) | Editor project files (lane layout, hits with no object, sound-set links, HS-mode state, locators, export settings, offset, time signatures) | Locators, cycle, offset, zoom, lane height and scroll position persist. Remaining project fields and migrations are pending. |
| F086 | Data compatibility (not a feature area, but needed for parity) | `.dawsukeys` key files | Not accepted as complete; partial or pending |
| F087 | Data compatibility (not a feature area, but needed for parity) | `.dawsuks` Keysound projects | Not delivered |
| F088 | Data compatibility (not a feature area, but needed for parity) | `.dawsudrum` drum maps, `drum-maps/` and `vst-renders/` in app data | Not delivered |

## Extra roadmap requirements

The mixer, effect-host transport, SoundFont/SFZ, stem CPU/GPU/cache/alignment, starter sounds/packs and map search requirements supplement the old parity table. They are not implemented in this checkpoint. Delegated approval of their designs is not implementation evidence.

The authoritative plan is E:/Obsidian/Workspace/.handoff/plan-complete-editor.md. Unfinished tasks and decisions remain in E:/Obsidian/Workspace/dawsu-rewrite/todo.md.
