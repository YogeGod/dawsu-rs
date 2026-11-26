use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_hitsounding::{Hitsounds, Place};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionAdditions {
    pub clap: &'static str,
    pub finish: &'static str,
    pub whistle: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectionSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<SelectionAdditions>,
    pub count: usize,
    #[serde(rename = "hasCustomFile", skip_serializing_if = "std::ops::Not::not")]
    pub has_custom_file: bool,
    pub object: String,
    #[serde(rename = "sampleSet", skip_serializing_if = "Option::is_none")]
    pub sample_set: Option<&'static str>,
    #[serde(rename = "sliderBody", skip_serializing_if = "Option::is_none")]
    pub slider_body: Option<SliderBodySelection>,
    pub start: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SliderBodySelection {
    pub index: usize,
    pub set: Option<&'static str>,
    pub start: f64,
    pub volume: i32,
    pub whistle: bool,
    pub whistle_set: &'static str,
}

pub fn summary<'a>(
    hits: &Hitsounds,
    selection: impl Iterator<Item = (usize, &'a SampleIdentity)>,
) -> Option<SelectionSummary> {
    let mut count = 0;
    let mut start = f64::INFINITY;
    let mut anchored = HashSet::new();
    let mut selected_moments = HashSet::new();
    let mut sets = HashSet::new();
    let mut has_custom_file = false;
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
        selected_moments.insert(index);
        if moment.place() != Place::NoObject {
            anchored.insert(index);
        }
        match lane {
            SampleIdentity::Standard { set, .. } => {
                sets.insert(*set);
            }
            SampleIdentity::File(_) => {
                has_custom_file = true;
            }
        }
    }
    if count == 0 {
        return None;
    }
    let sample_set = match sets.len() {
        0 => None,
        1 => sets.into_iter().next().map(|set| match set {
            SampleSet::Normal => "normal",
            SampleSet::Soft => "soft",
            SampleSet::Drum => "drum",
        }),
        _ => Some("mixed"),
    };
    let addition_state = |kind: Hitsound| -> &'static str {
        let mut with = 0usize;
        for &idx in &selected_moments {
            let has = hits.moments()[idx].hits().any(|lane| {
                matches!(lane, SampleIdentity::Standard { sound, .. } if *sound == Sound::Hit(kind))
            });
            if has {
                with += 1;
            }
        }
        if with == 0 {
            "off"
        } else if with == selected_moments.len() {
            "on"
        } else {
            "mixed"
        }
    };
    let additions = Some(SelectionAdditions {
        whistle: addition_state(Hitsound::Whistle),
        finish: addition_state(Hitsound::Finish),
        clap: addition_state(Hitsound::Clap),
    });
    let object = match anchored.len() {
        0 => "No object".into(),
        1 => {
            if let Some(&idx) = anchored.iter().next() {
                let moment = &hits.moments()[idx];
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
            } else {
                "No object".into()
            }
        }
        count => format!("{count} object moments"),
    };
    Some(SelectionSummary {
        count,
        object,
        slider_body: None,
        start,
        sample_set,
        additions,
        has_custom_file,
    })
}
