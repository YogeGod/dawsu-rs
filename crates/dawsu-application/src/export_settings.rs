use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, Sound};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const SLOTS: [&str; 12] = [
    "normal-hitnormal",
    "normal-hitwhistle",
    "normal-hitfinish",
    "normal-hitclap",
    "soft-hitnormal",
    "soft-hitwhistle",
    "soft-hitfinish",
    "soft-hitclap",
    "drum-hitnormal",
    "drum-hitwhistle",
    "drum-hitfinish",
    "drum-hitclap",
];

#[derive(Serialize)]
pub struct ExportCombination {
    pub(crate) key: String,
    pub(crate) time: f64,
    pub(crate) layers: Vec<String>,
    pub(crate) destination: String,
}

/// C# ExportMergeSettings v1. The style is kept for round-tripping the preset.
/// Generate always uses the accepted on-object placement.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ExportSettings {
    pub version: u32,
    pub pair_samples: bool,
    pub style: u32,
    pub priorities: Vec<String>,
    pub rules: BTreeMap<String, String>,
    pub occurrence_rules: BTreeMap<String, String>,
    pub keep_original_rules: BTreeMap<String, bool>,
    pub occurrence_keep_original: BTreeMap<String, bool>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            version: 1,
            pair_samples: false,
            style: 0,
            priorities: SLOTS.iter().map(|slot| (*slot).to_string()).collect(),
            rules: BTreeMap::new(),
            occurrence_rules: BTreeMap::new(),
            keep_original_rules: BTreeMap::new(),
            occurrence_keep_original: BTreeMap::new(),
        }
    }
}

impl ExportSettings {
    pub fn read_file(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|error| error.to_string())?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("The merge preset is larger than 4 MiB.".into());
        }
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let settings: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("Could not read merge preset: {error}"))?;
        settings.checked_bytes()?;
        Ok(settings)
    }

    pub fn checked_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("The merge preset is larger than 4 MiB.".into());
        }
        Ok(bytes)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let order = self
            .priorities
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if self.version != 1
            || self.style > 1
            || self.priorities.len() != SLOTS.len()
            || !SLOTS.iter().all(|slot| order.contains(slot))
        {
            return Err(
                "A version 1 merge preset needs each of the twelve sample slots exactly once."
                    .into(),
            );
        }
        if [
            self.rules.len(),
            self.occurrence_rules.len(),
            self.keep_original_rules.len(),
            self.occurrence_keep_original.len(),
        ]
        .into_iter()
        .any(|count| count > 10_000)
            || self.rules.iter().any(|(key, slot)| {
                key.len() > 65_536 || !SLOTS.contains(&slot.as_str()) || !valid_combination(key)
            })
            || self
                .occurrence_rules
                .iter()
                .any(|(key, slot)| key.len() > 131_072 || !SLOTS.contains(&slot.as_str()))
            || self
                .keep_original_rules
                .keys()
                .chain(self.occurrence_keep_original.keys())
                .any(|key| key.len() > 131_072)
        {
            return Err("The merge preset contains an invalid combination rule.".into());
        }
        Ok(())
    }

    pub(crate) fn priority(&self, lanes: &[SampleIdentity]) -> &str {
        let mut additions = BTreeSet::new();
        let mut normals = BTreeSet::new();
        for lane in lanes {
            if let SampleIdentity::Standard {
                set,
                sound: Sound::Hit(role),
                ..
            } = *lane
            {
                let name = slot_name(set, role);
                if role == Hitsound::Normal {
                    normals.insert(name);
                } else {
                    additions.insert(name);
                }
            }
        }
        let participating = if additions.is_empty() {
            normals
        } else {
            additions
        };
        self.priorities
            .iter()
            .find(|slot| participating.contains(slot.as_str()))
            .unwrap_or(&self.priorities[0])
    }
}

fn valid_combination(key: &str) -> bool {
    serde_json::from_str::<Vec<String>>(key).is_ok_and(|labels| {
        !labels.is_empty()
            && labels.iter().all(|label| !label.trim().is_empty())
            && labels.windows(2).all(|pair| pair[0] <= pair[1])
    })
}

pub(crate) fn slot_name(set: SampleSet, role: Hitsound) -> &'static str {
    let bank = match set {
        SampleSet::Normal => 0,
        SampleSet::Soft => 4,
        SampleSet::Drum => 8,
    };
    SLOTS[bank
        + match role {
            Hitsound::Normal => 0,
            Hitsound::Whistle => 1,
            Hitsound::Finish => 2,
            Hitsound::Clap => 3,
        }]
}

pub(crate) fn slot(value: &str) -> Result<(SampleSet, Hitsound), String> {
    let index = SLOTS
        .iter()
        .position(|slot| *slot == value)
        .ok_or("Unknown merge sample slot.")?;
    Ok((
        [SampleSet::Normal, SampleSet::Soft, SampleSet::Drum][index / 4],
        [
            Hitsound::Normal,
            Hitsound::Whistle,
            Hitsound::Finish,
            Hitsound::Clap,
        ][index % 4],
    ))
}

type Combinations<T> = BTreeMap<Vec<String>, T>;
type Occurrences<T> = BTreeMap<String, Combinations<T>>;

pub(crate) struct Rules<'a> {
    rules: Combinations<&'a str>,
    occurrences: Occurrences<&'a str>,
    keep: Combinations<bool>,
    occurrence_keep: Occurrences<bool>,
}

impl<'a> Rules<'a> {
    pub(crate) fn new(settings: &'a ExportSettings) -> Self {
        Self {
            rules: combinations(
                settings
                    .rules
                    .iter()
                    .map(|(key, slot)| (key.as_str(), slot.as_str())),
            ),
            occurrences: occurrences(
                settings
                    .occurrence_rules
                    .iter()
                    .map(|(key, slot)| (key.as_str(), slot.as_str())),
            ),
            keep: combinations(
                settings
                    .keep_original_rules
                    .iter()
                    .map(|(key, keep)| (key.as_str(), *keep)),
            ),
            occurrence_keep: occurrences(
                settings
                    .occurrence_keep_original
                    .iter()
                    .map(|(key, keep)| (key.as_str(), *keep)),
            ),
        }
    }

    pub(crate) fn resolve(&self, time: f64, labels: &[String]) -> (Option<&'a str>, bool) {
        let time = time.to_string();
        let slot = self
            .occurrences
            .get(&time)
            .and_then(|rules| rules.get(labels))
            .or_else(|| self.rules.get(labels))
            .copied();
        let keep = self
            .occurrence_keep
            .get(&time)
            .and_then(|rules| rules.get(labels))
            .or_else(|| self.keep.get(labels))
            .copied()
            .unwrap_or(false);
        (slot, keep)
    }
}

fn combinations<'a, T>(rules: impl Iterator<Item = (&'a str, T)>) -> Combinations<T> {
    rules
        .filter_map(|(key, value)| Some((serde_json::from_str(key).ok()?, value)))
        .collect()
}

fn occurrences<'a, T>(rules: impl Iterator<Item = (&'a str, T)>) -> Occurrences<T> {
    let mut by_time = BTreeMap::<String, Combinations<T>>::new();
    for (key, value) in rules {
        if let Some((time, key)) = key.split_once('|')
            && let (Ok(time), Ok(labels)) = (time.parse::<f64>(), serde_json::from_str(key))
            && time.is_finite()
        {
            by_time
                .entry(time.to_string())
                .or_default()
                .insert(labels, value);
        }
    }
    by_time
}

pub(crate) fn balance(volume: i32, highest: i32) -> String {
    // C# divides decimal sample volumes. Compute the same representation for preset keys.
    if volume == highest {
        return "1".into();
    }
    let precision = 10_i128.pow(28);
    let scaled = i128::from(volume) * precision;
    let divisor = i128::from(highest);
    let rounded = (scaled + divisor / 2) / divisor;
    let decimal = format!("{rounded:028}");
    format!("0.{}", decimal.trim_end_matches('0'))
}

pub(crate) fn legacy_key(labels: &[String]) -> String {
    use std::fmt::Write;
    let mut json = String::from("[");
    for (i, label) in labels.iter().enumerate() {
        if i != 0 {
            json.push(',');
        }
        json.push('"');
        for unit in label.encode_utf16() {
            match unit {
                0x08 => json.push_str(r"\b"),
                0x09 => json.push_str(r"\t"),
                0x0a => json.push_str(r"\n"),
                0x0c => json.push_str(r"\f"),
                0x0d => json.push_str(r"\r"),
                0x5c => json.push_str(r"\\"),
                0x20..=0x7e if !matches!(unit, 0x22 | 0x26 | 0x27 | 0x2b | 0x3c | 0x3e | 0x60) => {
                    json.push(char::from_u32(u32::from(unit)).unwrap());
                }
                _ => write!(json, "\\u{unit:04X}").unwrap(),
            }
        }
        json.push('"');
    }
    json.push(']');
    json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_combination_keys_match_csharp_default_json_encoding() {
        let labels = [r#"abc+def \"sna"re é"#.to_string()];
        let encoded = legacy_key(&labels);
        assert_eq!(encoded, r#"["abc\u002Bdef \\\u0022sna\u0022re \u00E9"]"#);
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&encoded).unwrap(),
            labels
        );
    }
}
