//! Manual edits on top of the shared editing kernel.

use dawsu_editor::tempo::{Snap, SnapMode, SnapSpacing, TempoMap};
use dawsu_editor::tools::{PointerInput, TimeRange, ToolPreview, ToolTarget};
use dawsu_editor::{Clipboard, EditHistory, Selection};
use dawsu_foundation::{Hitsound, SampleIdentity, SongTime, Sound};
use dawsu_hitsounding::{Hitsounds, MomentId};
use std::convert::Infallible;

mod workflows;
pub use workflows::{RangeOperation, VolumeScope};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HitKey {
    pub moment: usize,
    pub lane: SampleIdentity,
}

pub struct Composer {
    pub history: EditHistory<Hitsounds>,
    pub selected: Selection<HitKey>,
    pub lanes: Vec<SampleIdentity>,
    pub tempo: TempoMap,
    pub spacing: SnapSpacing,
    pub snap_enabled: bool,
    pub snap_mode: SnapMode,
    object_times: Vec<SongTime>,
    pub insert_volume: i32,
    pub protect_normals: bool,
    pub range: Option<TimeRange>,
    pub view_start: f64,
    pub row_offset: f64,
    pub picker_requested: bool,
    pub audition_requests: Vec<SampleIdentity>,
    pub listen_muted: Vec<HitKey>,
    pub listen_version: u64,
    pub preview: ToolPreview,
    clipboard: Clipboard<(SampleIdentity, i32)>,
}

impl Composer {
    pub fn new(hitsounds: Hitsounds) -> Self {
        let mut object_times: Vec<_> = hitsounds
            .moments()
            .iter()
            .filter(|m| m.place() != dawsu_hitsounding::Place::NoObject)
            .map(|m| m.time())
            .collect();
        object_times.sort_by(|a, b| a.ms().total_cmp(&b.ms()));
        object_times.dedup();
        let lanes = hitsounds
            .lanes()
            .iter()
            .map(|l| l.identity.clone())
            .collect();
        Self {
            history: EditHistory::new(hitsounds),
            selected: Selection::new(),
            lanes,
            tempo: TempoMap::default(),
            spacing: SnapSpacing::fraction(4).unwrap(),
            snap_enabled: true,
            snap_mode: SnapMode::Grid,
            object_times,
            insert_volume: 100,
            protect_normals: false,
            range: None,
            view_start: 0.0,
            row_offset: 0.0,
            picker_requested: false,
            audition_requests: Vec::new(),
            listen_muted: Vec::new(),
            listen_version: 0,
            preview: ToolPreview::None,
            clipboard: Clipboard::new(),
        }
    }
    pub fn hitsounds(&self) -> &Hitsounds {
        self.history.document()
    }
    pub fn dirty(&self) -> bool {
        self.history.is_dirty()
    }
    pub fn mark_saved(&mut self) {
        self.history.mark_clean();
    }
    pub fn undo(&mut self) {
        self.history.undo();
        self.selected.clear();
    }
    pub fn redo(&mut self) {
        self.history.redo();
        self.selected.clear();
    }
    pub fn apply(&mut self, label: &str, edit: impl FnOnce(&mut Hitsounds)) {
        let _ = self.history.apply(label, true, |h| {
            edit(h);
            Ok::<_, Infallible>(())
        });
    }
    pub fn add(&mut self, time: f64, lane: SampleIdentity, volume: i32, tolerance: f64) {
        let mut id = 0;
        self.apply("Draw hit", |h| {
            id = insert(h, time, lane.clone(), volume, tolerance);
        });
        if !self.lanes.contains(&lane) {
            self.lanes.push(lane.clone());
        }
        self.selected.replace([HitKey { moment: id, lane }]);
    }
    pub fn remove_selected(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let keys: Vec<_> = self.selected.items().iter().cloned().collect();
        let protect = self.protect_normals;
        self.apply("Delete hits", |h| {
            for key in keys {
                if !protect || !normal(&key.lane) {
                    h.remove_hit(MomentId(key.moment), &key.lane);
                }
            }
        });
        self.selected.clear();
    }
    pub fn move_selected(
        &mut self,
        delta: f64,
        destination: Option<SampleIdentity>,
        tolerance: f64,
    ) {
        self.move_to(destination, tolerance, |_, time| (time + delta).max(0.0));
    }

    pub fn nudge(&mut self, direction: i32) {
        let spacing = self.spacing;
        self.move_to(None, 5.0, |tempo, time| {
            tempo
                .nudge(SongTime::from_ms(time), direction, spacing)
                .ms()
        });
    }

    fn move_to(
        &mut self,
        destination: Option<SampleIdentity>,
        tolerance: f64,
        target: impl Fn(&TempoMap, f64) -> f64,
    ) {
        let keys: Vec<_> = self
            .selected
            .items()
            .iter()
            .map(|key| {
                (
                    key.clone(),
                    target(
                        &self.tempo,
                        self.hitsounds().moments()[key.moment].time().ms(),
                    ),
                )
            })
            .collect();
        let mut moved = Vec::new();
        self.apply("Move hits", |h| {
            let values: Vec<_> = keys
                .iter()
                .map(|(k, time)| (k.clone(), *time, h.moments()[k.moment].volume()))
                .collect();
            for (key, _, _) in &values {
                h.remove_hit(MomentId(key.moment), &key.lane);
            }
            for (key, time, volume) in values {
                let lane = destination.clone().unwrap_or(key.lane);
                let moment = insert(h, time, lane.clone(), volume, tolerance);
                moved.push(HitKey { moment, lane });
            }
        });
        if let Some(lane) = destination {
            if !self.lanes.contains(&lane) {
                self.lanes.push(lane);
            }
        }
        self.selected.replace(moved);
    }
    pub fn move_rows(&mut self, delta: i32) {
        let items: Vec<_> = self.selected.items().iter().cloned().collect();
        if items.is_empty() {
            return;
        }
        self.history.begin("Move hits between lanes");
        ToolTarget::move_by(self, &items, 0.0, delta);
        self.history.commit();
    }
    pub fn select_neighbour(&mut self, direction: i32, add: bool) {
        let time = self
            .selected
            .primary()
            .map(|k| self.hitsounds().moments()[k.moment].time().ms());
        let mut keys: Vec<_> = self
            .hitsounds()
            .moments()
            .iter()
            .enumerate()
            .flat_map(|(i, m)| {
                m.hits().map(move |lane| HitKey {
                    moment: i,
                    lane: lane.clone(),
                })
            })
            .collect();
        keys.sort_by(|a, b| {
            self.hitsounds().moments()[a.moment]
                .time()
                .ms()
                .total_cmp(&self.hitsounds().moments()[b.moment].time().ms())
                .then_with(|| a.lane.cmp(&b.lane))
        });
        if direction < 0 {
            keys.reverse();
        }
        if let Some(key) = keys.into_iter().find(|k| {
            time.is_none_or(|t| {
                if direction < 0 {
                    self.hitsounds().moments()[k.moment].time().ms() < t
                } else {
                    self.hitsounds().moments()[k.moment].time().ms() > t
                }
            })
        }) {
            if add {
                self.selected.add(key);
            } else {
                self.selected.replace([key]);
            }
        }
    }
    pub fn volume(&mut self, value: i32, relative: bool) {
        if self.selected.is_empty() {
            return;
        }
        let keys: Vec<_> = self.selected.items().iter().cloned().collect();
        self.apply("Change volume", |h| {
            let mut moments: Vec<_> = keys.iter().map(|k| k.moment).collect();
            moments.sort_unstable();
            moments.dedup();
            for id in moments {
                let v = if relative {
                    h.moments()[id].volume() + value
                } else {
                    value
                };
                h.set_volume(MomentId(id), v.clamp(0, 100));
            }
        });
    }
    pub fn select_all(&mut self) {
        self.selected.replace(
            self.hitsounds()
                .moments()
                .iter()
                .enumerate()
                .flat_map(|(i, m)| {
                    m.hits().map(move |lane| HitKey {
                        moment: i,
                        lane: lane.clone(),
                    })
                })
                .collect::<Vec<_>>(),
        );
    }
    pub fn copy(&mut self) {
        let items: Vec<_> = self
            .selected
            .items()
            .iter()
            .map(|k| {
                let m = &self.hitsounds().moments()[k.moment];
                (m.time(), (k.lane.clone(), m.volume()))
            })
            .collect();
        self.clipboard.copy(items);
    }
    pub fn paste(&mut self, at: f64) {
        if self.clipboard.is_empty() {
            return;
        }
        let clipboard = self.clipboard.paste_at(SongTime::from_ms(at));
        let mut added = Vec::new();
        self.apply("Paste hits", |h| {
            for (time, (lane, v)) in clipboard {
                let moment = insert(h, time.ms(), lane.clone(), v, 5.0);
                added.push(HitKey { moment, lane });
            }
        });
        self.selected.replace(added);
    }
    pub fn duplicate(&mut self) {
        let Some(last) = self
            .selected
            .items()
            .iter()
            .map(|k| self.hitsounds().moments()[k.moment].time().ms())
            .reduce(f64::max)
        else {
            return;
        };
        let step = self
            .spacing
            .step_ms(self.tempo.beat_length_at(SongTime::from_ms(last)));
        self.copy();
        self.paste(last + step);
    }
}

fn normal(lane: &SampleIdentity) -> bool {
    matches!(
        lane,
        SampleIdentity::Standard {
            sound: Sound::Hit(Hitsound::Normal),
            ..
        }
    )
}
fn insert(
    h: &mut Hitsounds,
    time: f64,
    lane: SampleIdentity,
    volume: i32,
    tolerance: f64,
) -> usize {
    let id = h
        .moments()
        .iter()
        .enumerate()
        .filter(|(_, m)| (m.time().ms() - time).abs() <= tolerance)
        .min_by(|(_, a), (_, b)| {
            (a.time().ms() - time)
                .abs()
                .total_cmp(&(b.time().ms() - time).abs())
        })
        .map(|(i, _)| MomentId(i))
        .unwrap_or_else(|| h.add_moment(SongTime::from_ms(time), volume));
    h.add_hit(id, lane);
    id.0
}

impl ToolTarget for Composer {
    type Item = HitKey;
    fn hit_test(&self, at: &PointerInput) -> Option<HitKey> {
        let lane = self.lanes.get(at.row.max(0) as usize)?;
        self.hitsounds()
            .moments()
            .iter()
            .enumerate()
            .filter(|(_, m)| m.has_hit(lane) && (m.time().ms() - at.time.ms()).abs() <= 10.0)
            .min_by(|(_, a), (_, b)| {
                (a.time().ms() - at.time.ms())
                    .abs()
                    .total_cmp(&(b.time().ms() - at.time.ms()).abs())
            })
            .map(|(moment, _)| HitKey {
                moment,
                lane: lane.clone(),
            })
    }
    fn in_range(&self, from: SongTime, to: SongTime, first: i32, last: i32) -> Vec<HitKey> {
        let mut keys = Vec::new();
        for (i, m) in self.hitsounds().moments().iter().enumerate() {
            if m.time().ms() < from.ms().min(to.ms()) || m.time().ms() > from.ms().max(to.ms()) {
                continue;
            }
            for lane in m.hits() {
                if let Some(row) = self.lanes.iter().position(|l| l == lane) {
                    if (first..=last).contains(&(row as i32)) {
                        keys.push(HitKey {
                            moment: i,
                            lane: lane.clone(),
                        });
                    }
                }
            }
        }
        keys
    }
    fn item_time(&self, item: &HitKey) -> SongTime {
        self.hitsounds().moments()[item.moment].time()
    }
    fn selection(&mut self) -> &mut Selection<HitKey> {
        &mut self.selected
    }
    fn snap(&self, time: SongTime) -> SongTime {
        Snap {
            on: self.snap_enabled,
            mode: self.snap_mode,
            spacing: self.spacing,
        }
        .time(&self.tempo, &self.object_times, time)
    }
    fn snap_offset(&self, from: SongTime, raw: f64) -> f64 {
        Snap {
            on: self.snap_enabled,
            mode: self.snap_mode,
            spacing: self.spacing,
        }
        .offset(&self.tempo, &self.object_times, from, raw)
    }
    fn begin(&mut self, label: &str) {
        self.history.begin(label);
    }
    fn commit(&mut self) {
        self.history.commit();
    }
    fn cancel(&mut self) {
        self.history.cancel();
    }
    fn insert(&mut self, at: SongTime, row: i32) -> Option<HitKey> {
        let lane = self.lanes.get(row.max(0) as usize)?.clone();
        let volume = self.insert_volume;
        let mut moment = 0;
        let _ = self.history.edit(true, |h| {
            moment = insert(h, at.ms(), lane.clone(), volume, 5.0);
            Ok::<_, Infallible>(())
        });
        self.audition_requests.push(lane.clone());
        Some(HitKey { moment, lane })
    }
    fn delete(&mut self, items: &[HitKey]) {
        let protect = self.protect_normals;
        let _ = self.history.edit(true, |h| {
            for k in items {
                if !protect || !normal(&k.lane) {
                    h.remove_hit(MomentId(k.moment), &k.lane);
                }
            }
            Ok::<_, Infallible>(())
        });
    }
    fn move_by(&mut self, items: &[HitKey], delta: f64, rows: i32) {
        let lanes = self.lanes.clone();
        let mut new = Vec::new();
        let _ = self.history.edit(true, |h| {
            let values: Vec<_> = items
                .iter()
                .map(|k| {
                    (
                        k.clone(),
                        h.moments()[k.moment].time().ms(),
                        h.moments()[k.moment].volume(),
                    )
                })
                .collect();
            for (k, _, _) in &values {
                h.remove_hit(MomentId(k.moment), &k.lane);
            }
            for (k, t, v) in values {
                let old = lanes.iter().position(|l| *l == k.lane).unwrap_or(0) as i32;
                let lane = lanes[(old + rows).clamp(0, lanes.len() as i32 - 1) as usize].clone();
                let moment = insert(h, (t + delta).max(0.0), lane.clone(), v, 5.0);
                new.push(HitKey { moment, lane });
            }
            Ok::<_, Infallible>(())
        });
        if rows != 0 {
            self.audition_requests
                .extend(new.iter().map(|k| k.lane.clone()));
        }
        self.selected.replace(new);
    }
    fn set_values(&mut self, values: &[(HitKey, i32)]) {
        let _ = self.history.edit(true, |h| {
            for (key, v) in values {
                h.set_volume(MomentId(key.moment), (*v).clamp(0, 100));
            }
            Ok::<_, Infallible>(())
        });
    }
    fn toggle_listen_mute(&mut self, item: &HitKey) {
        if let Some(i) = self.listen_muted.iter().position(|k| k == item) {
            self.listen_muted.remove(i);
        } else {
            self.listen_muted.push(item.clone());
        }
        self.listen_version += 1;
    }
    fn audition(&mut self, item: &HitKey) {
        self.audition_requests.push(item.lane.clone());
    }
    fn pan(&mut self, ms: f64, rows: f64) {
        self.view_start = (self.view_start + ms).max(0.0);
        self.row_offset = (self.row_offset + rows).max(0.0);
    }
    fn set_range(&mut self, range: Option<TimeRange>) {
        self.range = range;
    }
    fn open_tool_picker(&mut self, _at: &PointerInput) {
        self.picker_requested = true;
    }
    fn show_preview(&mut self, preview: ToolPreview) {
        self.preview = preview;
    }
}
