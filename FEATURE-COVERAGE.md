# Full feature target and current checkpoint

Date: 2026-11-18. The full beta target includes the Later workflows. This checkpoint covers M3 and records the verification for each row.

## Available for the first test

Real `.osu` opening and `.osz` import; difficulty switching with unsaved-edit protection; real sample lanes; basic seven-tool input; selection/copy/cut/paste/duplicate/delete; undo/redo surviving save; grid/relative/object snap and beat divisions; volume editing and colouring; lane mute/solo/audition; waveform; output-clock transport and cycle; prepared pitch-preserving playback speeds; basic preferences and recent maps; command-catalogue input/help; safe save and reviewed New/Generate/Copy output.

Native-window and listening checks remain for several workflows. Ordinary Save, HS-mode Save, Generate and Copy render supported sample mixes. Green-line Save and Copy support indexed mixes where the compiler can isolate the slider node's lookup.

## Missing from this build

The node graph and classic detection parity, Live and Record/MIDI, Keysound mode and `.dawsuks`, VST3, SoundFont/SFZ and drum rendering, mixer/effect returns, stem splitting, osu! client link and live reload, map preview, licensed sound packs and map search, installer and updater, and cross-platform runtime proof remain open. Remaining M3 work includes linked Sound sets, HS-circle ownership, general lane addition and deletion, complete selection controls, advanced Generate and Copy options, and audio output preferences. Linked Sound sets were deferred for this pass.

## Planned parity capabilities

“Basic code” means only a first path is present, not acceptance of the whole capability. “Not delivered” is explicit. Other rows remain unaccepted and must be checked against the actual implementation, not inferred from an underlying M2 library.

| ID | Area | Capability | Current coverage |
| --- | --- | --- | --- |
| F001 | Hitsound – Map files | Open a `.osu`; drop a `.osu` or `.osz` on the window | Windows timeline preview: native picker opened a copied difficulty. Tauri opened a synthetic difficulty on Windows. The file-drop bridge tests route .osu and .osz paths to Open. Native file drop and archive import checks remain open. |
| F002 | Hitsound – Map files | Recent maps | Basic code; native check pending |
| F003 | Hitsound – Map files | Import `.osz` | Basic code; native check pending |
| F004 | Hitsound – Map files | New hitsound diff | Basic code; native check pending |
| F005 | Hitsound – Map files | Revert to last saved | Confirmation and saved-file reload covered by automated workflow test. |
| F006 | Hitsound – Map files | Open map folder / hitsound folder / project folder | Tauri routes all three to the native shell. The hitsound library uses its own saved folder and picker. Native opening and picker checks pending. |
| F007 | Hitsound – Map files | Sample files added to the folder show up as lanes | Automated: a numbered sample added after opening appears as a lane and plays without reopening. Existing edits, redo and map bytes are preserved. Native folder check pending. |
| F008 | Hitsound – Map files | Difficulty list on the ribbon (switch difficulty, asks about unsaved edits) | Tauri tests exercise Save, Discard, Cancel and stale Save refusal. The ribbon remains accessible with the inspector hidden. Native Windows switched to the generated difficulty and reopened it. |
| F009 | Hitsound – osu! connection | Open map from osu! (osu!stable from memory, osu!lazer after Edit externally) | Not delivered |
| F010 | Hitsound – osu! connection | Live reload when osu!'s editor saves, keeping unsaved hitsound edits | Not delivered |
| F011 | Hitsound – osu! connection | Reload osu!'s editor after Save | Not delivered |
| F012 | Hitsound – Lanes | Sample lanes for hitnormal, whistle, finish and clap, default sound first | Automated transport test mixes a skin hitnormal and numbered map whistle, finish and clap at 50%. Physical listening is pending. |
| F013 | Hitsound – Lanes | Sample-set folders (Normal, Soft, Drum) with M and S, reorderable headers | Tauri has reorderable Normal, Soft and Drum folders, group M/S, and per-difficulty order and collapse. A sibling-map switch and reopen preserve the layout without changing map bytes. Native checks pending. |
| F014 | Hitsound – Lanes | Numbered layers (+ on a lane) | The Tauri plus control creates a new playable family WAV without overwriting existing assets. The fixture adds layers 8 and 9 while preserving sibling references and file bytes. |
| F015 | Hitsound – Lanes | Add, delete and filter lanes; hide lane names | Tauri filters lane names and source paths with stable model rows, and View can hide lane names. Numbered and external replacement paths exist. General lane addition and deletion remain open. |
| F016 | Hitsound – Lanes | Active lane and sample replacement (file browser or drop) | The Tauri mapset browser previews before assignment. Native picker and audio drop assign to the captured lane. The copied-map regression skips occupied and sibling-reserved indexes, verifies undo, redo, Save, reopen and exact PCM, and preserves the source and sibling map. Native picker, drop and listening pending. |
| F017 | Hitsound – Lanes | Mute, solo and audition per lane, plus the Song audio row | Automated: lane audition resolves the numbered sample and muting the clap hit keeps the other three cues playing. Native lane mute, solo and listening checks remain open. |
| F018 | Hitsound – Lanes | Collapse sample layers | Tauri collapses numbered families into previews and expands them before edits. Per-difficulty collapse is restored after switching or reopening. Native visual check pending. |
| F019 | Hitsound – Lanes | Only lanes with hits ("Used only") | Tauri snapshots the used lanes when enabled and preserves original row identities for pointer edits, mute, solo and audition. Frontend regression passed. |
| F020 | Hitsound – Hit editing | Tools: Select, Grab, Draw, Erase, Mute, Ramp, Range | Tauri exposes all seven tools. App and kernel checks cover raw Grab pan, vertical scroll, visible listening-only mute, undoable Erase strokes, drawing, selection, ramps and ranges. Native gesture check pending. |
| F021 | Hitsound – Hit editing | Cut, copy, paste at playhead, duplicate, delete, select all | Basic code; native check pending |
| F022 | Hitsound – Hit editing | Quantize, repeat, edit range, scale by volume | Tauri tests verify strength-50 quantize of a loose hit, a repeat crossing a red line, marked-range Copy/Move/Delete with Undo, and a shared-volume scale applied once. Native checks pending. |
| F023 | Hitsound – Hit editing | Insert volume for new hits | Basic code; native check pending |
| F024 | Hitsound – Hit editing | Protect hitnormals from deletion | Basic code; native check pending |
| F025 | Hitsound – Hit editing | Slider body sounds | Select a slider node to edit the slide and tick sample set, toggle the whistle and edit linked body/head volume. Copied-map Undo, Save, reopen and decoded playback-cue checks passed. Independent whistle sample set override, listening and import remain open. |
| F026 | Hitsound – Hit editing | Undo, redo and the History panel | Volume edit and History restore verified on Windows. |
| F027 | Hitsound – Hit editing | Drum hit detection (kick/snare, beat grid, stem option) | Not delivered |
| F028 | Hitsound – Hit editing | Live: play lanes from the computer keyboard or MIDI **(3.1.4)** | Not delivered |
| F029 | Hitsound – Hit editing | Record taps from the playhead, as one undo step **(3.1.4)** | Not delivered |
| F030 | Hitsound – Timeline navigation | Snap on/off and snap modes: Grid, Grid relative, Objects | Basic code; native check pending |
| F031 | Hitsound – Timeline navigation | Beat divisions 1/n and 2/1–4/1 | Basic code; native check pending |
| F032 | Hitsound – Timeline navigation | Zoom, Fit and follow playback | Tauri Fit, follow playback and pointer-anchored lane zoom pass frontend tests, including the lane-height buttons and slider. Native check pending. |
| F033 | Hitsound – Timeline navigation | Overview strip | Overview navigation verified on Windows and in automated tests. |
| F034 | Hitsound – Timeline navigation | Bookmarks with labels | Tauri shows labels on the ruler and in the inspector. Automated navigation, rename, removal and reopen preserve each difficulty’s bookmarks and its original map bytes. Native check pending. |
| F035 | Hitsound – Timeline navigation | L/R locators and cycle | Locator markers and cycle controls present. Native check pending. |
| F036 | Hitsound – Timeline navigation | Time signatures (editor-only) | Tauri accepts per-bar changes and shows bar numbers and the active signature. The 7/8 fixture covers a tempo change, unchanged quarter-note snap, reopen and unchanged map bytes. Native ruler check pending. |
| F037 | Hitsound – Timeline navigation | Object lines and warning markers | Tauri draws object lines and timed Save/export warning markers for the active difficulty. Stale diagnostics clear after edits or undo. Native visual check pending. |
| F038 | Hitsound – Timeline navigation | Z/V jumps to the first and last object | Basic code; native check pending |
| F039 | Hitsound – Timeline navigation | Snap colouring | Tauri ruler and grid distinguish snap subdivisions and reset at tempo changes. Automated colour and tempo-boundary checks passed. Native visual check pending. |
| F040 | Hitsound – Timeline navigation | Position display: time, bar.beat.tick, tempo, time signature and snap | Tauri transport shows time and Rust-derived bar.beat.tick, BPM, time signature and snap. The fixture checks 1.3.192 at 120 BPM. Native check pending. |
| F041 | Hitsound – Timeline navigation | Show and hide the Inspector, Lane info, volume Controller and Drums view | Tauri View controls hide and restore the Inspector, lane names and volume controller with reclaimed space. Lane info shows sample indexes. Drums view remains open. |
| F042 | Hitsound – Timeline navigation | Map preview window (circles, sliders, spinners, skins, stacking) | Not delivered |
| F043 | Hitsound – Volume control | Volume controller: one bar per circle or slider end | A Tauri fixture verifies one shared bar for three sounds on a circle, plus the head, repeat and tail of a slider. Native visual check pending. |
| F044 | Hitsound – Volume control | Drag, Alt-drag to paint, Shift-drag or Ramp tool for ramps | Tauri tests cover plain drag, relative multiselect, Alt painting, Shift and Ramp gestures, Cancel and one-step Undo with unchanged fixture bytes. Native interaction pending. |
| F045 | Hitsound – Volume control | Ramp shapes | Tauri applies Linear, Ease in, Ease out and S-curve. Tests compare the curve preview and committed volume values, including reverse drag, Shift and Undo. Native interaction pending. |
| F046 | Hitsound – Volume control | Volume colours on hits (blue → purple → red) | Hits and controller bars share the C# palette. Frontend checks cover endpoints, the midpoint and intermediate volumes. Native visual check pending. |
| F047 | Hitsound – Volume control | Selection card: relative or set-all volume, start time | Tauri shows selection count and context, relative and set-all volume, Quantize strength, Duplicate and Delete. Moving Start from 1120 to 1330.5 ms preserved two hits’ spacing and volumes, left game objects in place and undid in one step. Per-lane gain editing remains open. |
| F048 | Hitsound – Map output | Save `.osu` with a backup outside the mapset (native sound combinations first, mixing only where needed) | Automated: native Save and mixed circle Save, external backup, byte-identical unrelated lines, resave, and preservation on stale edits, blocked backups or missing samples. Native Save check pending. |
| F049 | Hitsound – Map output | Volume floor on save: changed hits written at 5% or more (3.1.3) | Tauri Save writes a changed 3% hit at 5%. The untouched 2% hit and every other CRLF fixture byte are preserved, and the saved values reopen correctly. |
| F050 | Hitsound – Map output | HS difficulty mode | Background Save merges two loose hits at 10,000 and 10,000.25 ms into one circle with the 80% and 40% layers mixed. Reopen and resave keep map bytes and avoid loose duplicates. The mode flag survives switching difficulties and reopening. Restoring ownership of generated circles and native listening remain open. |
| F051 | Hitsound – Map output | Mixed sample indexes and sets on one object | Save, HS-mode Save, Generate and Copy render mixed circles and supported spinner ends. Green-line Save and Copy also cover safely encodable slider nodes using indexed WAVs and checked restoring lines. A tail fixture checks PCM, sibling index reservations, reopened bytes, re-Save and failed writes. Native listening and import pending. |
| F052 | Hitsound – Map output | Save/export problems, warning badge and Next problem | Tauri Save preview lists timed issues and supports First and Next. Ctrl+S uses checked Save directly. Generate and Copy report typed problems. The warning marker checks passed in the frontend suite. Native interaction is pending. |
| F053 | Hitsound – Map output | Generate hitsound diff, with review output | Generate stages PCM mixes and optional slide, whistle and tick audio before Apply. Regenerate checks the prior output hash, backs up the difficulty and reuses byte-matching WAVs. Native Windows staged, applied and reopened a synthetic mix, then Save left all five mapset hashes unchanged. Detailed job progress, repeatable first-preview WAV names, listening and osu! import remain open. |
| F054 | Hitsound – Map output | Export settings: merge rules and presets | Tauri edits per-difficulty pairing, slot priority and combination rules, with C# v1 JSON import/export. Review stages fused WAVs or role-preserving files under one index and retains native combinations. The copied preset fixture verifies hash-based rules, reopen, exact PCM and explicit Apply. Native preset dialogs pending. |
| F055 | Hitsound – Map output | Copy hitsounds to other difficulties | Tauri sends the chosen Copy policy and selected targets to review, where mixed WAVs can be auditioned. Tests check target storyboard preservation, exact PCM, stale-target refusal, and rollback after a cancelled partial write. Supported slider nodes are covered. Slider tick and slide propagation options and native checks remain open. |
| F056 | Hitsound – Sound sets | Save and apply sound sets | Linked editor placement and member gains were deferred for this pass. The domain codec remains covered by synthetic tests. |
| F057 | Hitsound – Sound sets | Role-preserving snapshots in the mapset | Domain snapshot planning is covered, including collision avoidance and verified reuse. The file-writing and editor placement workflow was deferred for this pass. |
| F058 | Hitsound – Sound sets | Sound sets panel in the inspector | Deferred with the linked Sound sets workflow. |
| F059 | Hitsound – Drums | Drum Map Setup and portable `.dawsudrum` presets (Yoge Expanded / Simple Kit) | Open check: Import both named drum presets, export a portable drum map and reopen it in a fresh project. |
| F060 | Hitsound – Drums | Map lanes from the drum instrument (GGD exports, note names, optional chokes) | Not delivered |
| F061 | Hitsound – Drums | Drum instrument: a VST3 or a SoundFont kit | Not delivered |
| F062 | Hitsound – Drums | Live Drums VST playback | Not delivered |
| F063 | Hitsound – Drums | Render drums to WAV (hash-checked cache) | Not delivered |
| F064 | Hitsound – Playback | Transport: play, stop, go to start, jump to L/R | Tauri has Go to start and Stop-again jumps to zero. Existing Play and locator controls remain. The Tauri worker checks both zero-seek actions. |
| F065 | Hitsound – Playback | Playback speed 25–150% without pitch change | Tauri offers 25%-150% pitch-preserving playback speeds. Picker regression and offline audio tests cover the render path. Listening pending. |
| F066 | Hitsound – Playback | Return on stop, start from | Tauri Play and Space use Cursor, Left locator, Selection, or Selection or left locator. The exact playback-start return point is retained across seeks. Native transport check pending. |
| F067 | Hitsound – Playback | Song volume, mute and offset (not saved into the map) | Song offset survives reopen and imports from legacy projects. Physical audio check pending. |
| F068 | Hitsound – Playback | Global volume (never below 20%) | Master gain is clamped to 20% on load and edit. An isolated restart regression feeds in a saved value of 5% and verifies the UI reports 20%. Physical output check pending. |
| F069 | Hitsound – Playback | Song waveform coloured by frequency | A synthetic 70/650/8000 Hz song produced the expected peak and dominant band in each portion. Tauri canvas tests verify distinct ruler and overview draw colours and amplitudes. Native visual review pending. |
| F070 | Start screen | Start screen (recent maps, mode choice, house button back to it) | Tauri Home opens recent maps and resumes a parked editor. The copied-map test covers Cancel, stale Save refusal preserving edits, and Discard with reopen. Keysound mode and native navigation check are pending. |
| F071 | Window, ribbon and menus | Ribbon groups (Map, History, Tool, Grid, Drums, View) whose labels shrink in narrow windows; Save keeps its label | Tauri Map, History, Tool, Grid and View controls shrink at three breakpoints and keep the Save label. Drums controls and native resize check remain open. |
| F072 | Window, ribbon and menus | Menus show each item's icon and shortcut, on/off settings as switches, and Export style, Start playing from and Speed as rows of buttons | Tauri has in-window Map, Edit, View and Playback menus with C# icons, active shortcut labels, switches and playback option rows. Generate displays its Per-object style. Keyboard and dismissal tests passed. Native focus and placement check pending. |
| F073 | Window, ribbon and menus | Top bar shows the open `.osu` file name, its path on hover, and "…" for a long name | Tauri shows the actual .osu filename with a dirty marker, full path on hover and ellipsis on overflow. Frontend regression passed; native resize pending. |
| F074 | Shortcuts | Key presets: DAW-like (from Cubase `Key Commands.xml`) or osu! | Tauri uses the Rust preset. Automated checks exercise snap, lane size, Ctrl+Right nudge, NumPad locator jump, undo and four-grid osu! seek. Native keypress checks pending. |
| F075 | Shortcuts | First-launch preset choice | Tauri prompts for DAW-like or osu! and persists the chosen preset. Automated first-launch checks cover both choices. Native prompt check pending. |
| F076 | Shortcuts | Shortcut customisation: key changes, filters, `.dawsukeys` import/export | F1 filters, captures and edits per-preset bindings. A rebound Help command runs through the Rust resolver. V1 import/export round-trips supported bindings, reports conflicts and applies legacy aliases. Native key capture and picker checks pending. |
| F077 | Preferences | Songs folder | Tauri can choose a Songs folder before opening a map. The next native picker reads the saved directory. Worker regression checks restart persistence; native picker check pending. |
| F078 | Preferences | Audio output (WASAPI, ASIO, DirectSound, MME) and buffer | Tauri lists devices on the current CPAL host (shared WASAPI on Windows) and requests 10–500 ms or device default. Switching while stopped preserves transport and gains. Tests cover rejected requests and saved preferences. Physical devices and ASIO, DirectSound and MME remain open. |
| F079 | Preferences | File browser style: Classic or Folder tree | Tauri browses mapset or configured library samples in the Folder tree sidebar and Classic inspector. Tests cover lazy nested browsing, filtering, Up, preview, guarded assignment and saved-style dispatch. Native listening and restart checks pending. |
| F080 | Preferences | Colour schemes (five, Slate default), accent, grid and ruler colours | Open check: Cycle all five schemes and reopen with custom accent, grid and ruler colours. |
| F081 | Preferences | Existing `preferences.json` read on first start | The copied C# preferences fixture checks supported paths, toggles, gain and both shortcut presets. Existing native preferences take precedence and the C# file stays byte-identical. Native first launch pending. |
| F082 | Help and updates | F1: searchable list of every shortcut and mouse action | Tauri F1 searches the active Rust catalogue by key, command or mouse action. Frontend regression covers search and empty results. Native keypress pending. |
| F083 | Help and updates | Quick start and full interface tour | Not delivered |
| F084 | Help and updates | Check for updates; download and restart | Open check: Run update, restart and failure recovery against a controlled update fixture. |
| F085 | Data compatibility (not a feature area, but needed for parity) | Editor project files (lane layout, hits with no object, sound-set links, HS-mode state, locators, export settings, offset, time signatures) | Per-difficulty layout, slots, bookmarks, HS-mode flag, locators, offset, time signatures and merge settings reopen. Legacy fixtures cover loose hits, supported lane layout, HS-mode enablement and time signatures. Sound-set links and ownership of legacy HS circles remain open. |
| F086 | Data compatibility (not a feature area, but needed for parity) | `.dawsukeys` key files | Tauri uses the existing C# v1 format. Automated round-trip covers supported legacy gestures and aliases, first-winner conflicts, omitted defaults, active preset isolation and unknown-version refusal. Native I/O and unsupported command coverage remain open. |
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
