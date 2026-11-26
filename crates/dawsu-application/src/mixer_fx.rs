//! Per-map mixer insert VST3 effects, weighted-DAG PDC routing, A/B comparison,
//! Single-Window Swap + Pin management, undo/redo, and session persistence.

use crate::project;
use crate::vst3::Vst3Manager;
use crate::{Application, lane_name};
use dawsu_audio::{
    AudioEffectProcessor, FxInsertSpec, FxStripSpec, MixerFxGraph, PdcNode, STRIP_DRY,
    STRIP_HITSOUNDS, STRIP_LANE_BASE, STRIP_MASTER, STRIP_SONG, compute_pdc,
};
use dawsu_audio_instruments::ShmChannel;
use dawsu_audio_instruments::host::{CTX_FLAG_PLAYING, RESTART_LATENCY_CHANGED, ShmProcessContext};
use dawsu_foundation::SampleIdentity;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_INSERTS_PER_STRIP: usize = 8;
pub const MAX_MIXER_FX_UNDO: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbBank {
    #[default]
    A,
    B,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxSlotConfig {
    pub slot_id: String,
    pub plugin_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_uid: Option<String>,
    pub name: String,
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default = "default_wet_dry")]
    pub wet_dry: f32,
    #[serde(default)]
    pub active_bank: AbBank,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_a_b64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_b_b64: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

fn default_wet_dry() -> f32 {
    1.0
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MixerFxSession {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub strips: BTreeMap<String, Vec<FxSlotConfig>>,
}

impl MixerFxSession {
    pub fn is_empty(&self) -> bool {
        self.strips.values().all(Vec::is_empty)
    }

    fn normalize(&mut self) {
        self.strips.retain(|key, slots| {
            normalize_strip_key(key).is_some() && !slots.is_empty()
        });
        for slots in self.strips.values_mut() {
            slots.truncate(MAX_INSERTS_PER_STRIP);
            for slot in slots {
                slot.wet_dry = if slot.wet_dry.is_finite() {
                    slot.wet_dry.clamp(0.0, 1.0)
                } else {
                    1.0
                };
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableEffectView {
    pub name: String,
    pub vendor: String,
    pub uid: Option<String>,
    pub path: PathBuf,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxSlotView {
    pub slot_id: String,
    pub plugin_path: String,
    pub plugin_uid: Option<String>,
    pub name: String,
    pub bypassed: bool,
    pub wet_dry: f32,
    pub active_bank: AbBank,
    pub has_state_a: bool,
    pub has_state_b: bool,
    pub pinned: bool,
    pub editor_open: bool,
    pub alive: bool,
    pub available: bool,
    pub latency_samples: u32,
    pub late_blocks: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerFxSnapshot {
    pub revision: u64,
    pub total_pdc_samples: u32,
    pub sample_rate: u32,
    pub can_undo: bool,
    pub can_redo: bool,
    pub sidecar_available: bool,
    pub available_effects: Vec<AvailableEffectView>,
    pub strips: BTreeMap<String, Vec<FxSlotView>>,
}

pub(crate) struct FxSlotRuntime {
    bypassed: AtomicBool,
    wet_dry_bits: AtomicU32,
    latency_samples: AtomicU32,
    late_blocks: AtomicU64,
}

impl FxSlotRuntime {
    fn new(bypassed: bool, wet_dry: f32, latency: u32) -> Arc<Self> {
        Arc::new(Self {
            bypassed: AtomicBool::new(bypassed),
            wet_dry_bits: AtomicU32::new(wet_dry.clamp(0.0, 1.0).to_bits()),
            latency_samples: AtomicU32::new(latency),
            late_blocks: AtomicU64::new(0),
        })
    }

    fn set_bypassed(&self, bypassed: bool) {
        self.bypassed.store(bypassed, Ordering::Release);
    }

    fn set_wet_dry(&self, wet_dry: f32) {
        self.wet_dry_bits
            .store(wet_dry.clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }

    fn latency(&self) -> u32 {
        self.latency_samples.load(Ordering::Acquire)
    }

    fn late_blocks(&self) -> u64 {
        self.late_blocks.load(Ordering::Relaxed)
    }
}

struct ShmEffectProcessor {
    shm: ShmChannel,
    runtime: Arc<FxSlotRuntime>,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    last_latency: u32,
    pending_latency_changed: bool,
}

impl ShmEffectProcessor {
    fn new(shm: ShmChannel, runtime: Arc<FxSlotRuntime>) -> Self {
        let initial_lat = shm.latency_samples();
        runtime.latency_samples.store(initial_lat, Ordering::Release);
        Self {
            shm,
            runtime,
            in_l: vec![0.0; 512],
            in_r: vec![0.0; 512],
            last_latency: initial_lat,
            pending_latency_changed: false,
        }
    }
}

impl AudioEffectProcessor for ShmEffectProcessor {
    fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        sample_rate: u32,
        is_playing: bool,
        position_ms: f64,
        speed: f64,
        bypassed: bool,
    ) {
        let frames = left.len().min(right.len()).min(self.in_l.len());
        if frames == 0 {
            return;
        }
        self.in_l[..frames].copy_from_slice(&left[..frames]);
        self.in_r[..frames].copy_from_slice(&right[..frames]);
        let mut ctx = ShmProcessContext {
            sample_rate: f64::from(sample_rate.max(1)),
            ..ShmProcessContext::default()
        };
        if is_playing {
            ctx.state_flags |= CTX_FLAG_PLAYING;
        }
        ctx.tempo = 120.0 * speed.max(0.25);
        ctx.project_time_samples = (position_ms * ctx.sample_rate / 1000.0).round() as i64;
        let ok = self.shm.process_audio_block(
            ctx.sample_rate,
            Some(&ctx),
            &[],
            Some(&self.in_l[..frames]),
            Some(&self.in_r[..frames]),
            &mut left[..frames],
            &mut right[..frames],
            bypassed,
        );
        if !ok {
            self.runtime
                .late_blocks
                .store(self.shm.late_blocks(), Ordering::Relaxed);
        }
        let lat = self.shm.latency_samples();
        self.runtime.latency_samples.store(lat, Ordering::Release);
        let restart = self.shm.take_restart_flags();
        if (restart & RESTART_LATENCY_CHANGED) != 0 || lat != self.last_latency {
            self.last_latency = lat;
            self.pending_latency_changed = true;
        }
    }

    fn latency_samples(&self) -> u32 {
        self.runtime.latency_samples.load(Ordering::Acquire)
    }

    fn wet_dry(&self) -> f32 {
        f32::from_bits(self.runtime.wet_dry_bits.load(Ordering::Relaxed))
    }

    fn is_bypassed(&self) -> bool {
        self.runtime.bypassed.load(Ordering::Acquire)
    }

    fn take_latency_changed(&mut self) -> bool {
        std::mem::take(&mut self.pending_latency_changed)
    }
}

struct SharedSlotProcessor {
    inner: Arc<Mutex<Box<dyn AudioEffectProcessor>>>,
    runtime: Arc<FxSlotRuntime>,
}

impl AudioEffectProcessor for SharedSlotProcessor {
    fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        sample_rate: u32,
        is_playing: bool,
        position_ms: f64,
        speed: f64,
        bypassed: bool,
    ) {
        if let Ok(mut guard) = self.inner.try_lock() {
            guard.process_block(
                left,
                right,
                sample_rate,
                is_playing,
                position_ms,
                speed,
                bypassed,
            );
            let lat = guard.latency_samples();
            self.runtime.latency_samples.store(lat, Ordering::Release);
        }
    }

    fn latency_samples(&self) -> u32 {
        self.runtime.latency_samples.load(Ordering::Acquire)
    }

    fn wet_dry(&self) -> f32 {
        f32::from_bits(self.runtime.wet_dry_bits.load(Ordering::Relaxed))
    }

    fn is_bypassed(&self) -> bool {
        self.runtime.bypassed.load(Ordering::Acquire)
    }

    fn take_latency_changed(&mut self) -> bool {
        if let Ok(mut guard) = self.inner.try_lock() {
            guard.take_latency_changed()
        } else {
            false
        }
    }

    fn reset(&mut self) {
        if let Ok(mut guard) = self.inner.try_lock() {
            guard.reset();
        }
    }
}

enum LiveBackend {
    Vst3 {
        instance_id: String,
    },
    #[cfg(test)]
    Mock {
        id: String,
        editor_open: bool,
        state: String,
        bypassed: bool,
    },
    Unavailable {
        reason: String,
    },
}

struct LiveSlotState {
    plugin_path: String,
    plugin_uid: Option<String>,
    backend: LiveBackend,
    runtime: Arc<FxSlotRuntime>,
    processor: Option<Arc<Mutex<Box<dyn AudioEffectProcessor>>>>,
}

#[cfg(test)]
pub(crate) trait TestEffectHost: Send {
    fn spawn_effect(
        &mut self,
        plugin_path: &str,
        plugin_uid: Option<&str>,
        runtime: Arc<FxSlotRuntime>,
    ) -> Result<
        (
            String,
            Box<dyn AudioEffectProcessor>,
            String,
        ),
        String,
    >;
    fn set_state(&mut self, id: &str, state_b64: &str) -> Result<(), String>;
    fn get_state(&mut self, id: &str) -> Result<String, String>;
    fn set_bypass(&mut self, id: &str, bypassed: bool) -> Result<(), String>;
    fn open_editor(&mut self, id: &str) -> Result<(), String>;
    fn close_editor(&mut self, id: &str) -> Result<(), String>;
    fn unload(&mut self, id: &str);
}

#[derive(Default)]
pub struct MixerFxManager {
    pub(crate) session: MixerFxSession,
    live: BTreeMap<String, LiveSlotState>,
    lane_strip_ids: BTreeMap<String, u32>,
    next_slot_seq: u64,
    revision: u64,
    undo_stack: VecDeque<MixerFxSession>,
    redo_stack: Vec<MixerFxSession>,
    #[cfg(test)]
    test_host: Option<Box<dyn TestEffectHost>>,
}

impl MixerFxManager {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn session(&self) -> &MixerFxSession {
        &self.session
    }

    pub fn strip_id_for_sample(&self, sample: &SampleIdentity) -> u32 {
        let key = format!("lane:{}", project::lane_key(sample));
        self.lane_strip_ids.get(&key).copied().unwrap_or(STRIP_DRY)
    }

    pub(crate) fn push_undo(&mut self, before: MixerFxSession) {
        if self.undo_stack.len() >= MAX_MIXER_FX_UNDO {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(before);
        self.redo_stack.clear();
    }

    fn rebuild_lane_strip_ids(&mut self) {
        self.lane_strip_ids.clear();
        let mut idx = 0u32;
        for (strip_key, slots) in &self.session.strips {
            if strip_key.starts_with("lane:") && !slots.is_empty() {
                self.lane_strip_ids
                    .insert(strip_key.clone(), STRIP_LANE_BASE.saturating_add(idx));
                idx = idx.saturating_add(1);
            }
        }
    }

    fn next_slot_id(&mut self) -> String {
        self.next_slot_seq += 1;
        format!("fx-{}", self.next_slot_seq)
    }

    fn strip_own_latency(&self, strip_key: &str) -> u32 {
        self.session
            .strips
            .get(strip_key)
            .map_or(0, |slots| {
                slots.iter().fold(0u32, |acc, slot| {
                    let lat = self
                        .live
                        .get(&slot.slot_id)
                        .map_or(0, |live| live.runtime.latency());
                    acc.saturating_add(lat)
                })
            })
    }

    pub fn total_pdc_samples(&self) -> u32 {
        if self.session.is_empty() {
            return 0;
        }
        let mut nodes = vec![
            PdcNode {
                id: STRIP_DRY,
                own_latency: 0,
                outputs: vec![STRIP_HITSOUNDS],
            },
            PdcNode {
                id: STRIP_SONG,
                own_latency: self.strip_own_latency("song"),
                outputs: vec![STRIP_MASTER],
            },
            PdcNode {
                id: STRIP_HITSOUNDS,
                own_latency: self.strip_own_latency("hitsounds"),
                outputs: vec![STRIP_MASTER],
            },
            PdcNode {
                id: STRIP_MASTER,
                own_latency: self.strip_own_latency("master"),
                outputs: vec![],
            },
        ];
        for (strip_key, &strip_id) in &self.lane_strip_ids {
            nodes.push(PdcNode {
                id: strip_id,
                own_latency: self.strip_own_latency(strip_key),
                outputs: vec![STRIP_HITSOUNDS],
            });
        }
        compute_pdc(&nodes).map_or(0, |plan| plan.total_latency)
    }

    fn build_fx_graph(&self, sample_rate: u32) -> Option<Box<MixerFxGraph>> {
        if self.session.is_empty() {
            return None;
        }
        let build_inserts = |strip_key: &str| -> Vec<FxInsertSpec> {
            self.session
                .strips
                .get(strip_key)
                .into_iter()
                .flatten()
                .filter_map(|slot| {
                    let live = self.live.get(&slot.slot_id)?;
                    let inner = live.processor.as_ref()?;
                    Some(FxInsertSpec::new(Box::new(SharedSlotProcessor {
                        inner: Arc::clone(inner),
                        runtime: Arc::clone(&live.runtime),
                    })))
                })
                .collect()
        };

        let mut specs = vec![
            FxStripSpec {
                id: STRIP_DRY,
                inserts: vec![],
                outputs: vec![(STRIP_HITSOUNDS, 1.0)],
            },
            FxStripSpec {
                id: STRIP_SONG,
                inserts: build_inserts("song"),
                outputs: vec![(STRIP_MASTER, 1.0)],
            },
            FxStripSpec {
                id: STRIP_HITSOUNDS,
                inserts: build_inserts("hitsounds"),
                outputs: vec![(STRIP_MASTER, 1.0)],
            },
            FxStripSpec {
                id: STRIP_MASTER,
                inserts: build_inserts("master"),
                outputs: vec![],
            },
        ];
        for (strip_key, &strip_id) in &self.lane_strip_ids {
            specs.push(FxStripSpec {
                id: strip_id,
                inserts: build_inserts(strip_key),
                outputs: vec![(STRIP_HITSOUNDS, 1.0)],
            });
        }

        MixerFxGraph::new(sample_rate, specs).ok().map(Box::new)
    }

    fn spawn_slot_live(
        &mut self,
        slot: &FxSlotConfig,
        vst3: &mut Vst3Manager,
        sample_rate_hz: u32,
    ) {
        let runtime = FxSlotRuntime::new(slot.bypassed, slot.wet_dry, 0);
        let active_state = match slot.active_bank {
            AbBank::A => slot.state_a_b64.as_deref(),
            AbBank::B => slot.state_b_b64.as_deref().or(slot.state_a_b64.as_deref()),
        };

        #[cfg(test)]
        if let Some(host) = &mut self.test_host {
            match host.spawn_effect(
                &slot.plugin_path,
                slot.plugin_uid.as_deref(),
                Arc::clone(&runtime),
            ) {
                Ok((id, proc_box, mut initial_state)) => {
                    if let Some(saved) = active_state
                        && !saved.is_empty()
                    {
                        let _ = host.set_state(&id, saved);
                        initial_state = saved.to_string();
                    }
                    let _ = host.set_bypass(&id, slot.bypassed);
                    runtime
                        .latency_samples
                        .store(proc_box.latency_samples(), Ordering::Release);
                    self.live.insert(
                        slot.slot_id.clone(),
                        LiveSlotState {
                            plugin_path: slot.plugin_path.clone(),
                            plugin_uid: slot.plugin_uid.clone(),
                            backend: LiveBackend::Mock {
                                id,
                                editor_open: false,
                                state: initial_state,
                                bypassed: slot.bypassed,
                            },
                            runtime,
                            processor: Some(Arc::new(Mutex::new(proc_box))),
                        },
                    );
                    return;
                }
                Err(reason) => {
                    self.live.insert(
                        slot.slot_id.clone(),
                        LiveSlotState {
                            plugin_path: slot.plugin_path.clone(),
                            plugin_uid: slot.plugin_uid.clone(),
                            backend: LiveBackend::Unavailable { reason },
                            runtime,
                            processor: None,
                        },
                    );
                    return;
                }
            }
        }

        match vst3.load_effect_at_rate(
            PathBuf::from(&slot.plugin_path),
            slot.plugin_uid.clone(),
            sample_rate_hz,
        ) {
            Ok(summary) => {
                if let Some(saved) = active_state
                    && !saved.is_empty()
                {
                    let _ = vst3.set_plugin_state(&summary.id, saved);
                }
                let _ = vst3.set_plugin_bypass(&summary.id, slot.bypassed);
                let processor = vst3
                    .take_shm(&summary.id)
                    .ok()
                    .flatten()
                    .map(|shm| {
                        let proc_box: Box<dyn AudioEffectProcessor> =
                            Box::new(ShmEffectProcessor::new(shm, Arc::clone(&runtime)));
                        Arc::new(Mutex::new(proc_box))
                    });
                self.live.insert(
                    slot.slot_id.clone(),
                    LiveSlotState {
                        plugin_path: slot.plugin_path.clone(),
                        plugin_uid: slot.plugin_uid.clone(),
                        backend: LiveBackend::Vst3 {
                            instance_id: summary.id,
                        },
                        runtime,
                        processor,
                    },
                );
            }
            Err(err) => {
                self.live.insert(
                    slot.slot_id.clone(),
                    LiveSlotState {
                        plugin_path: slot.plugin_path.clone(),
                        plugin_uid: slot.plugin_uid.clone(),
                        backend: LiveBackend::Unavailable {
                            reason: err.to_string(),
                        },
                        runtime,
                        processor: None,
                    },
                );
            }
        }
    }

    fn unload_slot_live(&mut self, slot_id: &str, vst3: &mut Vst3Manager) {
        let Some(live) = self.live.remove(slot_id) else {
            return;
        };
        match live.backend {
            LiveBackend::Vst3 { instance_id } => {
                let _ = vst3.unload_plugin(&instance_id);
            }
            #[cfg(test)]
            LiveBackend::Mock { id, .. } => {
                if let Some(host) = &mut self.test_host {
                    host.unload(&id);
                }
            }
            LiveBackend::Unavailable { .. } => {}
        }
    }

    fn sync_live_slot_params(
        &mut self,
        slot: &FxSlotConfig,
        vst3: &mut Vst3Manager,
        restore_state: bool,
    ) {
        let Some(live) = self.live.get_mut(&slot.slot_id) else {
            return;
        };
        live.runtime.set_bypassed(slot.bypassed);
        live.runtime.set_wet_dry(slot.wet_dry);
        let desired_state = match slot.active_bank {
            AbBank::A => slot.state_a_b64.as_deref(),
            AbBank::B => slot.state_b_b64.as_deref().or(slot.state_a_b64.as_deref()),
        };
        match &mut live.backend {
            LiveBackend::Vst3 { instance_id } => {
                let _ = vst3.set_plugin_bypass(instance_id, slot.bypassed);
                if restore_state
                    && let Some(state) = desired_state
                    && !state.is_empty()
                {
                    let _ = vst3.set_plugin_state(instance_id, state);
                }
            }
            #[cfg(test)]
            LiveBackend::Mock {
                id,
                state: mock_state,
                bypassed,
                ..
            } => {
                *bypassed = slot.bypassed;
                if let Some(host) = &mut self.test_host {
                    let _ = host.set_bypass(id, slot.bypassed);
                    if restore_state
                        && let Some(saved) = desired_state
                        && !saved.is_empty()
                    {
                        let _ = host.set_state(id, saved);
                        *mock_state = saved.to_string();
                    }
                }
            }
            LiveBackend::Unavailable { .. } => {}
        }
    }

    fn capture_live_state(&mut self, slot_id: &str, vst3: &mut Vst3Manager) -> Option<String> {
        let live = self.live.get(slot_id)?;
        match &live.backend {
            LiveBackend::Vst3 { instance_id } => vst3.get_plugin_state(instance_id).ok(),
            #[cfg(test)]
            LiveBackend::Mock { id, state, .. } => {
                if let Some(host) = &mut self.test_host {
                    host.get_state(id).ok().or_else(|| Some(state.clone()))
                } else {
                    Some(state.clone())
                }
            }
            LiveBackend::Unavailable { .. } => None,
        }
    }

    fn open_slot_editor(&mut self, slot_id: &str, vst3: &mut Vst3Manager) -> Result<(), String> {
        let live = self
            .live
            .get_mut(slot_id)
            .ok_or_else(|| format!("FX slot {slot_id} is not loaded"))?;
        match &mut live.backend {
            LiveBackend::Vst3 { instance_id } => {
                vst3.open_editor(instance_id).map_err(|e| e.to_string())
            }
            #[cfg(test)]
            LiveBackend::Mock { id, editor_open, .. } => {
                if let Some(host) = &mut self.test_host {
                    host.open_editor(id)?;
                }
                *editor_open = true;
                Ok(())
            }
            LiveBackend::Unavailable { reason } => Err(reason.clone()),
        }
    }

    fn close_slot_editor(&mut self, slot_id: &str, vst3: &mut Vst3Manager) -> Result<(), String> {
        let Some(live) = self.live.get_mut(slot_id) else {
            return Ok(());
        };
        match &mut live.backend {
            LiveBackend::Vst3 { instance_id } => {
                vst3.close_editor(instance_id).map_err(|e| e.to_string())
            }
            #[cfg(test)]
            LiveBackend::Mock { id, editor_open, .. } => {
                if let Some(host) = &mut self.test_host {
                    let _ = host.close_editor(id);
                }
                *editor_open = false;
                Ok(())
            }
            LiveBackend::Unavailable { .. } => Ok(()),
        }
    }

    pub(crate) fn reconcile_live(
        &mut self,
        vst3: &mut Vst3Manager,
        sample_rate_hz: u32,
        restore_state: bool,
    ) {
        let mut desired: BTreeMap<String, FxSlotConfig> = BTreeMap::new();
        for slots in self.session.strips.values() {
            for slot in slots {
                desired.insert(slot.slot_id.clone(), slot.clone());
            }
        }

        let existing_ids: Vec<String> = self.live.keys().cloned().collect();
        for id in existing_ids {
            let keep = desired.get(&id).is_some_and(|slot| {
                self.live.get(&id).is_some_and(|live| {
                    live.plugin_path == slot.plugin_path && live.plugin_uid == slot.plugin_uid
                })
            });
            if !keep {
                self.unload_slot_live(&id, vst3);
            }
        }

        for slot in desired.values() {
            if self.live.contains_key(&slot.slot_id) {
                self.sync_live_slot_params(slot, vst3, restore_state);
            } else {
                self.spawn_slot_live(slot, vst3, sample_rate_hz);
            }
        }
        self.rebuild_lane_strip_ids();
    }

    pub(crate) fn unload_all(&mut self, vst3: &mut Vst3Manager) {
        let ids: Vec<String> = self.live.keys().cloned().collect();
        for id in ids {
            self.unload_slot_live(&id, vst3);
        }
        self.session.strips.clear();
        self.lane_strip_ids.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn snapshot(&self, vst3: &Vst3Manager, sample_rate: u32) -> MixerFxSnapshot {
        let available_effects = vst3
            .scanned_plugins()
            .iter()
            .filter(|info| info.is_effect)
            .map(|info| AvailableEffectView {
                name: info.name.clone(),
                vendor: info.vendor.clone(),
                uid: Some(info.uid.clone()),
                path: info.path.clone(),
                category: if info.is_instrument {
                    "Instrument|Fx".to_string()
                } else {
                    "Fx".to_string()
                },
            })
            .collect();

        let mut strips = BTreeMap::new();
        for (strip_key, slots) in &self.session.strips {
            let views: Vec<FxSlotView> = slots
                .iter()
                .map(|slot| {
                    let live = self.live.get(&slot.slot_id);
                    let (editor_open, alive, available, latency_samples, late_blocks, reason) =
                        match live {
                            Some(live) => {
                                let latency = live.runtime.latency();
                                let late = live.runtime.late_blocks();
                                match &live.backend {
                                    LiveBackend::Vst3 { instance_id } => {
                                        let ed = vst3.instance_editor_open(instance_id);
                                        (
                                            ed.unwrap_or(false),
                                            ed.is_some(),
                                            true,
                                            latency,
                                            late,
                                            None,
                                        )
                                    }
                                    #[cfg(test)]
                                    LiveBackend::Mock { editor_open, .. } => {
                                        (*editor_open, true, true, latency, late, None)
                                    }
                                    LiveBackend::Unavailable { reason } => (
                                        false,
                                        false,
                                        false,
                                        latency,
                                        late,
                                        Some(reason.clone()),
                                    ),
                                }
                            }
                            None => (false, false, false, 0, 0, Some("Not initialized".into())),
                        };
                    FxSlotView {
                        slot_id: slot.slot_id.clone(),
                        plugin_path: slot.plugin_path.clone(),
                        plugin_uid: slot.plugin_uid.clone(),
                        name: slot.name.clone(),
                        bypassed: slot.bypassed,
                        wet_dry: slot.wet_dry,
                        active_bank: slot.active_bank,
                        has_state_a: slot.state_a_b64.as_ref().is_some_and(|s| !s.is_empty()),
                        has_state_b: slot.state_b_b64.as_ref().is_some_and(|s| !s.is_empty()),
                        pinned: slot.pinned,
                        editor_open,
                        alive,
                        available,
                        latency_samples,
                        late_blocks,
                        unavailable_reason: reason,
                    }
                })
                .collect();
            if !views.is_empty() {
                strips.insert(strip_key.clone(), views);
            }
        }

        MixerFxSnapshot {
            revision: self.revision,
            total_pdc_samples: self.total_pdc_samples(),
            sample_rate: sample_rate.max(1),
            can_undo: !self.undo_stack.is_empty(),
            can_redo: !self.redo_stack.is_empty(),
            sidecar_available: vst3.is_sidecar_available(),
            available_effects,
            strips,
        }
    }
}

pub fn normalize_strip_key(strip: &str) -> Option<String> {
    let trimmed = strip.trim();
    match trimmed {
        "song" | "hitsounds" | "master" => Some(trimmed.to_string()),
        _ => {
            let rest = trimmed.strip_prefix("lane:")?;
            let id = project::lane_identity(rest)?;
            Some(format!("lane:{}", project::lane_key(&id)))
        }
    }
}

fn find_slot_mut<'a>(
    session: &'a mut MixerFxSession,
    strip_key: &str,
    slot_id: &str,
) -> Result<&'a mut FxSlotConfig, String> {
    session
        .strips
        .get_mut(strip_key)
        .and_then(|slots| slots.iter_mut().find(|slot| slot.slot_id == slot_id))
        .ok_or_else(|| format!("FX slot {slot_id} not found on strip {strip_key}"))
}

fn default_plugin_name(plugin_path: &str, explicit_name: Option<String>) -> String {
    if let Some(name) = explicit_name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return name;
    }
    PathBuf::from(plugin_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "VST3 Effect".to_string())
}

impl Application {
    pub fn mixer_fx_snapshot(&self) -> MixerFxSnapshot {
        self.mixer_fx.snapshot(&self.vst3, self.live_sample_rate())
    }

    pub(crate) fn mixer_fx_revision(&self) -> u64 {
        self.mixer_fx.revision()
    }

    pub(crate) fn push_mixer_fx_graph(&self) {
        if let Some(a) = &self.audio {
            match self.mixer_fx.build_fx_graph(a.sample_rate()) {
                Some(graph) => a.set_fx_graph(Some(graph)),
                None => a.clear_fx_graph(),
            }
        }
    }

    pub(crate) fn unload_all_mixer_fx(&mut self) {
        if let Some(a) = &self.audio {
            a.clear_fx_graph();
        }
        self.mixer_fx.unload_all(&mut self.vst3);
    }

    pub(crate) fn restore_mixer_fx_from_session(&mut self, saved: Option<MixerFxSession>) {
        self.mixer_fx.unload_all(&mut self.vst3);
        let Some(mut session) = saved else {
            self.push_mixer_fx_graph();
            self.bump_view_revision();
            return;
        };
        session.normalize();
        let mut max_seq = self.mixer_fx.next_slot_seq;
        for slots in session.strips.values() {
            for slot in slots {
                if let Some(n) = slot
                    .slot_id
                    .strip_prefix("fx-")
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    max_seq = max_seq.max(n);
                }
            }
        }
        self.mixer_fx.next_slot_seq = max_seq;
        self.mixer_fx.session = session;
        let sr = self.live_sample_rate();
        self.mixer_fx.reconcile_live(&mut self.vst3, sr, true);
        self.mixer_fx.revision = self.mixer_fx.revision.wrapping_add(1);
        self.push_mixer_fx_graph();
        self.bump_view_revision();
    }

    fn finish_mixer_fx_mutation(&mut self, resync_cues: bool) -> MixerFxSnapshot {
        self.mixer_fx.revision = self.mixer_fx.revision.wrapping_add(1);
        self.push_mixer_fx_graph();
        if resync_cues {
            self.sync_audio();
        }
        let _ = self.write_session(None, None, None, None);
        self.bump_view_revision();
        self.mixer_fx_snapshot()
    }

    fn ensure_fx_doc_ready(&self) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job before editing mixer effects.".into());
        }
        if self.document.is_none() {
            return Err("Open a difficulty first.".into());
        }
        Ok(())
    }

    pub fn mixer_fx_add_insert(
        &mut self,
        strip: &str,
        plugin_path: String,
        plugin_uid: Option<String>,
        name: Option<String>,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let count = self
            .mixer_fx
            .session
            .strips
            .get(&strip_key)
            .map_or(0, Vec::len);
        if count >= MAX_INSERTS_PER_STRIP {
            return Err(format!(
                "A mixer strip supports at most {MAX_INSERTS_PER_STRIP} insert effects."
            ));
        }

        // Strict per-instance sidecar isolation: verify load succeeds before pushing onto undo/session.
        let slot_id = self.mixer_fx.next_slot_id();
        let display_name = default_plugin_name(&plugin_path, name);
        let config = FxSlotConfig {
            slot_id: slot_id.clone(),
            plugin_path,
            plugin_uid,
            name: display_name,
            bypassed: false,
            wet_dry: 1.0,
            active_bank: AbBank::A,
            state_a_b64: None,
            state_b_b64: None,
            pinned: false,
        };
        let sr = self.live_sample_rate();
        self.mixer_fx.spawn_slot_live(&config, &mut self.vst3, sr);
        let unavailable_err = self.mixer_fx.live.get(&slot_id).and_then(|live| match &live.backend {
            LiveBackend::Unavailable { reason } => Some(reason.clone()),
            _ => None,
        });
        if let Some(reason) = unavailable_err {
            self.mixer_fx.live.remove(&slot_id);
            return Err(reason);
        }
        let initial_state = self.mixer_fx.capture_live_state(&slot_id, &mut self.vst3);
        let mut stored = config;
        stored.state_a_b64 = initial_state;

        let before = self.mixer_fx.session.clone();
        self.mixer_fx.push_undo(before);
        self.mixer_fx
            .session
            .strips
            .entry(strip_key)
            .or_default()
            .push(stored);
        self.mixer_fx.rebuild_lane_strip_ids();
        Ok(self.finish_mixer_fx_mutation(true))
    }

    pub fn mixer_fx_remove_insert(
        &mut self,
        strip: &str,
        slot_id: &str,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let pos = self
            .mixer_fx
            .session
            .strips
            .get(&strip_key)
            .and_then(|slots| slots.iter().position(|s| s.slot_id == slot_id))
            .ok_or_else(|| format!("FX slot {slot_id} not found on strip {strip_key}"))?;
        let before = self.mixer_fx.session.clone();
        self.mixer_fx.push_undo(before);
        if let Some(slots) = self.mixer_fx.session.strips.get_mut(&strip_key) {
            slots.remove(pos);
            if slots.is_empty() {
                self.mixer_fx.session.strips.remove(&strip_key);
            }
        }
        self.mixer_fx.unload_slot_live(slot_id, &mut self.vst3);
        self.mixer_fx.rebuild_lane_strip_ids();
        Ok(self.finish_mixer_fx_mutation(true))
    }

    pub fn mixer_fx_move_insert(
        &mut self,
        strip: &str,
        slot_id: &str,
        to_index: usize,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let (from_idx, dest) = {
            let slots = self
                .mixer_fx
                .session
                .strips
                .get(&strip_key)
                .ok_or_else(|| format!("FX slot {slot_id} not found on strip {strip_key}"))?;
            let from_idx = slots
                .iter()
                .position(|s| s.slot_id == slot_id)
                .ok_or_else(|| format!("FX slot {slot_id} not found on strip {strip_key}"))?;
            let dest = to_index.min(slots.len().saturating_sub(1));
            (from_idx, dest)
        };
        if dest == from_idx {
            return Ok(self.mixer_fx_snapshot());
        }
        let before = self.mixer_fx.session.clone();
        self.mixer_fx.push_undo(before);
        if let Some(slots) = self.mixer_fx.session.strips.get_mut(&strip_key) {
            let item = slots.remove(from_idx);
            slots.insert(dest, item);
        }
        Ok(self.finish_mixer_fx_mutation(false))
    }

    pub fn mixer_fx_set_bypass(
        &mut self,
        strip: &str,
        slot_id: &str,
        bypassed: bool,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let before = self.mixer_fx.session.clone();
        let updated = {
            let slot = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)?;
            if slot.bypassed == bypassed {
                return Ok(self.mixer_fx_snapshot());
            }
            slot.bypassed = bypassed;
            slot.clone()
        };
        self.mixer_fx.push_undo(before);
        self.mixer_fx
            .sync_live_slot_params(&updated, &mut self.vst3, false);
        Ok(self.finish_mixer_fx_mutation(false))
    }

    pub fn mixer_fx_set_wet_dry(
        &mut self,
        strip: &str,
        slot_id: &str,
        wet_dry: f32,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let clamped = if wet_dry.is_finite() {
            wet_dry.clamp(0.0, 1.0)
        } else {
            return Err("Wet/dry mix must be finite.".into());
        };
        let before = self.mixer_fx.session.clone();
        let updated = {
            let slot = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)?;
            if (slot.wet_dry - clamped).abs() < 1e-5 {
                return Ok(self.mixer_fx_snapshot());
            }
            slot.wet_dry = clamped;
            slot.clone()
        };
        self.mixer_fx.push_undo(before);
        self.mixer_fx
            .sync_live_slot_params(&updated, &mut self.vst3, false);
        Ok(self.finish_mixer_fx_mutation(false))
    }

    pub fn mixer_fx_switch_ab(
        &mut self,
        strip: &str,
        slot_id: &str,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let before = self.mixer_fx.session.clone();
        let live_state = self.mixer_fx.capture_live_state(slot_id, &mut self.vst3);
        let updated = {
            let slot = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)?;
            match slot.active_bank {
                AbBank::A => {
                    if let Some(state) = live_state {
                        slot.state_a_b64 = Some(state);
                    }
                    if slot.state_b_b64.is_none() {
                        slot.state_b_b64 = slot.state_a_b64.clone();
                    }
                    slot.active_bank = AbBank::B;
                }
                AbBank::B => {
                    if let Some(state) = live_state {
                        slot.state_b_b64 = Some(state);
                    }
                    if slot.state_a_b64.is_none() {
                        slot.state_a_b64 = slot.state_b_b64.clone();
                    }
                    slot.active_bank = AbBank::A;
                }
            }
            slot.clone()
        };
        self.mixer_fx.push_undo(before);
        self.mixer_fx
            .sync_live_slot_params(&updated, &mut self.vst3, true);
        Ok(self.finish_mixer_fx_mutation(false))
    }

    pub fn mixer_fx_copy_ab(
        &mut self,
        strip: &str,
        slot_id: &str,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let before = self.mixer_fx.session.clone();
        let live_state = self.mixer_fx.capture_live_state(slot_id, &mut self.vst3);
        let slot = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)?;
        match slot.active_bank {
            AbBank::A => {
                if let Some(state) = live_state {
                    slot.state_a_b64 = Some(state);
                }
                slot.state_b_b64 = slot.state_a_b64.clone();
            }
            AbBank::B => {
                if let Some(state) = live_state {
                    slot.state_b_b64 = Some(state);
                }
                slot.state_a_b64 = slot.state_b_b64.clone();
            }
        }
        self.mixer_fx.push_undo(before);
        Ok(self.finish_mixer_fx_mutation(false))
    }

    pub fn mixer_fx_open_editor(
        &mut self,
        strip: &str,
        slot_id: &str,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let _ = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)?;

        // Single-Window Swap + Pin: close every other unpinned slot's native editor.
        let unpinned_others: Vec<String> = self
            .mixer_fx
            .session
            .strips
            .values()
            .flatten()
            .filter(|slot| slot.slot_id != slot_id && !slot.pinned)
            .map(|slot| slot.slot_id.clone())
            .collect();
        for other_id in unpinned_others {
            let _ = self.mixer_fx.close_slot_editor(&other_id, &mut self.vst3);
        }

        self.mixer_fx.open_slot_editor(slot_id, &mut self.vst3)?;
        self.mixer_fx.revision = self.mixer_fx.revision.wrapping_add(1);
        self.bump_view_revision();
        Ok(self.mixer_fx_snapshot())
    }

    pub fn mixer_fx_close_editor(
        &mut self,
        strip: &str,
        slot_id: &str,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let _ = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)?;
        let captured = self.mixer_fx.capture_live_state(slot_id, &mut self.vst3);
        if let Some(state) = captured
            && let Ok(slot) = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)
        {
            match slot.active_bank {
                AbBank::A => slot.state_a_b64 = Some(state),
                AbBank::B => slot.state_b_b64 = Some(state),
            }
        }
        self.mixer_fx.close_slot_editor(slot_id, &mut self.vst3)?;
        let _ = self.write_session(None, None, None, None);
        self.mixer_fx.revision = self.mixer_fx.revision.wrapping_add(1);
        self.bump_view_revision();
        Ok(self.mixer_fx_snapshot())
    }

    pub fn mixer_fx_set_pinned(
        &mut self,
        strip: &str,
        slot_id: &str,
        pinned: bool,
    ) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let strip_key = normalize_strip_key(strip)
            .ok_or_else(|| format!("Unknown mixer strip '{strip}'."))?;
        let before = self.mixer_fx.session.clone();
        let slot = find_slot_mut(&mut self.mixer_fx.session, &strip_key, slot_id)?;
        if slot.pinned == pinned {
            return Ok(self.mixer_fx_snapshot());
        }
        slot.pinned = pinned;
        self.mixer_fx.push_undo(before);
        Ok(self.finish_mixer_fx_mutation(false))
    }

    pub fn mixer_fx_undo(&mut self) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let Some(prev) = self.mixer_fx.undo_stack.pop_back() else {
            return Ok(self.mixer_fx_snapshot());
        };
        let current = std::mem::replace(&mut self.mixer_fx.session, prev);
        self.mixer_fx.redo_stack.push(current);
        let sr = self.live_sample_rate();
        self.mixer_fx.reconcile_live(&mut self.vst3, sr, true);
        Ok(self.finish_mixer_fx_mutation(true))
    }

    pub fn mixer_fx_redo(&mut self) -> Result<MixerFxSnapshot, String> {
        self.ensure_fx_doc_ready()?;
        let Some(next) = self.mixer_fx.redo_stack.pop() else {
            return Ok(self.mixer_fx_snapshot());
        };
        let current = std::mem::replace(&mut self.mixer_fx.session, next);
        if self.mixer_fx.undo_stack.len() >= MAX_MIXER_FX_UNDO {
            self.mixer_fx.undo_stack.pop_front();
        }
        self.mixer_fx.undo_stack.push_back(current);
        let sr = self.live_sample_rate();
        self.mixer_fx.reconcile_live(&mut self.vst3, sr, true);
        Ok(self.finish_mixer_fx_mutation(true))
    }

    #[cfg(test)]
    pub(crate) fn install_mock_fx_host(&mut self, host: Box<dyn TestEffectHost>) {
        self.mixer_fx.test_host = Some(host);
    }

    #[allow(dead_code)]
    pub(crate) fn _describe_strip_key(sample: &SampleIdentity) -> String {
        lane_name(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::lane_key;
    use dawsu_foundation::{CustomIndex, Hitsound, SampleSet, Sound};
    use std::collections::HashMap;
    use std::path::Path;

    struct MockPluginData {
        state: String,
        bypassed: bool,
        editor_open: bool,
    }

    struct MockHostState {
        next_id: u64,
        plugins: HashMap<String, MockPluginData>,
        fixed_latency: u32,
        events: Vec<String>,
    }

    impl MockHostState {
        fn new(latency: u32) -> Self {
            Self {
                next_id: 0,
                plugins: HashMap::new(),
                fixed_latency: latency,
                events: Vec::new(),
            }
        }
    }

    struct MockProc {
        latency: u32,
    }

    impl AudioEffectProcessor for MockProc {
        fn process_block(
            &mut self,
            _left: &mut [f32],
            _right: &mut [f32],
            _sample_rate: u32,
            _is_playing: bool,
            _position_ms: f64,
            _speed: f64,
            _bypassed: bool,
        ) {
        }

        fn latency_samples(&self) -> u32 {
            self.latency
        }
    }

    struct MockHostHandle {
        inner: Arc<Mutex<MockHostState>>,
    }

    impl TestEffectHost for MockHostHandle {
        fn spawn_effect(
            &mut self,
            plugin_path: &str,
            _plugin_uid: Option<&str>,
            _runtime: Arc<FxSlotRuntime>,
        ) -> Result<(String, Box<dyn AudioEffectProcessor>, String), String> {
            let mut st = self.inner.lock().unwrap();
            st.next_id += 1;
            let id = format!("mock-{}", st.next_id);
            let initial_state = format!("init:{plugin_path}:{}", st.next_id);
            let lat = st.fixed_latency;
            st.plugins.insert(
                id.clone(),
                MockPluginData {
                    state: initial_state.clone(),
                    bypassed: false,
                    editor_open: false,
                },
            );
            st.events.push(format!("spawn:{id}:{plugin_path}"));
            Ok((id, Box::new(MockProc { latency: lat }), initial_state))
        }

        fn set_state(&mut self, id: &str, state_b64: &str) -> Result<(), String> {
            let mut st = self.inner.lock().unwrap();
            if let Some(p) = st.plugins.get_mut(id) {
                p.state = state_b64.to_string();
                st.events.push(format!("set_state:{id}:{state_b64}"));
                Ok(())
            } else {
                Err("missing".into())
            }
        }

        fn get_state(&mut self, id: &str) -> Result<String, String> {
            let st = self.inner.lock().unwrap();
            st.plugins
                .get(id)
                .map(|p| p.state.clone())
                .ok_or_else(|| "missing".to_string())
        }

        fn set_bypass(&mut self, id: &str, bypassed: bool) -> Result<(), String> {
            let mut st = self.inner.lock().unwrap();
            if let Some(p) = st.plugins.get_mut(id) {
                p.bypassed = bypassed;
                st.events.push(format!("bypass:{id}:{bypassed}"));
                Ok(())
            } else {
                Err("missing".into())
            }
        }

        fn open_editor(&mut self, id: &str) -> Result<(), String> {
            let mut st = self.inner.lock().unwrap();
            if let Some(p) = st.plugins.get_mut(id) {
                p.editor_open = true;
                st.events.push(format!("open_editor:{id}"));
                Ok(())
            } else {
                Err("missing".into())
            }
        }

        fn close_editor(&mut self, id: &str) -> Result<(), String> {
            let mut st = self.inner.lock().unwrap();
            if let Some(p) = st.plugins.get_mut(id) {
                p.editor_open = false;
                st.events.push(format!("close_editor:{id}"));
                Ok(())
            } else {
                Err("missing".into())
            }
        }

        fn unload(&mut self, id: &str) {
            let mut st = self.inner.lock().unwrap();
            st.plugins.remove(id);
            st.events.push(format!("unload:{id}"));
        }
    }

    fn make_temp_mapset(label: &str) -> (PathBuf, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dawsu-mixer-fx-{label}-{nanos}"));
        let data_root = root.join("data");
        let mapset = root.join("mapset");
        std::fs::create_dir_all(&data_root).unwrap();
        std::fs::create_dir_all(&mapset).unwrap();
        let osu = mapset.join("test [Diff].osu");
        std::fs::write(
            &osu,
            "\
osu file format v14

[General]
AudioFilename: audio.mp3
Mode: 0

[Metadata]
Title: FX Test
Artist: Tester
Version: Diff

[TimingPoints]
0,500,4,2,0,50,1,0

[HitObjects]
256,192,1000,1,8,0:0:0:0:
256,192,1500,1,0,0:0:0:0:
",
        )
        .unwrap();
        (root, osu)
    }

    fn open_test_app(root: &Path, osu: &Path, mock: Arc<Mutex<MockHostState>>) -> Application {
        let mut app = Application::headless_with_projects(root.join("data"));
        app.install_mock_fx_host(Box::new(MockHostHandle {
            inner: Arc::clone(&mock),
        }));
        app.open_now(crate::OpenSource::Map(osu.to_path_buf()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.is_busy() && std::time::Instant::now() < deadline {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.document().is_some(), "expected open document, error={:?}", app.error());
        app
    }

    #[test]
    fn mixer_fx_ab_bypass_wet_pin_undo_and_persistence_work() {
        let (root, osu) = make_temp_mapset("workflow");
        let mock = Arc::new(Mutex::new(MockHostState::new(512)));
        let mut app = open_test_app(&root, &osu, Arc::clone(&mock));

        let clap = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: CustomIndex::new(0),
        };
        let clap_strip = format!("lane:{}", lane_key(&clap));

        let s1 = app
            .mixer_fx_add_insert(
                &clap_strip,
                "C:/Plugins/ProQ.vst3".into(),
                Some("UID1".into()),
                Some("Pro-Q".into()),
            )
            .unwrap();
        assert_eq!(s1.total_pdc_samples, 512);
        let id1 = s1.strips[&clap_strip][0].slot_id.clone();

        // Scheduled cue on Soft Clap lane routes to dedicated strip >= STRIP_LANE_BASE; normal stays 0.
        let doc = app.document().unwrap();
        let sounds = app.scheduled_sounds(doc);
        assert!(sounds.iter().any(|s| s.strip_id == STRIP_LANE_BASE));
        assert!(sounds.iter().any(|s| s.strip_id == STRIP_DRY));

        // Add two master inserts (strict per-instance isolation: 3 distinct mock processes spawned!).
        let s2 = app
            .mixer_fx_add_insert("master", "C:/Plugins/Comp.vst3".into(), None, Some("Comp".into()))
            .unwrap();
        let id2 = s2.strips["master"][0].slot_id.clone();
        let s3 = app
            .mixer_fx_add_insert("master", "C:/Plugins/Lim.vst3".into(), None, Some("Lim".into()))
            .unwrap();
        let id3 = s3.strips["master"][1].slot_id.clone();
        assert_eq!(mock.lock().unwrap().plugins.len(), 3);
        assert_eq!(s3.total_pdc_samples, 1536);

        // Soft bypass preserves PDC latency!
        let s_bypass = app.mixer_fx_set_bypass(&clap_strip, &id1, true).unwrap();
        assert!(s_bypass.strips[&clap_strip][0].bypassed);
        assert_eq!(s_bypass.strips[&clap_strip][0].latency_samples, 512);
        assert_eq!(s_bypass.total_pdc_samples, 1536);

        // Wet/dry mix clamps and persists.
        let s_wet = app.mixer_fx_set_wet_dry(&clap_strip, &id1, 0.35).unwrap();
        assert!((s_wet.strips[&clap_strip][0].wet_dry - 0.35).abs() < 1e-5);

        // Mutate live state on bank A, copy to B, mutate A, switch to B -> B keeps copied state!
        let inst1 = mock
            .lock()
            .unwrap()
            .plugins
            .keys()
            .find(|k| k.ends_with("-1"))
            .unwrap()
            .clone();
        mock.lock().unwrap().plugins.get_mut(&inst1).unwrap().state = "preset_A".into();
        app.mixer_fx_copy_ab(&clap_strip, &id1).unwrap();
        mock.lock().unwrap().plugins.get_mut(&inst1).unwrap().state = "preset_A_edited".into();
        let s_b = app.mixer_fx_switch_ab(&clap_strip, &id1).unwrap();
        assert_eq!(s_b.strips[&clap_strip][0].active_bank, AbBank::B);
        assert_eq!(mock.lock().unwrap().plugins[&inst1].state, "preset_A");
        let s_a = app.mixer_fx_switch_ab(&clap_strip, &id1).unwrap();
        assert_eq!(s_a.strips[&clap_strip][0].active_bank, AbBank::A);
        assert_eq!(mock.lock().unwrap().plugins[&inst1].state, "preset_A_edited");

        // Single-Window Swap + Pin: pin slot 2, open slot 1 -> opening slot 3 closes slot 1 (unpinned) while keeping slot 2 open!
        app.mixer_fx_set_pinned("master", &id2, true).unwrap();
        let e2 = app.mixer_fx_open_editor("master", &id2).unwrap();
        assert!(e2.strips["master"][0].editor_open);
        let e1 = app.mixer_fx_open_editor(&clap_strip, &id1).unwrap();
        assert!(e1.strips[&clap_strip][0].editor_open);
        assert!(e1.strips["master"][0].editor_open, "pinned slot 2 must remain open");
        let e3 = app.mixer_fx_open_editor("master", &id3).unwrap();
        assert!(e3.strips["master"][1].editor_open);
        assert!(e3.strips["master"][0].editor_open, "pinned slot 2 stays open");
        assert!(!e3.strips[&clap_strip][0].editor_open, "unpinned slot 1 auto-closed");

        // Reorder master slots.
        let moved = app.mixer_fx_move_insert("master", &id3, 0).unwrap();
        assert_eq!(moved.strips["master"][0].slot_id, id3);

        // Undo move and redo move.
        let undone = app.mixer_fx_undo().unwrap();
        assert_eq!(undone.strips["master"][0].slot_id, id2);
        let redone = app.mixer_fx_redo().unwrap();
        assert_eq!(redone.strips["master"][0].slot_id, id3);

        // Reload difficulty from disk and confirm session persistence restored all inserts cleanly!
        app.save_session().unwrap();
        app.close_document();
        assert_eq!(mock.lock().unwrap().plugins.len(), 0, "close must unload live FX sidecars");

        let app2 = open_test_app(&root, &osu, Arc::clone(&mock));
        let restored = app2.mixer_fx_snapshot();
        assert_eq!(restored.strips[&clap_strip].len(), 1);
        assert!(restored.strips[&clap_strip][0].bypassed);
        assert!((restored.strips[&clap_strip][0].wet_dry - 0.35).abs() < 1e-5);
        assert_eq!(restored.strips["master"].len(), 2);
        assert_eq!(restored.strips["master"][0].slot_id, id3);
        assert!(restored.strips["master"][1].pinned);
        assert_eq!(restored.total_pdc_samples, 1536);

        let _ = std::fs::remove_dir_all(&root);
    }
}
