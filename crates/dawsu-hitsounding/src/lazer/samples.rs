// Ported from osu!lazer (ppy/osu) at 131fe72a15581049401a588a13684938d4d0e0c7:
//   osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs (readCustomSampleBanks, convertSoundType,
//   SampleBankInfo, LegacyHitSampleInfo, FileHitSampleInfo)
//   osu.Game/Audio/HitSampleInfo.cs (LookupNames)
//   osu.Game/Beatmaps/ControlPoints/SampleControlPoint.cs and LegacyDecoder.LegacySampleControlPoint (ApplyTo)
//   osu.Game/Skinning/LegacySkin.cs (getLegacyLookupNames, getFallbackSampleNames)
//   osu.Game/Skinning/LegacyBeatmapSkin.cs (GetSample: beatmap samples only from index 1)
// Copyright (c) ppy Pty Ltd <contact@ppy.sh>. Licensed under the MIT Licence (ADR 0027).

use super::control_points::SamplePointData;
use dawsu_osu_format::HitSample;

pub const HIT_NORMAL: &str = "hitnormal";
pub const HIT_WHISTLE: &str = "hitwhistle";
pub const HIT_FINISH: &str = "hitfinish";
pub const HIT_CLAP: &str = "hitclap";
/// `SampleControlPoint.DEFAULT_BANK`.
pub const DEFAULT_BANK: &str = "normal";

const NORMAL: i32 = 1;
const WHISTLE: i32 = 2;
const FINISH: i32 = 4;
const CLAP: i32 = 8;

/// `SampleBankInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleBankInfo {
    pub filename: Option<String>,
    pub bank_for_normal: Option<String>,
    pub bank_for_additions: Option<String>,
    pub volume: i32,
    pub custom_sample_bank: i32,
    pub editor_auto_bank: bool,
}

impl Default for SampleBankInfo {
    fn default() -> SampleBankInfo {
        SampleBankInfo { filename: None, bank_for_normal: None, bank_for_additions: None, volume: 0, custom_sample_bank: 0, editor_auto_bank: true }
    }
}

/// A `LegacySampleBank` read from a hit sample: values outside the enum play Normal.
pub(crate) fn bank_of(n: i32) -> Option<String> {
    match n {
        1 => Some("normal".into()),
        2 => Some("soft".into()),
        3 => Some("drum".into()),
        0 => None,
        _ => Some("normal".into()),
    }
}

impl SampleBankInfo {
    /// `readCustomSampleBanks` applied to the bank info, from a hit sample the format crate read.
    /// `banks_only` is set for a slider's own hit sample, whose index, volume and file lazer ignores.
    pub fn read(mut self, sample: Option<&HitSample>, banks_only: bool) -> SampleBankInfo {
        let Some(s) = sample else { return self };
        let normal = bank_of(s.normal_set);
        let addition = bank_of(s.addition_set);
        self.editor_auto_bank = addition.is_none();
        self.bank_for_additions = addition.clone().or_else(|| normal.clone());
        self.bank_for_normal = normal;
        if banks_only {
            return self;
        }
        self.custom_sample_bank = s.index;
        self.volume = s.volume.max(0);
        self.filename = s.filename.clone();
        self
    }

    /// Only the banks, from a slider node's edge set (`readCustomSampleBanks(sets[i], info)`).
    pub fn read_edge_set(mut self, normal_set: i32, addition_set: i32) -> SampleBankInfo {
        let normal = bank_of(normal_set);
        let addition = bank_of(addition_set);
        self.editor_auto_bank = addition.is_none();
        self.bank_for_additions = addition.or_else(|| normal.clone());
        self.bank_for_normal = normal;
        // An edge set like "1:2" has no index, volume or file part: lazer leaves those as they were,
        // except the file name, which it sets to null when there is no fifth part.
        self.filename = None;
        self
    }
}

/// One sample as lazer holds it (`LegacyHitSampleInfo` or `FileHitSampleInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySample {
    pub name: String,
    pub bank: String,
    pub suffix: Option<String>,
    pub volume: i32,
    pub editor_auto_bank: bool,
    pub use_beatmap_samples: bool,
    pub is_layered: bool,
    pub bank_specified: bool,
    /// Set for a `FileHitSampleInfo`.
    pub filename: Option<String>,
}

impl LegacySample {
    /// `new LegacyHitSampleInfo(name, bank, volume, editorAutoBank, customSampleBank, isLayered)`.
    pub fn legacy(name: &str, bank: Option<&str>, volume: i32, editor_auto_bank: bool, custom_sample_bank: i32, is_layered: bool) -> LegacySample {
        LegacySample {
            name: name.to_string(),
            bank: bank.unwrap_or(DEFAULT_BANK).to_string(),
            suffix: (custom_sample_bank >= 2).then(|| custom_sample_bank.to_string()),
            volume,
            editor_auto_bank,
            use_beatmap_samples: custom_sample_bank >= 1,
            is_layered,
            bank_specified: bank.is_some_and(|b| !b.is_empty()),
            filename: None,
        }
    }

    /// `new FileHitSampleInfo(filename, volume)`.
    pub fn file(filename: &str, volume: i32) -> LegacySample {
        let mut s = LegacySample::legacy(HIT_NORMAL, Some(DEFAULT_BANK), volume, false, 1, false);
        s.filename = Some(filename.to_string());
        s
    }

    /// `LegacyHitSampleInfo.CustomSampleBank`.
    pub fn custom_sample_bank(&self) -> i32 {
        match &self.suffix {
            Some(s) => s.parse().unwrap_or(0),
            None => i32::from(self.use_beatmap_samples),
        }
    }

    pub fn kind(&self) -> &'static str {
        if self.filename.is_some() { "file" } else { "legacy" }
    }

    /// `With(newName, newBank, newVolume, newCustomSampleBank)`; a file sample keeps its file and
    /// name and takes only the new volume (`FileHitSampleInfo.With`).
    fn with(&self, name: Option<&str>, bank: Option<&str>, volume: Option<i32>, custom_sample_bank: Option<i32>) -> LegacySample {
        if let Some(f) = &self.filename {
            return LegacySample::file(f, volume.unwrap_or(self.volume));
        }
        LegacySample::legacy(
            name.unwrap_or(&self.name),
            Some(bank.unwrap_or(&self.bank)),
            volume.unwrap_or(self.volume),
            self.editor_auto_bank,
            custom_sample_bank.unwrap_or(self.custom_sample_bank()),
            self.is_layered,
        )
    }

    /// The same sample under another name (`With("slidertick")`, `With("sliderslide")`).
    pub fn renamed(&self, name: &str) -> LegacySample {
        self.with(Some(name), None, None, None)
    }

    /// A copy (`o.With()`), as `PopulateNodeSamples` makes.
    pub fn copied(&self) -> LegacySample {
        self.with(None, None, None, None)
    }

    /// `LegacySampleControlPoint.ApplyTo`, or `SampleControlPoint.DEFAULT.ApplyTo` when `point` is `None`.
    pub fn apply(&self, point: Option<&SamplePointData>) -> LegacySample {
        match point {
            Some(p) => self.with(
                None,
                Some(if self.bank_specified { self.bank.as_str() } else { p.bank.as_str() }),
                Some(if self.volume > 0 { self.volume } else { p.volume }),
                Some(if self.custom_sample_bank() > 0 { self.custom_sample_bank() } else { p.custom_sample_bank }),
            ),
            // The default point is a plain SampleControlPoint: it keeps the bank, and a volume of 0 becomes 100.
            None => self.with(None, Some(&self.bank.clone()), Some(if self.volume > 0 { self.volume } else { 100 }), None),
        }
    }

    /// `HitSampleInfo.LookupNames`, with `FileHitSampleInfo`'s file names first.
    pub fn lookup_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(f) = &self.filename {
            out.push(f.clone());
            out.push(change_extension_to_none(f));
        }
        if let Some(s) = self.suffix.as_deref().filter(|s| !s.is_empty()) {
            out.push(format!("Gameplay/{}-{}{}", self.bank, self.name, s));
        }
        out.push(format!("Gameplay/{}-{}", self.bank, self.name));
        out.push(format!("Gameplay/{}", self.name));
        out
    }

    /// The names a legacy skin looks a sample up by (`LegacySkin.getLegacyLookupNames`).
    /// A beatmap's own skin uses custom sample banks; other legacy skins do not.
    pub fn legacy_lookup_names(&self, use_custom_sample_banks: bool) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for name in self.lookup_names() {
            let last = name.rsplit('/').next().unwrap_or(&name).to_string();
            names.push(name);
            names.push(last);
        }
        if let Some(suffix) = self.suffix.as_deref().filter(|s| !s.is_empty()) {
            names.retain(|n| n.ends_with(suffix) == use_custom_sample_banks);
        }
        names.push(self.name.clone());
        names
    }
}

/// `Path.ChangeExtension(name, null)`: the name without its last extension.
fn change_extension_to_none(name: &str) -> String {
    let start = name.rfind(['/', '\\']).map_or(0, |i| i + 1);
    match name[start..].rfind('.') {
        Some(dot) => name[..start + dot].to_string(),
        None => name.to_string(),
    }
}

/// `convertSoundType`: the samples of a hitsound field with its bank info.
pub fn convert_sound_type(sound_type: i32, bank: &SampleBankInfo) -> Vec<LegacySample> {
    let mut out = Vec::new();
    match bank.filename.as_deref().filter(|f| !f.is_empty()) {
        None => out.push(LegacySample::legacy(
            HIT_NORMAL,
            bank.bank_for_normal.as_deref(),
            bank.volume,
            true,
            bank.custom_sample_bank,
            // Without the Normal flag the hitnormal is still added, as a layered sample; 0 counts as normal.
            sound_type != 0 && sound_type & NORMAL == 0,
        )),
        Some(f) => out.push(LegacySample::file(f, bank.volume)),
    }
    for (flag, name) in [(FINISH, HIT_FINISH), (WHISTLE, HIT_WHISTLE), (CLAP, HIT_CLAP)] {
        if sound_type & flag != 0 {
            out.push(LegacySample::legacy(name, bank.bank_for_additions.as_deref(), bank.volume, bank.editor_auto_bank, bank.custom_sample_bank, false));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_2_looks_only_for_the_numbered_file_then_the_bare_name() {
        let s = LegacySample::legacy(HIT_CLAP, Some("soft"), 50, false, 2, false);
        assert_eq!(s.legacy_lookup_names(true), vec!["Gameplay/soft-hitclap2", "soft-hitclap2", "hitclap"]);
        assert_eq!(s.legacy_lookup_names(false), vec!["Gameplay/soft-hitclap", "soft-hitclap", "Gameplay/hitclap", "hitclap", "hitclap"]);
    }

    #[test]
    fn a_file_sample_tries_the_file_then_its_name_without_extension() {
        let s = LegacySample::file("dir/sub sample.ogg", 40);
        assert_eq!(&s.lookup_names()[..2], &["dir/sub sample.ogg".to_string(), "dir/sub sample".to_string()]);
    }

    #[test]
    fn additions_without_a_normal_flag_make_a_layered_hitnormal() {
        let bank = SampleBankInfo::default();
        let samples = convert_sound_type(CLAP, &bank);
        assert_eq!(samples.len(), 2);
        assert!(samples[0].is_layered);
        assert_eq!(samples[1].name, HIT_CLAP);
        assert!(!convert_sound_type(0, &bank)[0].is_layered);
    }

    #[test]
    fn applying_a_point_fills_what_the_object_left_open() {
        let s = LegacySample::legacy(HIT_NORMAL, None, 0, true, 0, false);
        let p = SamplePointData { time: 0.0, bank: "soft".into(), volume: 60, custom_sample_bank: 3 };
        let a = s.apply(Some(&p));
        assert_eq!((a.bank.as_str(), a.volume, a.suffix.as_deref(), a.bank_specified), ("soft", 60, Some("3"), true));
    }
}
