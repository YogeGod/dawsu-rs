use dawsu_editor::tempo::{BarBeat, TempoMap, TimingSection};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct EditorTimeSignature {
    pub bar: u32,
    pub numerator: u32,
    pub denominator: u32,
}

impl EditorTimeSignature {
    pub(crate) fn valid(&self) -> bool {
        self.bar >= 1
            && (1..=32).contains(&self.numerator)
            && matches!(self.denominator, 1 | 2 | 4 | 8 | 16)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EditorBarSection {
    pub start: f64,
    pub beat: f64,
    pub numerator: u32,
    pub denominator: u32,
    pub bar: i64,
}

impl EditorBarSection {
    pub(crate) fn position(&self, time: f64) -> BarBeat {
        let beats = (time - self.start) / self.beat;
        let whole = (beats + 1e-9).floor();
        let meter = self.numerator as i64;
        BarBeat {
            bar: self.bar.saturating_add((whole as i64).div_euclid(meter)),
            beat: (whole as i64).rem_euclid(meter) as u32 + 1,
            beat_fraction: (beats - whole).max(0.0),
        }
    }
}

pub(crate) fn normalise(signatures: &mut Vec<EditorTimeSignature>) {
    signatures.retain(EditorTimeSignature::valid);
    signatures.sort_by_key(|signature| signature.bar);
    signatures.dedup_by(|later, earlier| {
        if later.bar == earlier.bar {
            *earlier = *later;
            true
        } else {
            false
        }
    });
}

pub(crate) fn sections(
    tempo: &TempoMap,
    signatures: &[EditorTimeSignature],
) -> Vec<EditorBarSection> {
    if signatures.is_empty() {
        return Vec::new();
    }
    let timing = if tempo.sections().is_empty() {
        &[TimingSection::DEFAULT][..]
    } else {
        tempo.sections()
    };
    let mut result = Vec::new();
    let mut bar = 1_i64;
    for (i, red) in timing.iter().enumerate() {
        let mut start = red.start.ms();
        let end = timing
            .get(i + 1)
            .map_or(f64::INFINITY, |next| next.start.ms());
        loop {
            let applied = signatures.partition_point(|signature| i64::from(signature.bar) <= bar);
            let (numerator, denominator) = applied
                .checked_sub(1)
                .map_or((red.beats_per_bar, 4), |index| {
                    (signatures[index].numerator, signatures[index].denominator)
                });
            let beat = red.beat_length_ms * 4.0 / f64::from(denominator);
            let bar_length = beat * f64::from(numerator);
            if result.is_empty() && start > 0.0 {
                start -= (start / bar_length).ceil() * bar_length;
            }
            let split = signatures.get(applied).map_or(f64::INFINITY, |signature| {
                start + (i64::from(signature.bar) - bar) as f64 * bar_length
            });
            result.push(EditorBarSection {
                start,
                beat,
                numerator,
                denominator,
                bar,
            });
            if split < end - 1e-6 {
                start = split;
                bar = i64::from(signatures[applied].bar);
                continue;
            }
            if end.is_finite() {
                bar = bar.saturating_add(((end - start) / bar_length - 1e-6).ceil() as i64);
            }
            break;
        }
    }
    result
}
