# Full feature target and current checkpoint

Date: 2026-11-17. The user explicitly requested every planned feature, including Later. The full request is **unfinished**. No row is accepted as fully verified in this checkpoint.

## Available for the first test

Real `.osu` opening and `.osz` import; difficulty switching with unsaved-edit protection; real sample lanes; basic seven-tool input; selection/copy/cut/paste/duplicate/delete; undo/redo surviving save; grid/relative/object snap and beat divisions; volume editing and colouring; lane mute/solo/audition; waveform; output-clock transport and cycle; prepared pitch-preserving playback speeds; basic preferences and recent maps; command-catalogue input/help; safe save and reviewed New/Generate/Copy output.

These features still need native-window and listening checks. Some are partial implementations of larger parity rows. Sample-mix rendering is not implemented, and an operation needing it is refused before any map write.

## Missing from this build

The node graph and classic detection parity; Live and Record/MIDI; Keysound mode and `.dawsuks`; VST3, SoundFont/SFZ and drum rendering; mixer/effect returns; stem splitting; osu! client link/live reload; map preview; licensed sound packs/map search; installer/updater and cross-platform runtime proof. Remaining M3 work includes complete project/legacy persistence, sound sets, sample replacement/watching, movable tabs, full command/key customisation, full timeline navigation/markers, export settings and sample-mix materialisation.

## Planned parity capabilities

“Basic code” means only a first path is present, not acceptance of the whole capability. “Not delivered” is explicit. Other rows remain unaccepted and must be checked against the actual implementation, not inferred from an underlying M2 library.

| ID | Area | Capability | Current coverage |
| --- | --- | --- | --- |
| F001 | Hitsound – Map files | Open a `.osu`; drop a `.osu` or `.osz` on the window | Windows timeline preview: native picker opened a copied difficulty. Dropping a difficulty and importing a dropped archive still need native checks. |
| F002 | Hitsound – Map files | Recent maps | Basic code; native check pending |
| F003 | Hitsound – Map files | Import `.osz` | Basic code; native check pending |
| F004 | Hitsound – Map files | New hitsound diff | Basic code; native check pending |
| F005 | Hitsound – Map files | Revert to last saved | Confirmation and saved-file reload covered by automated workflow test. |
| F006 | Hitsound – Map files | Open map folder / hitsound folder / project folder | Open check: Open all three folder actions on a copied map and check each resolved directory. |
| F007 | Hitsound – Map files | Sample files added to the folder show up as lanes | Open check: Add a sample while the difficulty is open and verify a playable lane appears without reopening. |
| F008 | Hitsound – Map files | Difficulty list on the ribbon (switch difficulty, asks about unsaved edits) | Open check: Switch with dirty edits and exercise Save, Discard and Cancel. Automated inspector-during-open regression in [Application](crates/dawsu-application/src/lib.rs). |
| F009 | Hitsound – osu! connection | Open map from osu! (osu!stable from memory, osu!lazer after Edit externally) | Not delivered |
| F010 | Hitsound – osu! connection | Live reload when osu!'s editor saves, keeping unsaved hitsound edits | Not delivered |
| F011 | Hitsound – osu! connection | Reload osu!'s editor after Save | Not delivered |
| F012 | Hitsound – Lanes | Sample lanes for hitnormal, whistle, finish and clap, default sound first | Basic code; native check pending |
| F013 | Hitsound – Lanes | Sample-set folders (Normal, Soft, Drum) with M and S, reorderable headers | Open check: Reorder Normal, Soft and Drum headers, then mute and solo each group during playback. |
| F014 | Hitsound – Lanes | Numbered layers (+ on a lane) | Open check: Add two numbered layers to one lane and verify independent editing and playback. |
| F015 | Hitsound – Lanes | Add, delete and filter lanes; hide lane names | Open check: Add a lane, filter it, hide names and delete it. Verify the remaining lane identities. |
| F016 | Hitsound – Lanes | Active lane and sample replacement (file browser or drop) | Open check: Replace a sample once via the browser and once by drop. Audition the destination after each. |
| F017 | Hitsound – Lanes | Mute, solo and audition per lane, plus the Song audio row | Open check: Mute and solo two lanes against the song, then audition during pause and playback. |
| F018 | Hitsound – Lanes | Collapse sample layers | Open check: Collapse and expand sample layers and verify the hits return to their original rows. |
| F019 | Hitsound – Lanes | Only lanes with hits ("Used only") | Basic code; native check pending |
| F020 | Hitsound – Hit editing | Tools: Select, Grab, Draw, Erase, Mute, Ramp, Range | Open check: Exercise all seven tools on one copied map. Verify affected hits and undo each gesture. |
| F021 | Hitsound – Hit editing | Cut, copy, paste at playhead, duplicate, delete, select all | Basic code; native check pending |
| F022 | Hitsound – Hit editing | Quantize, repeat, edit range, scale by volume | Open check: Quantize off-grid hits, repeat a range and scale its volume. Compare the resulting times and volumes. |
| F023 | Hitsound – Hit editing | Insert volume for new hits | Basic code; native check pending |
| F024 | Hitsound – Hit editing | Protect hitnormals from deletion | Basic code; native check pending |
| F025 | Hitsound – Hit editing | Slider body sounds | Open check: Edit and reopen slide, whistle and tick sounds on a slider fixture. |
| F026 | Hitsound – Hit editing | Undo, redo and the History panel | Volume edit and History restore verified on Windows. |
| F027 | Hitsound – Hit editing | Drum hit detection (kick/snare, beat grid, stem option) | Not delivered |
| F028 | Hitsound – Hit editing | Live: play lanes from the computer keyboard or MIDI **(3.1.4)** | Not delivered |
| F029 | Hitsound – Hit editing | Record taps from the playhead, as one undo step **(3.1.4)** | Not delivered |
| F030 | Hitsound – Timeline navigation | Snap on/off and snap modes: Grid, Grid relative, Objects | Basic code; native check pending |
| F031 | Hitsound – Timeline navigation | Beat divisions 1/n and 2/1–4/1 | Basic code; native check pending |
| F032 | Hitsound – Timeline navigation | Zoom, Fit and follow playback | Open check: Zoom around a selected hit, fit the song and follow playback beyond the viewport. |
| F033 | Hitsound – Timeline navigation | Overview strip | Overview navigation verified on Windows and in automated tests. |
| F034 | Hitsound – Timeline navigation | Bookmarks with labels | Open check: Add two labelled bookmarks, navigate between them and reopen the difficulty. |
| F035 | Hitsound – Timeline navigation | L/R locators and cycle | Locator markers and cycle controls present. Native check pending. |
| F036 | Hitsound – Timeline navigation | Time signatures (editor-only) | Open check: Add a signature change, check bar labels across it and verify unchanged map bytes. |
| F037 | Hitsound – Timeline navigation | Object lines and warning markers | Object lines added. Warning markers pending. |
| F038 | Hitsound – Timeline navigation | Z/V jumps to the first and last object | Basic code; native check pending |
| F039 | Hitsound – Timeline navigation | Snap colouring | Open check: Inspect subdivisions at two snap divisors and compare colours on either side of a tempo change. |
| F040 | Hitsound – Timeline navigation | Position display: time, bar.beat.tick, tempo, time signature and snap | Position and tempo display added. Native check pending. |
| F041 | Hitsound – Timeline navigation | Show and hide the Inspector, Lane info, volume Controller and Drums view | Open check: Toggle each named panel, check its reclaimed space and restore it. |
| F042 | Hitsound – Timeline navigation | Map preview window (circles, sliders, spinners, skins, stacking) | Not delivered |
| F043 | Hitsound – Volume control | Volume controller: one bar per circle or slider end | Open check: Compare controller bars with circle and slider-end times on a copied difficulty. |
| F044 | Hitsound – Volume control | Drag, Alt-drag to paint, Shift-drag or Ramp tool for ramps | Open check: Drag and paint volumes with each modifier, then undo and verify the affected range. |
| F045 | Hitsound – Volume control | Ramp shapes | Open check: Apply each ramp shape to the same hit range and compare intermediate volumes. |
| F046 | Hitsound – Volume control | Volume colours on hits (blue → purple → red) | Basic code; native check pending |
| F047 | Hitsound – Volume control | Selection card: relative or set-all volume, start time | Open check: Set a mixed selection to one volume, apply a relative change and verify its start-time readout. |
| F048 | Hitsound – Map output | Save `.osu` with a backup outside the mapset (native sound combinations first, mixing only where needed) | Open check: Save a native combination, inspect the external backup and reopen. Sample-mix output still needs implementation. |
| F049 | Hitsound – Map output | Volume floor on save: changed hits written at 5% or more (3.1.3) | Open check: Save a changed hit below 5% beside an untouched low-volume hit and inspect the written values. |
| F050 | Hitsound – Map output | HS difficulty mode | Open check: Enable HS mode, change a hit, save and reopen. Check mode state and generated objects. |
| F051 | Hitsound – Map output | Mixed sample indexes and sets on one object | Open check: Place mixed sample indexes and sets at one object. Verify output audio and the reviewed file list. |
| F052 | Hitsound – Map output | Save/export problems, warning badge and Next problem | Open check: Trigger two save problems, check the badge count and use Next problem to visit both. |
| F053 | Hitsound – Map output | Generate hitsound diff, with review output | Open check: Review Generate, cancel and compare file hashes. Repeat, accept, then reopen and inspect the new difficulty. |
| F054 | Hitsound – Map output | Export settings: merge rules and presets | Open check: Save a named export preset, change settings, restore the preset and compare reviewed output. |
| F055 | Hitsound – Map output | Copy hitsounds to other difficulties | Open check: Copy to two difficulties, inspect the review and reopen both targets. Exercise a changed-on-disk target. |
| F056 | Hitsound – Sound sets | Save and apply sound sets | Open check: Save a sound set, apply it on a second selection and compare member lanes and volume. |
| F057 | Hitsound – Sound sets | Role-preserving snapshots in the mapset | Open check: Create a mapset snapshot, reopen and verify member roles and source references. |
| F058 | Hitsound – Sound sets | Sound sets panel in the inspector | Open check: Open the Sound sets inspector, select a saved set and apply it to a hit. |
| F059 | Hitsound – Drums | Drum Map Setup and portable `.dawsudrum` presets (Yoge Expanded / Simple Kit) | Open check: Import both named drum presets, export a portable drum map and reopen it in a fresh project. |
| F060 | Hitsound – Drums | Map lanes from the drum instrument (GGD exports, note names, optional chokes) | Not delivered |
| F061 | Hitsound – Drums | Drum instrument: a VST3 or a SoundFont kit | Not delivered |
| F062 | Hitsound – Drums | Live Drums VST playback | Not delivered |
| F063 | Hitsound – Drums | Render drums to WAV (hash-checked cache) | Not delivered |
| F064 | Hitsound – Playback | Transport: play, stop, go to start, jump to L/R | Basic code; native check pending |
| F065 | Hitsound – Playback | Playback speed 25–150% without pitch change | Basic code; native check pending |
| F066 | Hitsound – Playback | Return on stop, start from | Basic code; native check pending |
| F067 | Hitsound – Playback | Song volume, mute and offset (not saved into the map) | Song offset survives reopen and imports from legacy projects. Physical audio check pending. |
| F068 | Hitsound – Playback | Global volume (never below 20%) | Open check: Set master gain below the floor and verify both the control value and output level. |
| F069 | Hitsound – Playback | Song waveform coloured by frequency | Open check: Inspect waveform amplitude and colour on a fixture with distinct low, middle and high bands. |
| F070 | Start screen | Start screen (recent maps, mode choice, house button back to it) | Open check: Open a recent map, use Home, then enter each available mode from the start screen. |
| F071 | Window, ribbon and menus | Ribbon groups (Map, History, Tool, Grid, Drums, View) whose labels shrink in narrow windows; Save keeps its label | Open check: Narrow the window through each ribbon breakpoint and verify Save remains identifiable. |
| F072 | Window, ribbon and menus | Menus show each item's icon and shortcut, on/off settings as switches, and Export style, Start playing from and Speed as rows of buttons | Basic code; native check pending |
| F073 | Window, ribbon and menus | Top bar shows the open `.osu` file name, its path on hover, and "…" for a long name | Basic code; native check pending |
| F074 | Shortcuts | Key presets: DAW-like (from Cubase `Key Commands.xml`) or osu! | Basic code; native check pending |
| F075 | Shortcuts | First-launch preset choice | Basic code; native check pending |
| F076 | Shortcuts | Shortcut customisation: key changes, filters, `.dawsukeys` import/export | Open check: Rebind and filter a command, export keys, reset and reimport. Verify the resulting shortcut. |
| F077 | Preferences | Songs folder | Open check: Choose a Songs folder, restart and verify the next map operation uses that location. |
| F078 | Preferences | Audio output (WASAPI, ASIO, DirectSound, MME) and buffer | Open check: Record separate WASAPI, ASIO, DirectSound and MME results: device selection, buffer size and physical playback. Mark unavailable backends individually. |
| F079 | Preferences | File browser style: Classic or Folder tree | Open check: Choose Classic and Folder tree in turn, reopen the browser and verify the saved choice. |
| F080 | Preferences | Colour schemes (five, Slate default), accent, grid and ruler colours | Open check: Cycle all five schemes and reopen with custom accent, grid and ruler colours. |
| F081 | Preferences | Existing `preferences.json` read on first start | Open check: Start with a copied legacy preferences file, inspect imported values, and compare original bytes. |
| F082 | Help and updates | F1: searchable list of every shortcut and mouse action | Open check: Press F1, search for a key and a mouse gesture and compare both results with the actual binding. |
| F083 | Help and updates | Quick start and full interface tour | Not delivered |
| F084 | Help and updates | Check for updates; download and restart | Open check: Run update, restart and failure recovery against a controlled update fixture. |
| F085 | Data compatibility (not a feature area, but needed for parity) | Editor project files (lane layout, hits with no object, sound-set links, HS-mode state, locators, export settings, offset, time signatures) | Locators, cycle, offset, zoom, lane height and scroll position persist. Hits with no object are restored from legacy projects. The automated legacy clap workflow covers volume edits, save, reopen and deletion. Other fields and migrations remain pending. |
| F086 | Data compatibility (not a feature area, but needed for parity) | `.dawsukeys` key files | Open check: Import a legacy key file, exercise the binding, export and reload in a fresh session. |
| F087 | Data compatibility (not a feature area, but needed for parity) | `.dawsuks` Keysound projects | Not delivered |
| F088 | Data compatibility (not a feature area, but needed for parity) | `.dawsudrum` drum maps, `drum-maps/` and `vst-renders/` in app data | Not delivered |

## Keysound workflow acceptance

These seven checks cover the separate Keysound requirements in the vault’s feature parity table. Each is pending. Record the build identifier, fixture, platform and observed result when running it.

| ID | Workflow | Acceptance check |
| --- | --- | --- |
| K01 | Project and tracks | Create a project from a copied difficulty, rename and delete tracks, change a time signature, then reopen. Verify the remaining tracks, signature and copied reference. Compare original map bytes. |
| K02 | Piano roll | Draw, split, glue, resize, transpose, copy and quantize notes against visible reference objects. Undo and redo, then verify note times, lengths and pitches. |
| K03 | Note input | Import a multitrack, multichannel MIDI file, record from a MIDI keyboard, then enter a note with musical typing. Verify part assignment, pitches and timing. |
| K04 | Instruments | Play the bundled piano, assign a VST3 and a SoundFont preset to separate tracks, open the instrument editor, save and reopen. Verify assignments, effect settings and audible output. |
| K05 | Gameplay sync | Preview a selection containing a matching and an unmatched note. Apply sync and verify the preview agrees with the resulting assignments. |
| K06 | Generation and export | Import MIDI, sync to gameplay, generate keysounds and reopen the exported archive. Verify each synced note’s WAV, exclusion of unmatched notes from generated output, difficulty references and original map bytes. |
| K07 | Playback | Play a song with a live instrument part, seek and cycle between both locators. Listen for alignment and check that playback, stop and restart preserve position as specified. |

## Full-beta release gates

Record a result for each stage, including the build, platform, copied fixture and evidence. A pending check remains open until someone exercises the workflow.

| Stage | Required evidence |
| --- | --- |
| Manual editing | Complete the Hitsound rows on a copied difficulty. Reopen the edits, compare unrelated lines and exercise undo across save. |
| Output and persistence | Inspect and reopen New, Generate and Copy output. Inject a stale target, cancellation and a write failure. Compare originals and backups. |
| Graph and live input | Compare the classic preset with a fixed old-app result, reject stale proposals and undo each apply or recording in one step. Exercise a real MIDI device. |
| Keysound and instruments | Complete K01–K07, including native instrument UI, exact state restoration, a host failure, explicit drum rendering and import in osu!. |
| Mixer and effects | Route an impulse through isolated returns, verify silence for late or failed hosts, then compare an explicit offline render with the live route. |
| Stems | Split a supplied song and model, check alignment, cache invalidation and cancellation, then run detection on the drum stem. Record CPU and GPU results separately. |
| Client link and preview | Open copied maps from each supported client, reload across unsaved edits and check transport sync in preview. Exercise licensed pack and map import. |
| Distribution | Launch the packaged app from a path with spaces, test local updater success and recovery, and record build and runtime checks by operating system. |

## Extra roadmap requirements

The mixer, effect-host transport, SoundFont/SFZ, stem CPU/GPU/cache/alignment, starter sounds/packs and map search requirements supplement the old parity table. They are not implemented in this checkpoint. Delegated approval of their designs is not implementation evidence.

This ledger records the release checks alongside the code. The design vault’s tracked `dawsu-rewrite/overview/feature-parity.md` sets the workflow scope, `dawsu-rewrite/roadmap.md` sets milestone order, and `dawsu-rewrite/todo.md` holds open decisions. Record outstanding behaviour and build-specific evidence separately before accepting a row.
