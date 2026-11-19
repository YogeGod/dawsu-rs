//! HS mode in compile (hitsound domain design doc Responsibility 14; the last-four-parts plan,
//! part B). While HS mode is on for a hitsound difficulty, every moment that has hits but no object
//! gets a centred circle, as the old app's `OsuFile.InsertHitCircle` wrote it. This is the only
//! place the module creates objects; plain `compile` never does (invariant 6). Removing the circles
//! the editor made, once they lose their last hit, is the editor's (M3).

use crate::compile::{compile, mix_problem, Compiled, Placement, SaveProblem};
use crate::copy::set_moment;
use crate::decompile::{decompile, Decompiled};
use crate::lazer::control_points::offset_for;
use crate::model::{Hitsounds, MomentId, ObjectId, Place};
use crate::rules::saved_volume;
use dawsu_foundation::SampleIdentity;
use dawsu_osu_format::OsuFile;
use std::collections::{BTreeMap, BTreeSet};

/// The outcome of compile in HS mode.
#[derive(Debug, Clone)]
pub enum CompiledHs {
    /// The patched difficulty, the moments it could not encode, and the circles it made, so the
    /// editor can keep them as its own.
    Done { file: OsuFile, problems: Vec<SaveProblem>, created: Vec<ObjectId> },
    Cancelled,
}

/// Compiles `hitsounds` into a copy of `file` in HS mode. Moments with hits and no object are
/// grouped by their file time rounded half to even to the millisecond; each group gets the circle
/// `256,192,<time>,1,0,0:0:0:0:`, inserted in time order, even beside an existing object at that
/// time. A group with more than one volume, or a lane twice, keeps a bare circle and is a
/// sample-mix save problem, as in Generate. Every other edit is carried onto the copy by object and
/// node, then compile writes it with `placement`.
pub fn compile_hs(file: &OsuFile, hitsounds: &Hitsounds, placement: Placement, cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> CompiledHs {
    let offset = offset_for(file.format_version());
    let mut groups: BTreeMap<i64, Vec<(SampleIdentity, i32)>> = BTreeMap::new();
    for m in hitsounds.moments().iter().filter(|m| m.place() == Place::NoObject && m.hits().next().is_some()) {
        groups.entry((m.time().ms() - offset).round_ties_even() as i64).or_default().extend(m.hits().map(|h| (h.clone(), m.hit_volume(h))));
    }
    if groups.is_empty() {
        return match compile(file, hitsounds, placement, cancel, progress) {
            Compiled::Done { file, problems } => CompiledHs::Done { file, problems, created: Vec::new() },
            Compiled::Cancelled => CompiledHs::Cancelled,
        };
    }

    let mut copy = file.clone();
    let created: Vec<ObjectId> = groups.keys().map(|t| ObjectId(copy.insert_hit_object(&format!("256,192,{t},1,0,0:0:0:0:")))).collect();
    let mut target = match decompile(&copy, &hitsounds.folder_files, cancel, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        Decompiled::Cancelled => return CompiledHs::Cancelled,
        // HS mode is for osu! difficulties; elsewhere it places nothing and compiles as it is.
        Decompiled::UnsupportedMode { .. } => {
            return match compile(file, hitsounds, placement, cancel, progress) {
                Compiled::Done { file, problems } => CompiledHs::Done { file, problems, created: Vec::new() },
                Compiled::Cancelled => CompiledHs::Cancelled,
            };
        }
    };

    // Line ids are stable, so a place names the same object and node in the copy.
    let at: BTreeMap<String, MomentId> = target.moments().iter().enumerate().map(|(i, m)| (format!("{:?}", m.place()), MomentId(i))).collect();
    for b in hitsounds.bodies().iter().filter(|b| b.is_changed()) {
        if let Some(i) = target.bodies().iter().position(|t| t.object() == b.object()) {
            if b.whistle_changed() {
                target.set_body_whistle(i, b.whistle());
            }
            if b.set_changed() && let Some(set) = b.set() {
                target.set_body_set(i, set);
            }
            if b.whistle_set_changed() {
                target.set_body_whistle_set(i, b.whistle_set);
            }
            if b.volume_changed() {
                target.set_body_volume(i, b.volume());
            }
        }
    }
    for m in hitsounds.moments().iter().filter(|m| m.is_changed() && m.place() != Place::NoObject) {
        if let Some(&id) = at.get(&format!("{:?}", m.place())) {
            set_moment(&mut target, id, &m.hits().cloned().collect::<Vec<_>>(), m.volume());
            for lane in m.hits() {
                target.set_hit_volume(id, lane, m.hit_volume(lane));
            }
        }
    }
    let mut problems = Vec::new();
    for (circle, layers) in created.iter().zip(groups.values()) {
        if cancel() {
            return CompiledHs::Cancelled;
        }
        let Some(&id) = at.get(&format!("{:?}", Place::Circle(*circle))) else { continue };
        let volumes: BTreeSet<i32> = layers.iter().map(|l| saved_volume(l.1)).collect();
        let lanes: BTreeSet<SampleIdentity> = layers.iter().map(|l| l.0.clone()).collect();
        if volumes.len() > 1 || lanes.len() < layers.len() {
            let m = target.moment(id);
            problems.push(SaveProblem { time: m.time(), place: m.place(), kind: mix_problem(layers) });
        } else {
            set_moment(&mut target, id, &lanes.into_iter().collect::<Vec<_>>(), layers[0].1);
        }
    }
    match compile(&copy, &target, placement, cancel, progress) {
        Compiled::Done { file, problems: more } => {
            problems.extend(more);
            problems.sort_by(|a, b| a.time.ms().total_cmp(&b.time.ms()));
            CompiledHs::Done { file, problems, created }
        }
        Compiled::Cancelled => CompiledHs::Cancelled,
    }
}
