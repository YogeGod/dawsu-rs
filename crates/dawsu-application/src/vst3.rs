//! Application-level VST3 plug-in manager wrapping `dawsu-audio-instruments`.
//!
//! Coordinates plug-in discovery, isolated sidecar `HostSession` lifecycle
//! (instrument/effect loading, native editor windows, MIDI preview notes),
//! and WAV bouncing with graceful fallback when the `dawsu-vst3-host` sidecar
//! binary has not been built yet.

use dawsu_audio_instruments::bounce::{trim_silence_tail, write_wav_f32, write_wav_i16};
use dawsu_audio_instruments::host::{
    HostError, HostSession, MidiEvent, ProcessTransport, RenderRequest, Vst3HostConfig,
};
use dawsu_audio_instruments::scanner::{
    ScanError, Vst3PluginInfo, default_vst3_dirs, discover_vst3_paths_in, scan_plugin,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Whether a loaded VST3 instance acts as a MIDI instrument or an audio effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRole {
    Instrument,
    Effect,
}

/// A short preview note sent to a loaded VST3 instrument (e.g. clicking a pad or key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiPreviewNote {
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
    pub duration_ms: u32,
}

impl MidiPreviewNote {
    /// Clamps channel (`0..=15`), note (`0..=127`), and velocity (`0..=127`) to valid MIDI ranges.
    pub fn clamped(self) -> Self {
        Self {
            channel: self.channel.min(15),
            note: self.note.min(127),
            velocity: self.velocity.min(127),
            duration_ms: self.duration_ms.max(1),
        }
    }
}

/// Snapshot of a loaded VST3 plug-in instance for UI and session state reporting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedPluginSummary {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub uid: Option<String>,
    pub role: PluginRole,
    pub editor_open: bool,
    pub alive: bool,
    pub last_preview_note: Option<MidiPreviewNote>,
}

/// Non-fatal per-file failure encountered while scanning a VST3 folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanSkip {
    pub path: PathBuf,
    pub reason: String,
}

/// Result of scanning one or more VST3 directories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vst3ScanReport {
    pub discovered_paths: Vec<PathBuf>,
    pub plugins: Vec<Vst3PluginInfo>,
    pub skipped: Vec<ScanSkip>,
    pub sidecar_available: bool,
}

/// Output sample encoding when bouncing audio to WAV.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BounceFormat {
    #[default]
    Pcm16,
    Float32,
}

/// Options controlling offline VST3 rendering and WAV bouncing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BounceOptions {
    pub sample_rate: u32,
    pub max_frames: u64,
    pub quiet_frames: u64,
    pub silence_threshold: f32,
    pub format: BounceFormat,
}

impl Default for BounceOptions {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            max_frames: 48_000 * 10,
            quiet_frames: 48_000 / 2,
            silence_threshold: 1e-4,
            format: BounceFormat::Pcm16,
        }
    }
}

/// Metadata for a bounced WAV file written to disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BounceOutput {
    pub path: PathBuf,
    pub frames: usize,
    pub sample_rate: u32,
    pub peak: f32,
    pub truncated: bool,
}

/// Strategy for grouping hit volumes/velocities when bouncing VST3 drum samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VelocityExportMode {
    /// Groups nearby volumes on each drum pitch whose span stays within `max_interval`
    /// and bounces one shared WAV per tier at the weighted average velocity.
    Tiered { max_interval: u8 },
    /// Emits a separate WAV file for every distinct volume level on each drum pitch.
    ExactPerVolume,
}

impl Default for VelocityExportMode {
    fn default() -> Self {
        Self::Tiered { max_interval: 20 }
    }
}

/// A programmed drum hit destined for VST3 offline bouncing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vst3DrumHit {
    pub time_ms: f64,
    pub pitch: u8,
    pub volume: u8,
    pub name: String,
}

/// One WAV file to bounce from the VST3 instrument for a drum pitch/velocity tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vst3BounceJob {
    pub filename: String,
    pub pitch: u8,
    pub midi_velocity: u8,
    pub hit_count: usize,
    pub min_volume: u8,
    pub max_volume: u8,
}

/// Maps a single input [`Vst3DrumHit`] to the bounced [`Vst3BounceJob`] it shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vst3HitAssignment {
    pub hit_index: usize,
    pub time_ms: f64,
    pub pitch: u8,
    pub original_volume: u8,
    pub tier_volume: u8,
    pub midi_velocity: u8,
    pub filename: String,
    pub job_index: usize,
}

/// Complete VST3 drum export plan pairing deduplicated bounce jobs with per-hit assignments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vst3DrumExportPlan {
    pub mode: VelocityExportMode,
    pub jobs: Vec<Vst3BounceJob>,
    pub assignments: Vec<Vst3HitAssignment>,
    pub saved_files_vs_exact: usize,
}

fn sanitize_drum_slug(name: &str, pitch: u8) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !slug.is_empty() && !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        format!("note-{pitch}")
    } else {
        slug.to_string()
    }
}

fn volume_to_midi_velocity(volume: u8) -> u8 {
    let clamped = volume.clamp(5, 100);
    ((f64::from(clamped) * 127.0 / 100.0).round() as u8).clamp(1, 127)
}

/// Plans deduplicated VST3 drum sample bounces for `hits` using the requested [`VelocityExportMode`].
///
/// Volumes are grouped per unique MIDI `pitch` so different drum pieces never merge together.
/// In [`VelocityExportMode::Tiered`], adjacent sorted volumes on the same pitch whose span
/// (`max - min`) stays within `max_interval` are clustered into one tier bounced at the
/// hit-count-weighted average volume (rounded, clamped `5..=100`, mapped to MIDI velocity `1..=127`).
pub fn plan_vst3_drum_export(
    hits: &[Vst3DrumHit],
    mode: VelocityExportMode,
) -> Vst3DrumExportPlan {
    let mut by_pitch: BTreeMap<u8, BTreeMap<u8, Vec<usize>>> = BTreeMap::new();
    let mut pitch_slugs: BTreeMap<u8, String> = BTreeMap::new();

    for (idx, hit) in hits.iter().enumerate() {
        let vol = hit.volume.clamp(5, 100);
        by_pitch
            .entry(hit.pitch)
            .or_default()
            .entry(vol)
            .or_default()
            .push(idx);
        pitch_slugs
            .entry(hit.pitch)
            .or_insert_with(|| sanitize_drum_slug(&hit.name, hit.pitch));
    }

    // Disambiguate filename slugs only if two distinct pitches share the exact same sanitized name.
    let mut slug_counts: BTreeMap<String, usize> = BTreeMap::new();
    for slug in pitch_slugs.values() {
        *slug_counts.entry(slug.clone()).or_insert(0) += 1;
    }

    let exact_file_count: usize = by_pitch.values().map(BTreeMap::len).sum();
    let mut jobs = Vec::new();
    let mut assignments = Vec::with_capacity(hits.len());

    for (pitch, volume_groups) in by_pitch {
        let base_slug = &pitch_slugs[&pitch];
        let slug = if slug_counts.get(base_slug).copied().unwrap_or(0) > 1 {
            format!("{base_slug}-p{pitch}")
        } else {
            base_slug.clone()
        };

        let clusters: Vec<Vec<(u8, Vec<usize>)>> = match mode {
            VelocityExportMode::ExactPerVolume => volume_groups
                .into_iter()
                .map(|entry| vec![entry])
                .collect(),
            VelocityExportMode::Tiered { max_interval } => {
                let mut grouped: Vec<Vec<(u8, Vec<usize>)>> = Vec::new();
                for (vol, indices) in volume_groups {
                    let fits = grouped
                        .last()
                        .and_then(|cluster| cluster.first())
                        .is_some_and(|(min_vol, _)| vol.saturating_sub(*min_vol) <= max_interval);
                    if fits {
                        grouped.last_mut().unwrap().push((vol, indices));
                    } else {
                        grouped.push(vec![(vol, indices)]);
                    }
                }
                grouped
            }
        };

        for cluster in clusters {
            let min_volume = cluster.first().map(|(v, _)| *v).unwrap_or(5);
            let max_volume = cluster.last().map(|(v, _)| *v).unwrap_or(min_volume);
            let mut weighted_sum: u64 = 0;
            let mut hit_count: usize = 0;
            for (vol, indices) in &cluster {
                weighted_sum += u64::from(*vol) * (indices.len() as u64);
                hit_count += indices.len();
            }
            let tier_volume = ((weighted_sum as f64 / hit_count.max(1) as f64).round() as u8)
                .clamp(5, 100);
            let midi_velocity = volume_to_midi_velocity(tier_volume);
            let filename = format!("vst-{slug}-v{tier_volume}.wav");
            let job_index = jobs.len();

            jobs.push(Vst3BounceJob {
                filename: filename.clone(),
                pitch,
                midi_velocity,
                hit_count,
                min_volume,
                max_volume,
            });

            for (_, indices) in cluster {
                for hit_index in indices {
                    let hit = &hits[hit_index];
                    assignments.push(Vst3HitAssignment {
                        hit_index,
                        time_ms: hit.time_ms,
                        pitch: hit.pitch,
                        original_volume: hit.volume,
                        tier_volume,
                        midi_velocity,
                        filename: filename.clone(),
                        job_index,
                    });
                }
            }
        }
    }

    assignments.sort_by_key(|a| a.hit_index);
    let saved_files_vs_exact = exact_file_count.saturating_sub(jobs.len());

    Vst3DrumExportPlan {
        mode,
        jobs,
        assignments,
        saved_files_vs_exact,
    }
}

/// Errors returned by [`Vst3Manager`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vst3Error {
    /// The isolated `dawsu-vst3-host` sidecar binary is not built or installed yet.
    SidecarNotBuilt { searched: Vec<PathBuf> },
    /// No loaded plug-in instance matches the requested ID.
    PluginNotFound(String),
    /// Scanning a plug-in failed.
    Scan(ScanError),
    /// Communicating with or spawning the sidecar host failed.
    Host(HostError),
    /// Bouncing audio to a WAV file failed.
    Bounce(String),
}

impl fmt::Display for Vst3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SidecarNotBuilt { searched } => {
                if searched.is_empty() {
                    write!(
                        f,
                        "VST3 host sidecar is not built yet. Build dawsu-vst3-host to use VST3 plug-ins."
                    )
                } else {
                    let paths: Vec<String> =
                        searched.iter().map(|p| p.display().to_string()).collect();
                    write!(
                        f,
                        "VST3 host sidecar is not built yet (looked in: {}).",
                        paths.join(", ")
                    )
                }
            }
            Self::PluginNotFound(id) => write!(f, "VST3 plug-in instance '{id}' is not loaded."),
            Self::Scan(err) => write!(f, "{err}"),
            Self::Host(err) => write!(f, "{err}"),
            Self::Bounce(msg) => write!(f, "Failed to bounce WAV: {msg}"),
        }
    }
}

impl std::error::Error for Vst3Error {}

impl From<ScanError> for Vst3Error {
    fn from(err: ScanError) -> Self {
        Self::Scan(err)
    }
}

impl From<HostError> for Vst3Error {
    fn from(err: HostError) -> Self {
        Self::Host(err)
    }
}

struct LoadedInstance {
    id: String,
    role: PluginRole,
    plugin_path: PathBuf,
    plugin_uid: Option<String>,
    editor_open: bool,
    last_preview_note: Option<MidiPreviewNote>,
    session: HostSession<ProcessTransport>,
}

impl LoadedInstance {
    fn summary(&mut self) -> LoadedPluginSummary {
        let alive = !self.session.is_crashed();
        LoadedPluginSummary {
            id: self.id.clone(),
            name: self.session.plugin_name().to_string(),
            path: self.plugin_path.clone(),
            uid: self.plugin_uid.clone(),
            role: self.role,
            editor_open: self.editor_open && alive,
            alive,
            last_preview_note: self.last_preview_note,
        }
    }
}

/// Manages VST3 plug-in discovery, isolated sidecar sessions, preview notes, and WAV bouncing.
pub struct Vst3Manager {
    host_executable: Option<PathBuf>,
    search_dirs: Vec<PathBuf>,
    scanned_plugins: Vec<Vst3PluginInfo>,
    instances: BTreeMap<String, LoadedInstance>,
    next_instance_id: u64,
}

impl Default for Vst3Manager {
    fn default() -> Self {
        Self::new()
    }
}

impl Vst3Manager {
    /// Creates a manager configured with the platform's default VST3 search folders.
    pub fn new() -> Self {
        Self::with_search_dirs(default_vst3_dirs())
    }

    /// Creates a manager using explicit VST3 search folders.
    pub fn with_search_dirs(dirs: Vec<PathBuf>) -> Self {
        Self {
            host_executable: None,
            search_dirs: dirs,
            scanned_plugins: Vec::new(),
            instances: BTreeMap::new(),
            next_instance_id: 1,
        }
    }

    /// Overrides the path to the `dawsu-vst3-host` sidecar executable.
    pub fn with_host_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.host_executable = Some(path.into());
        self
    }

    pub fn search_dirs(&self) -> &[PathBuf] {
        &self.search_dirs
    }

    pub fn set_search_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.search_dirs = dirs;
    }

    /// Returns the resolved sidecar executable path, or [`Vst3Error::SidecarNotBuilt`] if
    /// the sidecar binary is not present on disk yet.
    pub fn host_executable(&self) -> Result<PathBuf, Vst3Error> {
        let mut searched = Vec::new();
        if let Some(explicit) = &self.host_executable {
            if explicit.is_file() {
                return Ok(explicit.clone());
            }
            searched.push(explicit.clone());
            return Err(Vst3Error::SidecarNotBuilt { searched });
        }

        if let Some(env_path) = std::env::var_os("DAWSU_VST3_HOST") {
            let candidate = PathBuf::from(env_path);
            if candidate.is_file() {
                return Ok(candidate);
            }
            searched.push(candidate);
        }

        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            for name in sidecar_binary_names() {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
                searched.push(candidate);
            }
        }

        Err(Vst3Error::SidecarNotBuilt { searched })
    }

    /// Whether the `dawsu-vst3-host` sidecar executable is available on disk.
    pub fn is_sidecar_available(&self) -> bool {
        self.host_executable().is_ok()
    }

    /// Scans the manager's configured VST3 search directories.
    pub fn scan_plugins(&mut self) -> Result<Vst3ScanReport, Vst3Error> {
        let dirs = self.search_dirs.clone();
        self.scan_in_dirs(&dirs)
    }

    /// Discovers `.vst3` bundles/files in `dirs` and queries their classes via the sidecar.
    ///
    /// If no `.vst3` files exist in `dirs`, returns an empty report even when the sidecar
    /// binary is not built yet. If `.vst3` files are found but the sidecar is missing,
    /// returns [`Vst3Error::SidecarNotBuilt`].
    pub fn scan_in_dirs(&mut self, dirs: &[PathBuf]) -> Result<Vst3ScanReport, Vst3Error> {
        let discovered_paths = discover_vst3_paths_in(dirs);
        if discovered_paths.is_empty() {
            self.scanned_plugins.clear();
            return Ok(Vst3ScanReport {
                discovered_paths,
                plugins: Vec::new(),
                skipped: Vec::new(),
                sidecar_available: self.is_sidecar_available(),
            });
        }

        let host_exe = self.host_executable()?;
        let mut plugins = Vec::new();
        let mut skipped = Vec::new();
        for path in &discovered_paths {
            match scan_plugin(&host_exe, path) {
                Ok(found) => plugins.extend(found),
                Err(err) => skipped.push(ScanSkip {
                    path: path.clone(),
                    reason: err.to_string(),
                }),
            }
        }
        self.scanned_plugins = plugins.clone();
        Ok(Vst3ScanReport {
            discovered_paths,
            plugins,
            skipped,
            sidecar_available: true,
        })
    }

    /// Returns the plug-ins cached from the most recent scan.
    pub fn scanned_plugins(&self) -> &[Vst3PluginInfo] {
        &self.scanned_plugins
    }

    /// Loads a VST3 instrument in a dedicated sidecar process.
    pub fn load_instrument(
        &mut self,
        plugin_path: impl Into<PathBuf>,
        plugin_uid: Option<String>,
    ) -> Result<LoadedPluginSummary, Vst3Error> {
        self.load_plugin(plugin_path, plugin_uid, PluginRole::Instrument, false)
    }

    /// Loads a VST3 audio effect in a dedicated sidecar process.
    pub fn load_effect(
        &mut self,
        plugin_path: impl Into<PathBuf>,
        plugin_uid: Option<String>,
    ) -> Result<LoadedPluginSummary, Vst3Error> {
        self.load_plugin(plugin_path, plugin_uid, PluginRole::Effect, false)
    }

    /// Loads a VST3 plug-in (instrument or effect, live or offline) in a dedicated sidecar.
    pub fn load_plugin(
        &mut self,
        plugin_path: impl Into<PathBuf>,
        plugin_uid: Option<String>,
        role: PluginRole,
        offline: bool,
    ) -> Result<LoadedPluginSummary, Vst3Error> {
        let host_executable = self.host_executable()?;
        let plugin_path = plugin_path.into();
        let id_num = self.next_instance_id;
        let id = format!("vst3-{id_num}");
        let pid = std::process::id();
        let config = Vst3HostConfig {
            host_executable,
            shm_name: format!("dawsu_vst3_{pid}_{id_num}"),
            parent_pid: pid,
            plugin_path: plugin_path.clone(),
            plugin_uid: plugin_uid.clone(),
            offline,
        };
        let session = HostSession::spawn(&config)?;
        self.next_instance_id += 1;

        let mut instance = LoadedInstance {
            id: id.clone(),
            role,
            plugin_path,
            plugin_uid,
            editor_open: false,
            last_preview_note: None,
            session,
        };
        let summary = instance.summary();
        self.instances.insert(id, instance);
        Ok(summary)
    }

    /// Shuts down and removes a loaded VST3 plug-in instance.
    pub fn unload_plugin(&mut self, instance_id: &str) -> Result<(), Vst3Error> {
        let mut instance = self
            .instances
            .remove(instance_id)
            .ok_or_else(|| Vst3Error::PluginNotFound(instance_id.to_string()))?;
        if !instance.session.is_crashed() {
            let _ = instance.session.quit();
        }
        Ok(())
    }

    /// Opens the plug-in's native VST3 editor window via the sidecar.
    pub fn open_editor(&mut self, instance_id: &str) -> Result<(), Vst3Error> {
        let instance = self
            .instances
            .get_mut(instance_id)
            .ok_or_else(|| Vst3Error::PluginNotFound(instance_id.to_string()))?;
        instance.session.show()?;
        instance.editor_open = true;
        Ok(())
    }

    /// Hides the plug-in's native VST3 editor window via the sidecar.
    pub fn close_editor(&mut self, instance_id: &str) -> Result<(), Vst3Error> {
        let instance = self
            .instances
            .get_mut(instance_id)
            .ok_or_else(|| Vst3Error::PluginNotFound(instance_id.to_string()))?;
        instance.session.hide()?;
        instance.editor_open = false;
        Ok(())
    }

    /// Reads the base64-encoded preset/component state from a loaded plug-in.
    pub fn get_plugin_state(&mut self, instance_id: &str) -> Result<String, Vst3Error> {
        let instance = self
            .instances
            .get_mut(instance_id)
            .ok_or_else(|| Vst3Error::PluginNotFound(instance_id.to_string()))?;
        Ok(instance.session.get_state()?)
    }

    /// Restores a base64-encoded preset/component state on a loaded plug-in.
    pub fn set_plugin_state(
        &mut self,
        instance_id: &str,
        base64_state: &str,
    ) -> Result<(), Vst3Error> {
        let instance = self
            .instances
            .get_mut(instance_id)
            .ok_or_else(|| Vst3Error::PluginNotFound(instance_id.to_string()))?;
        instance.session.set_state(base64_state)?;
        Ok(())
    }

    /// Records a MIDI preview note for a loaded VST3 instance after verifying it is alive.
    pub fn send_preview_note(
        &mut self,
        instance_id: &str,
        note: MidiPreviewNote,
    ) -> Result<(), Vst3Error> {
        let instance = self
            .instances
            .get_mut(instance_id)
            .ok_or_else(|| Vst3Error::PluginNotFound(instance_id.to_string()))?;
        if instance.session.is_crashed() {
            return Err(Vst3Error::Host(HostError::Crashed(
                "VST3 plug-in host is no longer running".to_string(),
            )));
        }
        instance.last_preview_note = Some(note.clamped());
        Ok(())
    }

    /// Converts a [`MidiPreviewNote`] into a Note On / Note Off [`MidiEvent`] pair for rendering.
    pub fn preview_note_to_midi_events(note: MidiPreviewNote, sample_rate: u32) -> [MidiEvent; 2] {
        let clamped = note.clamped();
        let rate = u64::from(sample_rate.max(1));
        let off_frame = (u64::from(clamped.duration_ms) * rate / 1000).max(1);
        [
            MidiEvent {
                frame: 0,
                status: 0x90 | clamped.channel,
                data1: clamped.note,
                data2: clamped.velocity,
            },
            MidiEvent {
                frame: off_frame,
                status: 0x80 | clamped.channel,
                data1: clamped.note,
                data2: 0,
            },
        ]
    }

    /// Renders a MIDI event sequence through a loaded VST3 instance to a WAV file via the sidecar.
    pub fn bounce_midi_to_wav(
        &mut self,
        instance_id: &str,
        events: Vec<MidiEvent>,
        out_path: impl AsRef<Path>,
        options: &BounceOptions,
    ) -> Result<BounceOutput, Vst3Error> {
        let instance = self
            .instances
            .get_mut(instance_id)
            .ok_or_else(|| Vst3Error::PluginNotFound(instance_id.to_string()))?;
        let path = out_path.as_ref().to_path_buf();
        let request = RenderRequest {
            out_path: path.clone(),
            max_frames: options.max_frames,
            quiet_frames: options.quiet_frames,
            threshold: f64::from(options.silence_threshold),
            events,
        };
        let rendered = instance.session.render(&request)?;
        Ok(BounceOutput {
            path,
            frames: rendered.frames as usize,
            sample_rate: options.sample_rate,
            peak: rendered.peak,
            truncated: rendered.truncated,
        })
    }

    /// Executes a [`Vst3DrumExportPlan`] by bouncing every [`Vst3BounceJob`] through the
    /// loaded VST3 instrument into `out_dir/<filename>`.
    ///
    /// Gracefully returns [`Vst3Error::SidecarNotBuilt`] if the `dawsu-vst3-host` sidecar
    /// binary has not been compiled yet.
    pub fn execute_vst3_drum_export(
        &mut self,
        instance_id: &str,
        plan: &Vst3DrumExportPlan,
        out_dir: impl AsRef<Path>,
        sample_rate: u32,
    ) -> Result<Vec<BounceOutput>, Vst3Error> {
        let _ = self.host_executable()?;
        if !self.instances.contains_key(instance_id) {
            return Err(Vst3Error::PluginNotFound(instance_id.to_string()));
        }
        let out_dir = out_dir.as_ref();
        std::fs::create_dir_all(out_dir)
            .map_err(|err| Vst3Error::Bounce(format!("create output dir failed: {err}")))?;

        let options = BounceOptions {
            sample_rate,
            ..BounceOptions::default()
        };
        let mut outputs = Vec::with_capacity(plan.jobs.len());
        for job in &plan.jobs {
            let events = Self::preview_note_to_midi_events(
                MidiPreviewNote {
                    channel: 9,
                    note: job.pitch,
                    velocity: job.midi_velocity,
                    duration_ms: 250,
                },
                sample_rate,
            )
            .to_vec();
            let out_path = out_dir.join(&job.filename);
            let output = self.bounce_midi_to_wav(instance_id, events, out_path, &options)?;
            outputs.push(output);
        }
        Ok(outputs)
    }

    /// Bounces interleaved stereo `f32` audio directly to a WAV file using `bounce.rs`.
    pub fn bounce_audio_to_wav(
        interleaved_stereo: &[f32],
        out_path: impl AsRef<Path>,
        options: &BounceOptions,
    ) -> Result<BounceOutput, Vst3Error> {
        let path = out_path.as_ref().to_path_buf();
        let threshold = (options.silence_threshold > 0.0).then_some(options.silence_threshold);
        let trimmed = match threshold {
            Some(t) => trim_silence_tail(interleaved_stereo, t),
            None => &interleaved_stereo[..(interleaved_stereo.len() & !1)],
        };
        let peak = trimmed
            .iter()
            .copied()
            .filter(|s| s.is_finite())
            .fold(0.0_f32, |acc, s| acc.max(s.abs()));

        let frames = match options.format {
            BounceFormat::Pcm16 => write_wav_i16(&path, trimmed, options.sample_rate, None),
            BounceFormat::Float32 => write_wav_f32(&path, trimmed, options.sample_rate, None),
        }
        .map_err(|err| Vst3Error::Bounce(err.to_string()))?;

        Ok(BounceOutput {
            path,
            frames,
            sample_rate: options.sample_rate,
            peak,
            truncated: false,
        })
    }

    /// Lists all currently loaded VST3 plug-in instances and their live status.
    pub fn list_loaded(&mut self) -> Vec<LoadedPluginSummary> {
        self.instances
            .values_mut()
            .map(LoadedInstance::summary)
            .collect()
    }

    /// Number of currently loaded VST3 plug-in instances.
    pub fn loaded_count(&self) -> usize {
        self.instances.len()
    }
}

fn sidecar_binary_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["dawsu-vst3-host.exe", "dawsu_vst3_host.exe"]
    }
    #[cfg(not(target_os = "windows"))]
    {
        &["dawsu-vst3-host", "dawsu_vst3_host"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dawsu-app-vst3-{label}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scan_empty_and_missing_paths_succeeds_without_sidecar() {
        let empty_dir = make_temp_dir("empty");
        let missing_dir = empty_dir.join("does-not-exist");
        let missing_sidecar = empty_dir.join("dawsu-vst3-host.exe");

        let mut manager = Vst3Manager::with_search_dirs(vec![missing_dir, empty_dir.clone()])
            .with_host_executable(&missing_sidecar);

        assert!(!manager.is_sidecar_available());
        let report = manager.scan_plugins().unwrap();
        let _ = fs::remove_dir_all(&empty_dir);

        assert!(report.discovered_paths.is_empty());
        assert!(report.plugins.is_empty());
        assert!(report.skipped.is_empty());
        assert!(!report.sidecar_available);
        assert!(manager.scanned_plugins().is_empty());
    }

    #[test]
    fn graceful_missing_sidecar_error_on_scan_and_load() {
        let root = make_temp_dir("missing-sidecar");
        let plugin_path = root.join("Vital.vst3");
        fs::write(&plugin_path, b"placeholder").unwrap();
        let missing_sidecar = root.join("dawsu-vst3-host.exe");

        let mut manager = Vst3Manager::with_search_dirs(vec![root.clone()])
            .with_host_executable(&missing_sidecar);

        let scan_err = manager.scan_plugins().unwrap_err();
        assert!(
            matches!(scan_err, Vst3Error::SidecarNotBuilt { ref searched } if searched == &[missing_sidecar.clone()]),
            "unexpected scan error: {scan_err:?}"
        );
        assert!(scan_err.to_string().contains("not built yet"));

        let load_inst_err = manager.load_instrument(&plugin_path, None).unwrap_err();
        assert!(matches!(load_inst_err, Vst3Error::SidecarNotBuilt { .. }));

        let load_fx_err = manager.load_effect(&plugin_path, None).unwrap_err();
        assert!(matches!(load_fx_err, Vst3Error::SidecarNotBuilt { .. }));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn state_listing_and_missing_instance_errors() {
        let mut manager = Vst3Manager::with_search_dirs(Vec::new())
            .with_host_executable(PathBuf::from("nonexistent-dawsu-vst3-host"));

        assert_eq!(manager.loaded_count(), 0);
        assert!(manager.list_loaded().is_empty());
        assert!(manager.scanned_plugins().is_empty());

        assert!(matches!(
            manager.open_editor("vst3-99"),
            Err(Vst3Error::PluginNotFound(id)) if id == "vst3-99"
        ));
        assert!(matches!(
            manager.close_editor("vst3-99"),
            Err(Vst3Error::PluginNotFound(_))
        ));
        assert!(matches!(
            manager.unload_plugin("vst3-99"),
            Err(Vst3Error::PluginNotFound(_))
        ));
        assert!(matches!(
            manager.get_plugin_state("vst3-99"),
            Err(Vst3Error::PluginNotFound(_))
        ));
        assert!(matches!(
            manager.send_preview_note(
                "vst3-99",
                MidiPreviewNote {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                    duration_ms: 250,
                }
            ),
            Err(Vst3Error::PluginNotFound(_))
        ));
        assert!(matches!(
            manager.bounce_midi_to_wav(
                "vst3-99",
                Vec::new(),
                Path::new("out.wav"),
                &BounceOptions::default()
            ),
            Err(Vst3Error::PluginNotFound(_))
        ));
    }

    #[test]
    fn preview_note_converts_to_clamped_midi_events() {
        let events = Vst3Manager::preview_note_to_midi_events(
            MidiPreviewNote {
                channel: 20, // clamps to 15
                note: 200,   // clamps to 127
                velocity: 127,
                duration_ms: 500,
            },
            48_000,
        );
        assert_eq!(
            events[0],
            MidiEvent {
                frame: 0,
                status: 0x9F,
                data1: 127,
                data2: 127,
            }
        );
        assert_eq!(
            events[1],
            MidiEvent {
                frame: 24_000,
                status: 0x8F,
                data1: 127,
                data2: 0,
            }
        );
    }

    #[test]
    fn bounce_audio_to_wav_writes_trimmed_pcm16_and_float32() {
        let root = make_temp_dir("bounce");
        let stereo = [
            0.5_f32, -0.5, // loud frame
            0.25, -0.25, // quiet frame
            0.0, 0.0, // trailing silence trimmed
        ];

        let pcm_path = root.join("hit_pcm16.wav");
        let pcm_out = Vst3Manager::bounce_audio_to_wav(
            &stereo,
            &pcm_path,
            &BounceOptions {
                sample_rate: 48_000,
                format: BounceFormat::Pcm16,
                silence_threshold: 0.001,
                ..BounceOptions::default()
            },
        )
        .unwrap();
        assert_eq!(pcm_out.frames, 2);
        assert_eq!(pcm_out.sample_rate, 48_000);
        assert!((pcm_out.peak - 0.5).abs() < 1e-6);
        assert!(!pcm_out.truncated);
        assert!(pcm_path.is_file());

        let f32_path = root.join("hit_f32.wav");
        let f32_out = Vst3Manager::bounce_audio_to_wav(
            &stereo,
            &f32_path,
            &BounceOptions {
                sample_rate: 44_100,
                format: BounceFormat::Float32,
                silence_threshold: 0.001,
                ..BounceOptions::default()
            },
        )
        .unwrap();
        assert_eq!(f32_out.frames, 2);
        assert_eq!(f32_out.sample_rate, 44_100);
        assert!(f32_path.is_file());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tiered_drum_export_groups_nearby_volumes_with_weighted_average() {
        let volumes = [25_u8, 30, 35, 70, 75, 95, 100];
        let hits: Vec<Vst3DrumHit> = volumes
            .iter()
            .enumerate()
            .map(|(i, &volume)| Vst3DrumHit {
                time_ms: (i as f64) * 250.0,
                pitch: 36,
                volume,
                name: "Kick".to_string(),
            })
            .collect();

        let plan = plan_vst3_drum_export(&hits, VelocityExportMode::default());
        assert_eq!(plan.mode, VelocityExportMode::Tiered { max_interval: 20 });
        assert_eq!(plan.jobs.len(), 3);
        assert_eq!(plan.saved_files_vs_exact, 4);
        assert_eq!(plan.assignments.len(), hits.len());

        // Tier 1: [25, 30, 35] -> avg 30 -> MIDI vel round(30 * 127 / 100) = 38
        assert_eq!(
            plan.jobs[0],
            Vst3BounceJob {
                filename: "vst-kick-v30.wav".to_string(),
                pitch: 36,
                midi_velocity: 38,
                hit_count: 3,
                min_volume: 25,
                max_volume: 35,
            }
        );
        // Tier 2: [70, 75] -> avg 72.5 -> 73 -> MIDI vel round(73 * 127 / 100) = 93
        assert_eq!(
            plan.jobs[1],
            Vst3BounceJob {
                filename: "vst-kick-v73.wav".to_string(),
                pitch: 36,
                midi_velocity: 93,
                hit_count: 2,
                min_volume: 70,
                max_volume: 75,
            }
        );
        // Tier 3: [95, 100] -> avg 97.5 -> 98 -> MIDI vel round(98 * 127 / 100) = 124
        assert_eq!(
            plan.jobs[2],
            Vst3BounceJob {
                filename: "vst-kick-v98.wav".to_string(),
                pitch: 36,
                midi_velocity: 124,
                hit_count: 2,
                min_volume: 95,
                max_volume: 100,
            }
        );

        assert_eq!(plan.assignments[0].filename, "vst-kick-v30.wav");
        assert_eq!(plan.assignments[3].filename, "vst-kick-v73.wav");
        assert_eq!(plan.assignments[6].filename, "vst-kick-v98.wav");

        // Verify weighted average favors volume with more hits: three 80s + one 60 => (240 + 60)/4 = 75.
        let weighted_hits = vec![
            Vst3DrumHit { time_ms: 0.0, pitch: 36, volume: 60, name: "Kick".into() },
            Vst3DrumHit { time_ms: 100.0, pitch: 36, volume: 80, name: "Kick".into() },
            Vst3DrumHit { time_ms: 200.0, pitch: 36, volume: 80, name: "Kick".into() },
            Vst3DrumHit { time_ms: 300.0, pitch: 36, volume: 80, name: "Kick".into() },
        ];
        let weighted_plan = plan_vst3_drum_export(&weighted_hits, VelocityExportMode::Tiered { max_interval: 20 });
        assert_eq!(weighted_plan.jobs.len(), 1);
        assert_eq!(weighted_plan.jobs[0].filename, "vst-kick-v75.wav");
        assert_eq!(weighted_plan.jobs[0].midi_velocity, 95);
        assert_eq!(weighted_plan.jobs[0].hit_count, 4);
        assert_eq!(weighted_plan.saved_files_vs_exact, 1);
    }

    #[test]
    fn exact_per_volume_creates_one_job_per_distinct_volume() {
        let volumes = [25_u8, 30, 35, 70, 75, 95, 100, 75];
        let hits: Vec<Vst3DrumHit> = volumes
            .iter()
            .enumerate()
            .map(|(i, &volume)| Vst3DrumHit {
                time_ms: (i as f64) * 125.0,
                pitch: 38,
                volume,
                name: "Snare".to_string(),
            })
            .collect();

        let plan = plan_vst3_drum_export(&hits, VelocityExportMode::ExactPerVolume);
        assert_eq!(plan.mode, VelocityExportMode::ExactPerVolume);
        assert_eq!(plan.jobs.len(), 7);
        assert_eq!(plan.saved_files_vs_exact, 0);
        assert_eq!(plan.assignments.len(), 8);
        assert_eq!(plan.assignments[4].filename, "vst-snare-v75.wav");
        assert_eq!(plan.assignments[7].filename, "vst-snare-v75.wav");
        assert_eq!(
            plan.jobs.iter().find(|j| j.filename == "vst-snare-v75.wav").unwrap().hit_count,
            2
        );
    }

    #[test]
    fn different_pitches_never_merge_at_identical_volumes() {
        let hits = vec![
            Vst3DrumHit { time_ms: 0.0, pitch: 36, volume: 70, name: "Kick".into() },
            Vst3DrumHit { time_ms: 100.0, pitch: 36, volume: 75, name: "Kick".into() },
            Vst3DrumHit { time_ms: 200.0, pitch: 38, volume: 70, name: "Snare".into() },
            Vst3DrumHit { time_ms: 300.0, pitch: 38, volume: 75, name: "Snare".into() },
        ];

        let plan = plan_vst3_drum_export(&hits, VelocityExportMode::Tiered { max_interval: 20 });
        assert_eq!(plan.jobs.len(), 2);
        assert_eq!(plan.saved_files_vs_exact, 2);
        assert_eq!(plan.jobs[0].pitch, 36);
        assert_eq!(plan.jobs[0].filename, "vst-kick-v73.wav");
        assert_eq!(plan.jobs[1].pitch, 38);
        assert_eq!(plan.jobs[1].filename, "vst-snare-v73.wav");

        // Distinct pitches sharing the same display name still produce distinct filenames.
        let same_name_hits = vec![
            Vst3DrumHit { time_ms: 0.0, pitch: 45, volume: 80, name: "Tom".into() },
            Vst3DrumHit { time_ms: 100.0, pitch: 47, volume: 80, name: "Tom".into() },
        ];
        let same_name_plan =
            plan_vst3_drum_export(&same_name_hits, VelocityExportMode::Tiered { max_interval: 20 });
        assert_eq!(same_name_plan.jobs.len(), 2);
        assert_ne!(same_name_plan.jobs[0].filename, same_name_plan.jobs[1].filename);
        assert_eq!(same_name_plan.jobs[0].filename, "vst-tom-p45-v80.wav");
        assert_eq!(same_name_plan.jobs[1].filename, "vst-tom-p47-v80.wav");
    }

    #[test]
    fn execute_vst3_drum_export_reports_sidecar_not_built_gracefully() {
        let root = make_temp_dir("drum-export-no-sidecar");
        let missing_sidecar = root.join("dawsu-vst3-host.exe");
        let mut manager = Vst3Manager::with_search_dirs(Vec::new())
            .with_host_executable(&missing_sidecar);

        let hits = vec![Vst3DrumHit {
            time_ms: 0.0,
            pitch: 36,
            volume: 85,
            name: "Kick".into(),
        }];
        let plan = plan_vst3_drum_export(&hits, VelocityExportMode::default());
        let err = manager
            .execute_vst3_drum_export("vst3-1", &plan, &root, 48_000)
            .unwrap_err();
        assert!(matches!(err, Vst3Error::SidecarNotBuilt { .. }));

        let _ = fs::remove_dir_all(&root);
    }
}
