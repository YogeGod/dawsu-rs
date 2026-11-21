//! Range-keyed LRU output cache for offline graph evaluation.

use crate::ids::{AnalysisRange, NodeTypeId, SocketId};
use crate::registry::NodeDiagnostic;
use crate::value::{EventList, SocketValue};
use dawsu_foundation::digest::sha256_hex;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Default memory cap for cached intermediate outputs (512 MiB).
pub const DEFAULT_MAX_BYTES: usize = 512 * 1024 * 1024;
const ENGINE_CACHE_SALT: &str = "dawsu-graph-v1";

/// Deterministic SHA-256 key covering node type, version, params, inputs, range, and source key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CacheKey(pub String);

impl CacheKey {
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        type_id: &NodeTypeId,
        type_version: u32,
        muted: bool,
        canonical_params: &str,
        source_key: Option<&str>,
        input_keys: &[(SocketId, Vec<CacheKey>)],
        range: AnalysisRange,
        song_duration_ms: Option<f64>,
    ) -> Self {
        let (from_bits, to_bits) = range.canonical_bits();
        let duration_bits = song_duration_ms.map(|d| if d == 0.0 { 0.0_f64.to_bits() } else { d.to_bits() });
        let inputs_json: Vec<(&str, Vec<&str>)> = input_keys
            .iter()
            .map(|(sock, keys)| {
                (
                    sock.as_str(),
                    keys.iter().map(|k| k.0.as_str()).collect(),
                )
            })
            .collect();
        let payload = serde_json::json!({
            "salt": ENGINE_CACHE_SALT,
            "type": type_id.as_str(),
            "typeVersion": type_version,
            "muted": muted,
            "params": canonical_params,
            "source": source_key,
            "inputs": inputs_json,
            "range": [from_bits, to_bits],
            "songDurationBits": duration_bits,
        });
        let text = serde_json::to_string(&payload).unwrap_or_default();
        Self(sha256_hex(text.as_bytes()))
    }
}

/// Cached evaluation result for one node.
#[derive(Debug, Clone)]
pub struct CachedNodeOutput {
    pub values: BTreeMap<SocketId, SocketValue>,
    pub diagnostics: Vec<NodeDiagnostic>,
    pub slot_output: Option<(String, Arc<EventList>)>,
    pub viewer_output: Option<SocketValue>,
    pub byte_size: usize,
}

impl CachedNodeOutput {
    pub fn estimate_bytes(
        values: &BTreeMap<SocketId, SocketValue>,
        slot_output: &Option<(String, Arc<EventList>)>,
        viewer_output: &Option<SocketValue>,
    ) -> usize {
        let values_bytes: usize = values.values().map(SocketValue::byte_size).sum();
        let slot_bytes = slot_output.as_ref().map_or(0, |(_, e)| e.byte_size());
        let viewer_bytes = viewer_output.as_ref().map_or(0, SocketValue::byte_size);
        values_bytes + slot_bytes + viewer_bytes + 64
    }
}

/// Diagnostic counters for `EvalCache`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    pub bytes: usize,
}

struct CacheEntry {
    output: CachedNodeOutput,
    last_used: u64,
}

/// Bounded LRU memory cache keyed by `CacheKey`.
pub struct EvalCache {
    max_bytes: usize,
    total_bytes: usize,
    tick: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    entries: HashMap<CacheKey, CacheEntry>,
}

impl Default for EvalCache {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BYTES)
    }
}

impl EvalCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            total_bytes: 0,
            tick: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: &CacheKey) -> Option<CachedNodeOutput> {
        let next_tick = self.tick.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(key) {
            self.tick = next_tick;
            entry.last_used = next_tick;
            self.hits += 1;
            Some(entry.output.clone())
        } else {
            self.misses += 1;
            None
        }
    }

    pub fn insert(&mut self, key: CacheKey, output: CachedNodeOutput) {
        if self.max_bytes == 0 || output.byte_size > self.max_bytes {
            return;
        }
        if let Some(prev) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(prev.output.byte_size);
        }
        while self.total_bytes + output.byte_size > self.max_bytes {
            let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest_key) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.output.byte_size);
                self.evictions += 1;
            }
        }
        self.tick = self.tick.wrapping_add(1);
        self.total_bytes += output.byte_size;
        self.entries.insert(
            key,
            CacheEntry {
                output,
                last_used: self.tick,
            },
        );
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            entries: self.entries.len(),
            bytes: self.total_bytes,
        }
    }
}
