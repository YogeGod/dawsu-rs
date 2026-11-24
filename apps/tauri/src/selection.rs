use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::{Hitsounds, Place};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Serialize)]
pub(super) struct SelectionSummary {
    pub(super) count: usize,
    pub(super) object: String,
    #[serde(rename = "sliderBody", skip_serializing_if = "Option::is_none")]
    pub(super) slider_body: Option<SliderBodySelection>,
    pub(super) start: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SliderBodySelection {
    pub(super) index: usize,
    pub(super) set: Option<&'static str>,
    pub(super) start: f64,
    pub(super) volume: i32,
    pub(super) whistle: bool,
    pub(super) whistle_set: &'static str,
}

pub(super) fn summary<'a>(
    hits: &Hitsounds,
    selection: impl Iterator<Item = (usize, &'a SampleIdentity)>,
) -> Option<SelectionSummary> {
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
    Some(SelectionSummary {
        count,
        object,
        slider_body: None,
        start,
    })
}
