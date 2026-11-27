//! Smart member lane resolution, grouped `ToolTarget` editing adapter, and grouped timeline
//! operations for merged rows in Drums view.

use crate::drum_view::DrumRowGroups;
use dawsu_editor::tempo::SnapMode;
use dawsu_editor::tools::{PointerInput, PointerSurface, TimeRange, ToolPreview, ToolTarget};
use dawsu_editor::Selection;
use dawsu_editor_composer::{Composer, HitKey};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::lazer::control_points::{CONTROL_POINT_LENIENCY, ControlPoints};
use dawsu_hitsounding::{Hitsounds, MomentId};
use dawsu_osu_format::OsuFile;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;

pub(crate) type DrumMoveStroke = (Hitsounds, Vec<(HitKey, f64, i32)>, f64, i32);

pub(crate) fn is_hitnormal(lane: &SampleIdentity) -> bool {
    matches!(
        lane,
        SampleIdentity::Standard {
            sound: Sound::Hit(Hitsound::Normal),
            ..
        }
    )
}

pub(crate) fn nearest_moment(h: &Hitsounds, time: f64, tolerance: f64) -> Option<MomentId> {
    h.moments()
        .iter()
        .enumerate()
        .filter(|(_, m)| (m.time().ms() - time).abs() <= tolerance)
        .min_by(|(_, a), (_, b)| {
            (a.time().ms() - time)
                .abs()
                .total_cmp(&(b.time().ms() - time).abs())
        })
        .map(|(i, _)| MomentId(i))
}

fn can_remove_hit(composer: &Composer, key: &HitKey) -> bool {
    (!composer.protect_normals || !is_hitnormal(&key.lane))
        && composer
            .hitsounds()
            .moments()
            .get(key.moment)
            .is_some_and(|moment| moment.has_hit(&key.lane))
}

fn parse_tp_set(bank: &str) -> Option<SampleSet> {
    match bank.to_ascii_lowercase().as_str() {
        "normal" => Some(SampleSet::Normal),
        "soft" => Some(SampleSet::Soft),
        "drum" => Some(SampleSet::Drum),
        _ => None,
    }
}

fn std_parts(lane: &SampleIdentity) -> Option<(SampleSet, Hitsound, CustomIndex)> {
    match lane {
        SampleIdentity::Standard {
            set,
            sound: Sound::Hit(sound),
            index,
        } => Some((*set, *sound, *index)),
        _ => None,
    }
}

/// Picks which underlying member lane in `members` should receive an edit at `at_ms`.
///
/// Priority:
/// 1. Existing co-timed hit within 5 ms (matching `(set, index)` or `index`),
/// 2. Active timing point at `at_ms + CONTROL_POINT_LENIENCY`,
/// 3. `preferred` (when in `members`), else `members[0]`.
pub(crate) fn choose_member_lane(
    hits: &Hitsounds,
    file: &OsuFile,
    at_ms: f64,
    members: &[SampleIdentity],
    preferred: Option<&SampleIdentity>,
) -> SampleIdentity {
    if members.is_empty() {
        return preferred.cloned().unwrap_or(SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Normal),
            index: CustomIndex::default(),
        });
    }
    if members.len() == 1 {
        return members[0].clone();
    }

    let is_normal_member = std_parts(&members[0])
        .is_some_and(|(_, sound, _)| sound == Hitsound::Normal);

    // 1. Moment match within 5 ms.
    if let Some(moment_id) = nearest_moment(hits, at_ms, 5.0) {
        let mut normals: Vec<(SampleSet, CustomIndex)> = Vec::new();
        let mut additions: Vec<(SampleSet, CustomIndex)> = Vec::new();
        let mut all: Vec<(SampleSet, CustomIndex)> = Vec::new();
        for hit_lane in hits.moments()[moment_id.0].hits() {
            if let Some((set, sound, index)) = std_parts(hit_lane) {
                all.push((set, index));
                if sound == Hitsound::Normal {
                    normals.push((set, index));
                } else {
                    additions.push((set, index));
                }
            }
        }
        if !all.is_empty() {
            let find_pair = |pairs: &[(SampleSet, CustomIndex)]| {
                members.iter().find(|m| {
                    std_parts(m).is_some_and(|(ms, _, mi)| pairs.contains(&(ms, mi)))
                })
            };
            let find_index = |pairs: &[(SampleSet, CustomIndex)]| {
                members.iter().find(|m| {
                    std_parts(m).is_some_and(|(_, _, mi)| pairs.iter().any(|&(_, pi)| pi == mi))
                })
            };
            let matched = if is_normal_member {
                find_pair(&all).or_else(|| find_index(&all))
            } else {
                find_pair(&additions)
                    .or_else(|| find_pair(&normals))
                    .or_else(|| find_index(&additions))
                    .or_else(|| find_index(&normals))
            };
            if let Some(lane) = matched {
                return lane.clone();
            }
        }
    }

    // 2. Active timing point match.
    let cp = ControlPoints::from_file(file);
    if let Some(tp) = cp.sample_point_at(at_ms + CONTROL_POINT_LENIENCY) {
        let tp_set = parse_tp_set(&tp.bank);
        let tp_index = tp.custom_sample_bank.max(0) as u32;
        if let Some(set) = tp_set
            && let Some(exact) = members.iter().find(|m| {
                std_parts(m).is_some_and(|(ms, _, mi)| ms == set && mi.get() == tp_index)
            })
        {
            return exact.clone();
        }
        if tp_index > 0
            && let Some(by_idx) = members.iter().find(|m| {
                std_parts(m).is_some_and(|(_, _, mi)| mi.get() == tp_index)
            })
        {
            return by_idx.clone();
        }
        if let Some(set) = tp_set
            && let Some(by_set) = members
                .iter()
                .enumerate()
                .filter_map(|(pos, m)| {
                    std_parts(m)
                        .filter(|(ms, _, _)| *ms == set)
                        .map(|(_, _, mi)| ((mi.get(), pos), m))
                })
                .min_by_key(|&(order, _)| order)
                .map(|(_, m)| m)
        {
            return by_set.clone();
        }
    }

    // 3. Fallback: keep preferred if it belongs to this merged group, else primary.
    if let Some(pref) = preferred
        && members.contains(pref)
    {
        return pref.clone();
    }
    members[0].clone()
}

pub(crate) struct DrumGroupContext {
    coord_lanes: Vec<Vec<SampleIdentity>>,
    all_group_lanes: Vec<Vec<SampleIdentity>>,
    group_of_identity: HashMap<SampleIdentity, usize>,
    coord_of_group: HashMap<usize, usize>,
    row_to_coord: HashMap<usize, usize>,
}

impl DrumGroupContext {
    pub(crate) fn new(
        groups: &DrumRowGroups,
        composer_lanes: &[SampleIdentity],
        visible_rows: Option<&[usize]>,
    ) -> Self {
        let mut group_of_identity = HashMap::new();
        let all_group_lanes: Vec<Vec<SampleIdentity>> = groups
            .groups
            .iter()
            .enumerate()
            .map(|(g, member_rows)| {
                member_rows
                    .iter()
                    .filter_map(|&r| {
                        let id = composer_lanes.get(r)?.clone();
                        group_of_identity.entry(id.clone()).or_insert(g);
                        Some(id)
                    })
                    .collect()
            })
            .collect();

        let mut active_group_ids: Vec<usize> = Vec::new();
        if let Some(vis) = visible_rows.filter(|v| !v.is_empty()) {
            let mut seen = HashSet::new();
            for &r in vis {
                if let Some(&g) = groups.group_of_lane.get(&r)
                    && seen.insert(g)
                {
                    active_group_ids.push(g);
                }
            }
        }
        if active_group_ids.is_empty() {
            active_group_ids = (0..groups.groups.len()).collect();
        }

        let mut coord_lanes = Vec::with_capacity(active_group_ids.len());
        let mut coord_of_group = HashMap::with_capacity(active_group_ids.len());
        let mut row_to_coord = HashMap::new();
        for (coord, &g) in active_group_ids.iter().enumerate() {
            coord_of_group.insert(g, coord);
            coord_lanes.push(all_group_lanes[g].clone());
            for &r in &groups.groups[g] {
                row_to_coord.entry(r).or_insert(coord);
            }
        }

        Self {
            coord_lanes,
            all_group_lanes,
            group_of_identity,
            coord_of_group,
            row_to_coord,
        }
    }

    pub(crate) fn translate_pointer(&self, mut input: PointerInput) -> PointerInput {
        if input.surface == PointerSurface::Grid {
            input.row = usize::try_from(input.row)
                .ok()
                .and_then(|r| self.row_to_coord.get(&r).copied())
                .map_or(-1, |c| c as i32);
        }
        input
    }
}

pub(crate) struct DrumRowTarget<'a> {
    pub composer: &'a mut Composer,
    pub file: &'a OsuFile,
    pub ctx: DrumGroupContext,
    pub move_stroke: &'a mut Option<DrumMoveStroke>,
}

impl ToolTarget for DrumRowTarget<'_> {
    type Item = HitKey;

    fn hit_test(&self, at: &PointerInput) -> Option<HitKey> {
        let coord = usize::try_from(at.row).ok()?;
        let lanes = self.ctx.coord_lanes.get(coord)?;
        let mut best: Option<((f64, usize), HitKey)> = None;
        for (moment_idx, m) in self.composer.hitsounds().moments().iter().enumerate() {
            let dist = (m.time().ms() - at.time.ms()).abs();
            if dist > 10.0 {
                continue;
            }
            for (pos, lane) in lanes.iter().enumerate() {
                if m.has_hit(lane) {
                    let score = (dist, pos);
                    if best
                        .as_ref()
                        .is_none_or(|(b_score, _)| {
                            score.0.total_cmp(&b_score.0).then_with(|| score.1.cmp(&b_score.1)).is_lt()
                        })
                    {
                        best = Some((
                            score,
                            HitKey {
                                moment: moment_idx,
                                lane: lane.clone(),
                            },
                        ));
                    }
                }
            }
        }
        best.map(|(_, k)| k)
    }

    fn in_range(&self, from: SongTime, to: SongTime, first: i32, last: i32) -> Vec<HitKey> {
        let lo = from.ms().min(to.ms());
        let hi = from.ms().max(to.ms());
        let mut keys = Vec::new();
        if first == i32::MIN && last == i32::MAX {
            for (i, m) in self.composer.hitsounds().moments().iter().enumerate() {
                if m.time().ms() < lo || m.time().ms() > hi {
                    continue;
                }
                for lane in m.hits() {
                    keys.push(HitKey {
                        moment: i,
                        lane: lane.clone(),
                    });
                }
            }
            return keys;
        }
        if self.ctx.coord_lanes.is_empty() || last < 0 {
            return keys;
        }
        let max_c = (self.ctx.coord_lanes.len() - 1) as i32;
        let lo_c = first.max(0);
        let hi_c = last.min(max_c);
        if lo_c > hi_c {
            return keys;
        }
        let allowed: HashSet<&SampleIdentity> = self.ctx.coord_lanes[lo_c as usize..=hi_c as usize]
            .iter()
            .flatten()
            .collect();
        if allowed.is_empty() {
            return keys;
        }
        for (i, m) in self.composer.hitsounds().moments().iter().enumerate() {
            if m.time().ms() < lo || m.time().ms() > hi {
                continue;
            }
            for lane in m.hits() {
                if allowed.contains(lane) {
                    keys.push(HitKey {
                        moment: i,
                        lane: lane.clone(),
                    });
                }
            }
        }
        keys
    }

    fn item_time(&self, item: &HitKey) -> SongTime {
        ToolTarget::item_time(self.composer, item)
    }

    fn selection(&mut self) -> &mut Selection<HitKey> {
        ToolTarget::selection(self.composer)
    }

    fn snap(&self, time: SongTime) -> SongTime {
        ToolTarget::snap(self.composer, time)
    }

    fn snap_steps(&self, from: SongTime, to: SongTime) -> Vec<SongTime> {
        ToolTarget::snap_steps(self.composer, from, to)
    }

    fn snap_offset(&self, from: SongTime, raw_ms: f64) -> f64 {
        ToolTarget::snap_offset(self.composer, from, raw_ms)
    }

    fn snap_offset_relative(&self, from: SongTime, raw_ms: f64) -> f64 {
        ToolTarget::snap_offset_relative(self.composer, from, raw_ms)
    }

    fn begin(&mut self, label: &str) {
        *self.move_stroke = None;
        ToolTarget::begin(self.composer, label);
    }

    fn commit(&mut self) {
        *self.move_stroke = None;
        ToolTarget::commit(self.composer);
    }

    fn cancel(&mut self) {
        *self.move_stroke = None;
        ToolTarget::cancel(self.composer);
    }

    fn insert(&mut self, at: SongTime, row: i32) -> Option<HitKey> {
        let coord = usize::try_from(row).ok()?;
        let lanes = self.ctx.coord_lanes.get(coord)?;
        if lanes.is_empty() {
            return None;
        }
        let volume = self.composer.insert_volume;
        let existing = nearest_moment(self.composer.hitsounds(), at.ms(), 5.0);
        if let Some(id) = existing
            && let Some(occupied) = lanes
                .iter()
                .find(|l| self.composer.hitsounds().moments()[id.0].has_hit(l))
        {
            return Some(HitKey {
                moment: id.0,
                lane: occupied.clone(),
            });
        }
        let chosen = choose_member_lane(self.composer.hitsounds(), self.file, at.ms(), lanes, None);
        let mut moment = 0;
        let _ = self.composer.history.edit(true, |h| {
            let id = existing.unwrap_or_else(|| h.add_moment(at, volume));
            h.add_hit(id, chosen.clone());
            moment = id.0;
            Ok::<_, Infallible>(())
        });
        self.composer.audition_requests.push(chosen.clone());
        Some(HitKey {
            moment,
            lane: chosen,
        })
    }

    fn delete(&mut self, items: &[HitKey]) {
        let mut removable: Vec<(MomentId, SampleIdentity)> = Vec::new();
        for key in items {
            let Some(moment) = self.composer.hitsounds().moments().get(key.moment) else {
                continue;
            };
            let candidates: &[SampleIdentity] = match self.ctx.group_of_identity.get(&key.lane) {
                Some(&g) => &self.ctx.all_group_lanes[g],
                None => std::slice::from_ref(&key.lane),
            };
            for lane in candidates {
                let candidate_key = HitKey {
                    moment: key.moment,
                    lane: lane.clone(),
                };
                if moment.has_hit(lane)
                    && can_remove_hit(self.composer, &candidate_key)
                    && !removable
                        .iter()
                        .any(|(m, l)| m.0 == key.moment && l == lane)
                {
                    removable.push((MomentId(key.moment), lane.clone()));
                }
            }
        }
        if removable.is_empty() {
            return;
        }
        let _ = self.composer.history.edit(true, |hits| {
            for (moment, lane) in &removable {
                hits.remove_hit(*moment, lane);
            }
            Ok::<_, Infallible>(())
        });
        for (moment, lane) in removable {
            self.composer.selected.remove(&HitKey {
                moment: moment.0,
                lane,
            });
        }
    }

    fn move_by(&mut self, items: &[HitKey], delta: f64, rows: i32) {
        if items.is_empty() || self.ctx.coord_lanes.is_empty() {
            return;
        }
        let primary = self.composer.selected.primary().cloned();
        if self.move_stroke.is_none() {
            let base_hits = self.composer.hitsounds().clone();
            let mut values: Vec<_> = items
                .iter()
                .filter_map(|k| {
                    let moment = base_hits.moments().get(k.moment)?;
                    moment
                        .has_hit(&k.lane)
                        .then(|| (k.clone(), moment.time().ms(), moment.hit_volume(&k.lane)))
                })
                .collect();
            if values.is_empty() {
                return;
            }
            values.sort_unstable_by(|(a, _, _), (b, _, _)| {
                (Some(a) != primary.as_ref(), a.moment, &a.lane)
                    .cmp(&(Some(b) != primary.as_ref(), b.moment, &b.lane))
            });
            *self.move_stroke = Some((base_hits, values, 0.0, 0));
        }
        let Some((base_hits, values, total_delta, total_rows)) = self.move_stroke.as_mut() else {
            return;
        };
        *total_delta += delta;
        *total_rows += rows;
        let base_snapshot = base_hits.clone();
        let snapshot_values = values.clone();
        let applied_delta = *total_delta;
        let applied_rows = *total_rows;
        let mut new = Vec::new();
        let _ = self.composer.history.edit(true, |h| {
            *h = base_snapshot;
            for (k, _, _) in &snapshot_values {
                h.remove_hit(MomentId(k.moment), &k.lane);
            }
            let max_c = self.ctx.coord_lanes.len() as i32 - 1;
            for (k, t, v) in snapshot_values {
                let orig_g = self.ctx.group_of_identity.get(&k.lane).copied();
                let dest_members: &[SampleIdentity] = if let Some(g) = orig_g
                    && let Some(&old_c) = self.ctx.coord_of_group.get(&g)
                {
                    let new_c = (old_c as i32 + applied_rows).clamp(0, max_c) as usize;
                    &self.ctx.coord_lanes[new_c]
                } else if applied_rows == 0
                    && let Some(g) = orig_g
                {
                    &self.ctx.all_group_lanes[g]
                } else if applied_rows != 0 {
                    let new_c = applied_rows.clamp(0, max_c) as usize;
                    &self.ctx.coord_lanes[new_c]
                } else {
                    std::slice::from_ref(&k.lane)
                };
                let dest_ms = (t + applied_delta).max(0.0);
                if let Some(existing) = nearest_moment(h, dest_ms, 5.0)
                    && dest_members
                        .iter()
                        .any(|l| h.moments()[existing.0].has_hit(l))
                {
                    continue;
                }
                let preferred = (applied_rows == 0).then_some(&k.lane);
                let lane = choose_member_lane(h, self.file, dest_ms, dest_members, preferred);
                let moment = nearest_moment(h, dest_ms, 5.0)
                    .unwrap_or_else(|| h.add_moment(SongTime::from_ms(dest_ms), v));
                h.add_hit_with_volume(moment, lane.clone(), v);
                new.push(HitKey {
                    moment: moment.0,
                    lane,
                });
            }
            Ok::<_, Infallible>(())
        });
        if rows != 0 {
            self.composer
                .audition_requests
                .extend(new.iter().map(|k| k.lane.clone()));
        }
        self.composer.selected.replace(new);
    }

    fn set_values(&mut self, values: &[(HitKey, i32)]) {
        ToolTarget::set_values(self.composer, values);
    }

    fn insert_value(&self) -> i32 {
        ToolTarget::insert_value(self.composer)
    }

    fn toggle_listen_mute(&mut self, item: &HitKey) {
        let mute = !self.composer.listen_muted.contains(item);
        let candidates: Vec<SampleIdentity> = match self.ctx.group_of_identity.get(&item.lane) {
            Some(&g) => self.ctx.all_group_lanes[g].clone(),
            None => vec![item.lane.clone()],
        };
        let co_timed: Vec<HitKey> = self
            .composer
            .hitsounds()
            .moments()
            .get(item.moment)
            .map(|moment| {
                candidates
                    .into_iter()
                    .filter(|lane| moment.has_hit(lane))
                    .map(|lane| HitKey {
                        moment: item.moment,
                        lane,
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![item.clone()]);
        for key in co_timed {
            if mute {
                if !self.composer.listen_muted.contains(&key) {
                    self.composer.listen_muted.push(key);
                }
            } else if let Some(pos) = self.composer.listen_muted.iter().position(|k| k == &key) {
                self.composer.listen_muted.remove(pos);
            }
        }
        self.composer.listen_version += 1;
    }

    fn audition(&mut self, item: &HitKey) {
        ToolTarget::audition(self.composer, item);
    }

    fn pan(&mut self, delta_ms: f64, delta_rows: f64) {
        ToolTarget::pan(self.composer, delta_ms, delta_rows);
    }

    fn set_range(&mut self, range: Option<TimeRange>) {
        ToolTarget::set_range(self.composer, range);
    }

    fn open_tool_picker(&mut self, at: &PointerInput) {
        ToolTarget::open_tool_picker(self.composer, at);
    }

    fn show_preview(&mut self, preview: ToolPreview) {
        ToolTarget::show_preview(self.composer, preview);
    }
}

pub(crate) fn paint_hits_on_group(
    composer: &mut Composer,
    file: &OsuFile,
    member_rows: &[usize],
    start_ms: f64,
    end_ms: f64,
) -> usize {
    let members: Vec<SampleIdentity> = member_rows
        .iter()
        .filter_map(|&r| composer.lanes.get(r).cloned())
        .collect();
    if members.is_empty() {
        return 0;
    }
    let steps = composer.paint_stroke_steps(start_ms, end_ms);
    if steps.is_empty() {
        return 0;
    }
    let mut to_place: Vec<SongTime> = Vec::with_capacity(steps.len());
    for at in steps {
        if to_place
            .iter()
            .any(|prev| (prev.ms() - at.ms()).abs() < 5.0)
        {
            continue;
        }
        if let Some(id) = nearest_moment(composer.hitsounds(), at.ms(), 5.0)
            && members
                .iter()
                .any(|m| composer.hitsounds().moments()[id.0].has_hit(m))
        {
            continue;
        }
        to_place.push(at);
    }
    if to_place.is_empty() {
        return 0;
    }
    let volume = composer.insert_volume;
    let mut placed = 0usize;
    composer.history.begin("Paint draw stroke");
    let _ = composer.history.edit(true, |h| {
        for at in &to_place {
            let existing = nearest_moment(h, at.ms(), 5.0);
            if let Some(id) = existing
                && members.iter().any(|m| h.moments()[id.0].has_hit(m))
            {
                continue;
            }
            let lane = choose_member_lane(h, file, at.ms(), &members, None);
            let id = existing.unwrap_or_else(|| h.add_moment(*at, volume));
            h.add_hit_with_volume(id, lane, volume);
            placed += 1;
        }
        Ok::<_, Infallible>(())
    });
    composer.history.commit();
    if placed > 0 {
        composer.audition_requests.push(members[0].clone());
    }
    placed
}

pub(crate) fn toggle_step_hit_group(
    composer: &mut Composer,
    file: &OsuFile,
    member_rows: &[usize],
    ms: f64,
) -> bool {
    if !ms.is_finite() || ms < 0.0 {
        return false;
    }
    let members: Vec<SampleIdentity> = member_rows
        .iter()
        .filter_map(|&r| composer.lanes.get(r).cloned())
        .collect();
    if members.is_empty() {
        return false;
    }
    if let Some(id) = nearest_moment(composer.hitsounds(), ms, 4.0) {
        let present: Vec<SampleIdentity> = members
            .iter()
            .filter(|m| composer.hitsounds().moments()[id.0].has_hit(m))
            .cloned()
            .collect();
        if !present.is_empty() {
            composer.history.begin("Remove step hit");
            let _ = composer.history.edit(true, |h| {
                for lane in &present {
                    h.remove_hit(id, lane);
                }
                Ok::<_, Infallible>(())
            });
            composer.history.commit();
            for lane in present {
                composer.selected.remove(&HitKey { moment: id.0, lane });
            }
            return true;
        }
    }
    let target = if composer.snap_mode == SnapMode::Objects {
        SongTime::from_ms(ms)
    } else {
        composer.snap(SongTime::from_ms(ms))
    };
    let volume = composer
        .insert_volume
        .clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
    let mut audition_lane = None;
    composer.history.begin("Add step hit");
    let _ = composer.history.edit(true, |h| {
        let id = nearest_moment(h, target.ms(), 4.0)
            .or_else(|| nearest_moment(h, ms, 4.0))
            .unwrap_or_else(|| h.add_moment(target, volume));
        let lane = choose_member_lane(h, file, h.moments()[id.0].time().ms(), &members, None);
        h.add_hit_with_volume(id, lane.clone(), volume);
        audition_lane = Some(lane);
        Ok::<_, Infallible>(())
    });
    composer.history.commit();
    if let Some(lane) = audition_lane {
        composer.audition_requests.push(lane);
    }
    true
}

pub(crate) fn select_across_drum_groups(
    composer: &mut Composer,
    rows: &[usize],
    groups: &DrumRowGroups,
    direction: isize,
    max_distance: f64,
) {
    let mut keys: Vec<HitKey> = composer.selected.items().iter().cloned().collect();
    let primary = composer.selected.primary();
    keys.sort_unstable_by(|a, b| {
        (Some(a) != primary, a.moment, &a.lane).cmp(&(Some(b) != primary, b.moment, &b.lane))
    });
    let moments = composer.hitsounds().moments();
    let Some((anchor, time)) = keys
        .iter()
        .filter_map(|key| {
            let moment = moments.get(key.moment)?;
            moment.has_hit(&key.lane).then(|| (key, moment.time().ms()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return;
    };
    let Some(anchor_row) = composer.lanes.iter().position(|l| l == &anchor.lane) else {
        return;
    };
    let Some(&anchor_g) = groups.group_of_lane.get(&anchor_row) else {
        composer.select_across_lanes(rows, direction, max_distance);
        return;
    };
    let row_groups: Vec<Option<usize>> = rows
        .iter()
        .map(|&r| groups.group_of_lane.get(&r).copied())
        .collect();
    let Some(start) = row_groups.iter().position(|&g| g == Some(anchor_g)) else {
        return;
    };

    let mut rank: HashMap<SampleIdentity, usize> = HashMap::new();
    for offset in 1..rows.len() {
        let Some(idx) = start.checked_add_signed(offset as isize * direction) else {
            break;
        };
        let Some(&row) = rows.get(idx) else {
            break;
        };
        if let Some(&g) = groups.group_of_lane.get(&row)
            && g != anchor_g
        {
            for &member_row in &groups.groups[g] {
                if let Some(lane) = composer.lanes.get(member_row) {
                    rank.entry(lane.clone()).or_insert(offset);
                }
            }
        } else if let Some(lane) = composer.lanes.get(row)
            && lane != &anchor.lane
        {
            rank.entry(lane.clone()).or_insert(offset);
        }
    }

    let mut closest = None;
    for (index, moment) in moments.iter().enumerate() {
        let distance = (moment.time().ms() - time).abs();
        if distance > max_distance {
            continue;
        }
        for lane in moment.hits() {
            let Some(&lane_rank) = rank.get(lane) else {
                continue;
            };
            let score = (lane_rank, distance, index);
            if closest
                .as_ref()
                .is_none_or(|(best, _)| score < *best)
            {
                closest = Some((score, lane.clone()));
            }
        }
    }
    if let Some(((_, _, moment), lane)) = closest {
        let hit = HitKey { moment, lane };
        if !composer.selected.contains(&hit) {
            composer.selected.replace(keys.into_iter().chain([hit]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::SongTime;
    use dawsu_hitsounding::{Decompiled, decompile};
    use dawsu_osu_format::OsuFile;

    fn std_id(set: SampleSet, sound: Hitsound, index: u32) -> SampleIdentity {
        SampleIdentity::Standard {
            set,
            sound: Sound::Hit(sound),
            index: CustomIndex::new(index),
        }
    }

    fn decompile_fixture(osu_text: &str, files: &[&str]) -> (Hitsounds, OsuFile) {
        let parsed = OsuFile::read(osu_text.as_bytes().to_vec()).file;
        let folder: Vec<String> = files.iter().map(|s| (*s).to_string()).collect();
        let mut progress = |_, _| {};
        let Decompiled::Done(hits) = decompile(&parsed, &folder, &|| false, &mut progress) else {
            panic!("expected osu standard fixture to decompile");
        };
        (hits, parsed)
    }

    #[test]
    fn choose_member_lane_follows_moment_then_timing_point_then_preferred_or_primary() {
        let osu = "osu file format v14\n\n[General]\nMode:0\nSampleSet:Drum\n\n[TimingPoints]\n0,500,4,3,1,100,1,0\n1000,-100,4,2,2,100,0,0\n2000,-100,4,3,0,100,0,0\n\n[HitObjects]\n256,192,500,1,8,1:2:2:0:\n";
        let (mut hits, file) = decompile_fixture(
            osu,
            &[
                "drum-hitnormal.wav",
                "drum-hitnormal2.wav",
                "drum-hitnormal3.wav",
                "normal-hitclap2.wav",
            ],
        );

        let d1 = std_id(SampleSet::Drum, Hitsound::Normal, 1);
        let d2 = std_id(SampleSet::Drum, Hitsound::Normal, 2);
        let d3 = std_id(SampleSet::Drum, Hitsound::Normal, 3);
        let s2 = std_id(SampleSet::Soft, Hitsound::Normal, 2);
        let members = vec![d1.clone(), d2.clone(), d3.clone()];

        // 1. Moment match at 500 ms: existing addition is Clap index 2 -> index matches d2 (over d1 primary).
        assert_eq!(choose_member_lane(&hits, &file, 500.0, &members, None), d2);

        // If an exact (Soft, 2) member is present at 500 ms (Soft Clap 2) or 750 ms (Soft Normal 2), exact pair wins over index-only match.
        let members_with_s2 = vec![d1.clone(), d2.clone(), d3.clone(), s2.clone()];
        assert_eq!(
            choose_member_lane(&hits, &file, 500.0, &members_with_s2, None),
            s2
        );
        let clap_s2 = std_id(SampleSet::Soft, Hitsound::Clap, 2);
        let clap_members = vec![
            std_id(SampleSet::Normal, Hitsound::Clap, 2),
            clap_s2.clone(),
        ];
        let m_clap = hits.add_moment(SongTime::from_ms(750.0), 100);
        hits.add_hit(m_clap, s2.clone());
        assert_eq!(
            choose_member_lane(&hits, &file, 750.0, &clap_members, None),
            clap_s2
        );

        // 2. Active timing point match at 1200 ms: (Soft, 2) active -> picks s2.
        assert_eq!(
            choose_member_lane(&hits, &file, 1200.0, &members_with_s2, None),
            s2
        );

        // At 2000 ms (Drum, index 0): Skin (Drum, 0) exact match wins if present.
        let d0 = std_id(SampleSet::Drum, Hitsound::Normal, 0);
        let with_skin = vec![d2.clone(), d1.clone(), d0.clone()];
        assert_eq!(
            choose_member_lane(&hits, &file, 2000.0, &with_skin, None),
            d0
        );

        // 3. Fallback when neither moment nor timing point narrows within members -> preferred else primary.
        let generic_members = vec![
            SampleIdentity::File("kick-a.wav".into()),
            SampleIdentity::File("kick-b.wav".into()),
        ];
        assert_eq!(
            choose_member_lane(
                &hits,
                &file,
                1500.0,
                &generic_members,
                Some(&generic_members[1])
            ),
            generic_members[1]
        );
        assert_eq!(
            choose_member_lane(&hits, &file, 1500.0, &generic_members, None),
            generic_members[0]
        );
    }
}

