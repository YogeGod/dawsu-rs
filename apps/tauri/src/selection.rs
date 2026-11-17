use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::{Hitsounds, Place};
use serde_json::{Value, json};
use std::collections::HashSet;

pub(super) fn summary<'a>(
    hits: &Hitsounds,
    selection: impl Iterator<Item = (usize, &'a SampleIdentity)>,
) -> Option<Value> {
    let mut count = 0;
    let mut start = f64::INFINITY;
    let mut anchored = HashSet::new();
    for (index, lane) in selection {
        let Some(moment) = hits
            .moments()
            .get(index)
            .filter(|moment| moment.has_hit(lane))
        else {
            continue;
        };
        count += 1;
        start = start.min(moment.time().ms());
        if moment.place() != Place::NoObject {
            anchored.insert(index);
        }
    }
    if count == 0 {
        return None;
    }
    let object = match anchored.len() {
        0 => "No object".into(),
        1 => {
            let moment = &hits.moments()[*anchored.iter().next().unwrap()];
            match moment.place() {
                Place::Circle(_) => "Circle",
                Place::SpinnerEnd(_) => "Spinner end",
                Place::SliderNode { node: 0, .. } => "Slider head",
                Place::SliderNode { object, .. }
                    if hits.bodies().iter().any(|body| {
                        body.object() == object
                            && (body.end().ms() - moment.time().ms()).abs() < 0.001
                    }) =>
                {
                    "Slider end"
                }
                Place::SliderNode { .. } => "Slider repeat",
                Place::NoObject => "No object",
            }
            .into()
        }
        count => format!("{count} object moments"),
    };
    Some(json!({"count": count, "start": start, "object": object}))
}
