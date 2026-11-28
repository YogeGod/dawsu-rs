//! Pure-Rust SoundFont 2 (`.sf2` RIFF `sfbk`) parser and polyphonic audio renderer.
//!
//! Reads `INFO` (`INAM`), `sdta` (`smpl` 16-bit PCM), and `pdta` (`phdr`/`pbag`/`pgen`/
//! `inst`/`ibag`/`igen`/`shdr`) directly with zero external dependencies, and shares the
//! click-free polyphonic voice renderer in [`crate::sfz`].

use crate::sfz::{SamplerAudio, SamplerClip, SamplerNote, SamplerSummary, VoiceParams, mix_voice_into_stereo};
use std::fs;
use std::path::{Path, PathBuf};

const GEN_ATTACK_VOL_ENV: u16 = 34;
const GEN_RELEASE_VOL_ENV: u16 = 38;
const GEN_INSTRUMENT: u16 = 41;
const GEN_KEY_RANGE: u16 = 43;
const GEN_VEL_RANGE: u16 = 44;
const GEN_INITIAL_ATTENUATION: u16 = 48;
const GEN_COARSE_TUNE: u16 = 51;
const GEN_FINE_TUNE: u16 = 52;
const GEN_SAMPLE_ID: u16 = 53;
const GEN_SAMPLE_MODES: u16 = 54;
const GEN_OVERRIDING_ROOT_KEY: u16 = 58;
const GEN_PAN: u16 = 17;

/// One playable sample zone inside a SoundFont 2 preset.
#[derive(Debug, Clone)]
pub struct Sf2Zone {
    pub sample_index: usize,
    pub lokey: u8,
    pub hikey: u8,
    pub lovel: u8,
    pub hivel: u8,
    pub root_key: u8,
    pub tune_cents: f32,
    pub attenuation_db: f32,
    pub pan: f32,
    pub attack_sec: f32,
    pub release_sec: f32,
    pub loop_continuous: bool,
}

impl Sf2Zone {
    pub fn matches(&self, pitch: u8, velocity: u8) -> bool {
        pitch >= self.lokey
            && pitch <= self.hikey
            && velocity >= self.lovel
            && velocity <= self.hivel
    }
}

/// One SoundFont 2 preset (`bank`:`program` + name + resolved sample zones).
#[derive(Debug, Clone)]
pub struct Sf2Preset {
    pub name: String,
    pub bank: u16,
    pub program: u16,
    pub zones: Vec<Sf2Zone>,
}

#[derive(Debug, Clone)]
struct Sf2SampleHeader {
    start: usize,
    end: usize,
    start_loop: usize,
    end_loop: usize,
    sample_rate: u32,
    original_pitch: u8,
    pitch_correction: i8,
}

/// Parsed SoundFont 2 bank containing sample pool and all resolved presets.
#[derive(Debug, Clone)]
pub struct Sf2Instrument {
    pub name: String,
    pub source_path: PathBuf,
    pub presets: Vec<Sf2Preset>,
    sample_pool: Vec<f32>,
    samples: Vec<Sf2SampleHeader>,
}

impl Sf2Instrument {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path)
            .map_err(|e| format!("Could not read SoundFont {}: {e}", path.display()))?;
        let fallback = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("SoundFont");
        let mut inst = Self::from_bytes(&bytes, fallback)?;
        inst.source_path = path.to_path_buf();
        Ok(inst)
    }

    pub fn from_bytes(bytes: &[u8], fallback_name: &str) -> Result<Self, String> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"sfbk" {
            return Err("Not a valid SoundFont 2 (RIFF sfbk) file.".into());
        }

        let mut bank_name = fallback_name.to_string();
        let mut smpl_bytes: Option<&[u8]> = None;
        let mut phdr_bytes: Option<&[u8]> = None;
        let mut pbag_bytes: Option<&[u8]> = None;
        let mut pgen_bytes: Option<&[u8]> = None;
        let mut inst_bytes: Option<&[u8]> = None;
        let mut ibag_bytes: Option<&[u8]> = None;
        let mut igen_bytes: Option<&[u8]> = None;
        let mut shdr_bytes: Option<&[u8]> = None;

        for (id, chunk) in iter_riff_chunks(&bytes[12..]) {
            if id != b"LIST" || chunk.len() < 4 {
                continue;
            }
            let list_type = &chunk[0..4];
            let body = &chunk[4..];
            match list_type {
                b"INFO" => {
                    for (sub_id, sub) in iter_riff_chunks(body) {
                        if sub_id == b"INAM" {
                            let name = read_zstr(sub);
                            if !name.is_empty() {
                                bank_name = name;
                            }
                        }
                    }
                }
                b"sdta" => {
                    for (sub_id, sub) in iter_riff_chunks(body) {
                        if sub_id == b"smpl" {
                            smpl_bytes = Some(sub);
                        }
                    }
                }
                b"pdta" => {
                    for (sub_id, sub) in iter_riff_chunks(body) {
                        match sub_id {
                            b"phdr" => phdr_bytes = Some(sub),
                            b"pbag" => pbag_bytes = Some(sub),
                            b"pgen" => pgen_bytes = Some(sub),
                            b"inst" => inst_bytes = Some(sub),
                            b"ibag" => ibag_bytes = Some(sub),
                            b"igen" => igen_bytes = Some(sub),
                            b"shdr" => shdr_bytes = Some(sub),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        let smpl = smpl_bytes.ok_or("SoundFont missing sdta/smpl sample data.")?;
        let sample_pool: Vec<f32> = smpl
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| f32::from(i16::from_le_bytes(c)) / 32768.0)
            .collect();

        let shdr = parse_shdr(shdr_bytes.ok_or("SoundFont missing pdta/shdr.")?, sample_pool.len());
        let inst_headers = parse_inst(inst_bytes.ok_or("SoundFont missing pdta/inst.")?);
        let ibags = parse_bags(ibag_bytes.ok_or("SoundFont missing pdta/ibag.")?);
        let igens = parse_gens(igen_bytes.ok_or("SoundFont missing pdta/igen.")?);
        let phdrs = parse_phdr(phdr_bytes.ok_or("SoundFont missing pdta/phdr.")?);
        let pbags = parse_bags(pbag_bytes.ok_or("SoundFont missing pdta/pbag.")?);
        let pgens = parse_gens(pgen_bytes.ok_or("SoundFont missing pdta/pgen.")?);

        // Pre-resolve instrument zones (`inst` last record is terminal EOI).
        let inst_count = inst_headers.len().saturating_sub(1);
        let mut inst_zones: Vec<Vec<Sf2Zone>> = Vec::with_capacity(inst_count);
        for i in 0..inst_count {
            let bag_start = inst_headers[i].bag_ndx;
            let bag_end = inst_headers[i + 1].bag_ndx.min(ibags.len().saturating_sub(1));
            inst_zones.push(resolve_instrument_zones(bag_start, bag_end, &ibags, &igens, &shdr));
        }

        // Resolve presets (`phdr` last record is terminal EOP).
        let preset_count = phdrs.len().saturating_sub(1);
        let mut presets: Vec<Sf2Preset> = Vec::with_capacity(preset_count);
        for p in 0..preset_count {
            let hdr = &phdrs[p];
            let bag_start = hdr.bag_ndx;
            let bag_end = phdrs[p + 1].bag_ndx.min(pbags.len().saturating_sub(1));
            let zones = resolve_preset_zones(bag_start, bag_end, &pbags, &pgens, &inst_zones);
            if !zones.is_empty() {
                presets.push(Sf2Preset {
                    name: hdr.name.clone(),
                    bank: hdr.bank,
                    program: hdr.preset,
                    zones,
                });
            }
        }
        presets.sort_by_key(|p| (p.bank, p.program));

        if presets.is_empty() {
            return Err("SoundFont contains no playable presets.".into());
        }

        Ok(Self {
            name: bank_name,
            source_path: PathBuf::new(),
            presets,
            sample_pool,
            samples: shdr,
        })
    }

    pub fn summary(&self) -> SamplerSummary {
        let mut region_count = 0usize;
        let mut key_low: Option<u8> = None;
        let mut key_high: Option<u8> = None;
        for preset in &self.presets {
            region_count += preset.zones.len();
            for z in &preset.zones {
                key_low = Some(key_low.map_or(z.lokey, |lo| lo.min(z.lokey)));
                key_high = Some(key_high.map_or(z.hikey, |hi| hi.max(z.hikey)));
            }
        }
        let presets = self
            .presets
            .iter()
            .map(|p| format!("{:03}:{:03} {}", p.bank, p.program, p.name))
            .collect();
        SamplerSummary {
            format: "sf2".into(),
            name: self.name.clone(),
            path: self.source_path.to_string_lossy().into_owned(),
            region_count,
            key_low,
            key_high,
            presets,
        }
    }

    /// Renders `notes` through the preset at `preset_index` (clamped to available presets).
    pub fn render_notes(
        &self,
        preset_index: usize,
        notes: &[SamplerNote],
        sample_rate: u32,
    ) -> Result<SamplerAudio, String> {
        let sr = sample_rate.max(8_000);
        if notes.is_empty() || self.presets.is_empty() {
            return Ok(SamplerAudio {
                samples: Vec::new(),
                channels: 2,
                sample_rate: sr,
            });
        }
        let preset = &self.presets[preset_index.min(self.presets.len() - 1)];

        struct PendingZone<'a> {
            zone: &'a Sf2Zone,
            start_frame: usize,
            end_frame: usize,
            pitch: u8,
            velocity: u8,
            attack_frames: usize,
            release_frames: usize,
        }

        let mut pending: Vec<PendingZone<'_>> = Vec::new();
        for note in notes {
            let vel = note.velocity.clamp(1, 127);
            let start_frame = ((note.start_ms.max(0.0) / 1000.0) * f64::from(sr)).round() as usize;
            let hold_frames = ((note.duration_ms.max(10.0) / 1000.0) * f64::from(sr)).round() as usize;
            for zone in preset.zones.iter().filter(|z| z.matches(note.pitch, vel)) {
                let attack_frames = (zone.attack_sec * sr as f32).round() as usize;
                let release_frames = (zone.release_sec * sr as f32).round().max(64.0) as usize;
                pending.push(PendingZone {
                    zone,
                    start_frame,
                    end_frame: start_frame + hold_frames,
                    pitch: note.pitch,
                    velocity: vel,
                    attack_frames,
                    release_frames,
                });
            }
        }

        if pending.is_empty() {
            return Ok(SamplerAudio {
                samples: Vec::new(),
                channels: 2,
                sample_rate: sr,
            });
        }

        let total_frames = pending
            .iter()
            .map(|p| p.end_frame.saturating_add(p.release_frames))
            .max()
            .unwrap_or(0)
            .min(sr as usize * 30);
        let mut out = vec![0.0_f32; total_frames * 2];

        for item in pending {
            let Some(sh) = self.samples.get(item.zone.sample_index) else {
                continue;
            };
            if sh.end <= sh.start || sh.end > self.sample_pool.len() {
                continue;
            }
            let slice = &self.sample_pool[sh.start..sh.end];
            let clip = SamplerClip {
                samples: slice.to_vec(),
                channels: 1,
                sample_rate: sh.sample_rate.max(8_000),
            };
            let semitones = f64::from(item.pitch) - f64::from(item.zone.root_key)
                + (f64::from(item.zone.tune_cents) + f64::from(sh.pitch_correction)) / 100.0;
            let pitch_ratio = 2.0_f64.powf(semitones / 12.0);
            let vel_gain = (f32::from(item.velocity) / 127.0).powf(1.5);
            let atten_gain = 10.0_f32.powf(-item.zone.attenuation_db / 20.0);
            let gain = (vel_gain * atten_gain).clamp(0.0, 4.0);
            let loop_range = if item.zone.loop_continuous && sh.end_loop > sh.start_loop && sh.start_loop >= sh.start && sh.end_loop <= sh.end {
                Some((sh.start_loop - sh.start, sh.end_loop - sh.start))
            } else {
                None
            };
            mix_voice_into_stereo(
                &mut out,
                sr,
                &clip,
                VoiceParams {
                    start_frame: item.start_frame,
                    end_frame: item.end_frame,
                    pitch_ratio,
                    gain,
                    pan: item.zone.pan,
                    attack_frames: item.attack_frames,
                    release_frames: item.release_frames,
                    loop_range,
                },
            );
        }

        for s in &mut out {
            *s = s.clamp(-1.0, 1.0);
        }

        Ok(SamplerAudio {
            samples: out,
            channels: 2,
            sample_rate: sr,
        })
    }
}

fn iter_riff_chunks(data: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor + 8 <= data.len() {
        let id = &data[cursor..cursor + 4];
        let size = u32::from_le_bytes([
            data[cursor + 4],
            data[cursor + 5],
            data[cursor + 6],
            data[cursor + 7],
        ]) as usize;
        cursor += 8;
        let end = cursor.saturating_add(size).min(data.len());
        out.push((id, &data[cursor..end]));
        cursor = end + (size & 1);
    }
    out
}

fn read_zstr(bytes: &[u8]) -> String {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).trim().to_string()
}

struct RawPhdr {
    name: String,
    preset: u16,
    bank: u16,
    bag_ndx: usize,
}

struct RawInst {
    bag_ndx: usize,
}

#[derive(Clone, Copy)]
struct RawGen {
    oper: u16,
    amount: u16,
}

fn parse_phdr(data: &[u8]) -> Vec<RawPhdr> {
    data.as_chunks::<38>()
        .0
        .iter()
        .map(|c| RawPhdr {
            name: read_zstr(&c[0..20]),
            preset: u16::from_le_bytes([c[20], c[21]]),
            bank: u16::from_le_bytes([c[22], c[23]]),
            bag_ndx: usize::from(u16::from_le_bytes([c[24], c[25]])),
        })
        .collect()
}

fn parse_inst(data: &[u8]) -> Vec<RawInst> {
    data.as_chunks::<22>()
        .0
        .iter()
        .map(|c| RawInst {
            bag_ndx: usize::from(u16::from_le_bytes([c[20], c[21]])),
        })
        .collect()
}

fn parse_bags(data: &[u8]) -> Vec<usize> {
    data.as_chunks::<4>()
        .0
        .iter()
        .map(|c| usize::from(u16::from_le_bytes([c[0], c[1]])))
        .collect()
}

fn parse_gens(data: &[u8]) -> Vec<RawGen> {
    data.as_chunks::<4>()
        .0
        .iter()
        .map(|c| RawGen {
            oper: u16::from_le_bytes([c[0], c[1]]),
            amount: u16::from_le_bytes([c[2], c[3]]),
        })
        .collect()
}

fn parse_shdr(data: &[u8], pool_len: usize) -> Vec<Sf2SampleHeader> {
    data.as_chunks::<46>()
        .0
        .iter()
        .map(|c| {
            let start = (u32::from_le_bytes([c[20], c[21], c[22], c[23]]) as usize).min(pool_len);
            let end = (u32::from_le_bytes([c[24], c[25], c[26], c[27]]) as usize).min(pool_len);
            let start_loop = (u32::from_le_bytes([c[28], c[29], c[30], c[31]]) as usize).min(pool_len);
            let end_loop = (u32::from_le_bytes([c[32], c[33], c[34], c[35]]) as usize).min(pool_len);
            let sample_rate = u32::from_le_bytes([c[36], c[37], c[38], c[39]]);
            let original_pitch = c[40].min(127);
            let pitch_correction = c[41] as i8;
            Sf2SampleHeader {
                start,
                end,
                start_loop,
                end_loop,
                sample_rate,
                original_pitch,
                pitch_correction,
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
struct ZoneAccum {
    lokey: u8,
    hikey: u8,
    lovel: u8,
    hivel: u8,
    overriding_root: Option<u8>,
    coarse_tune: i16,
    fine_tune: i16,
    attenuation_cb: i16,
    pan_tenths: i16,
    attack_sec: f32,
    release_sec: f32,
    sample_modes: u16,
}

impl Default for ZoneAccum {
    fn default() -> Self {
        Self {
            lokey: 0,
            hikey: 127,
            lovel: 0,
            hivel: 127,
            overriding_root: None,
            coarse_tune: 0,
            fine_tune: 0,
            attenuation_cb: 0,
            pan_tenths: 0,
            attack_sec: 0.002,
            release_sec: 0.12,
            sample_modes: 0,
        }
    }
}

fn timecents_to_secs(tc: i16) -> f32 {
    if tc <= -12_000 {
        return 0.001;
    }
    2.0_f32.powf(f32::from(tc) / 1200.0).clamp(0.001, 10.0)
}

fn apply_gen(acc: &mut ZoneAccum, item: RawGen) {
    match item.oper {
        GEN_KEY_RANGE => {
            let lo = (item.amount & 0xff) as u8;
            let hi = ((item.amount >> 8) & 0xff) as u8;
            acc.lokey = lo.min(127);
            acc.hikey = hi.min(127);
        }
        GEN_VEL_RANGE => {
            let lo = (item.amount & 0xff) as u8;
            let hi = ((item.amount >> 8) & 0xff) as u8;
            acc.lovel = lo.min(127);
            acc.hivel = hi.min(127);
        }
        GEN_OVERRIDING_ROOT_KEY => {
            let v = item.amount as i16;
            if (0..=127).contains(&v) {
                acc.overriding_root = Some(v as u8);
            }
        }
        GEN_COARSE_TUNE => acc.coarse_tune = acc.coarse_tune.saturating_add(item.amount as i16),
        GEN_FINE_TUNE => acc.fine_tune = acc.fine_tune.saturating_add(item.amount as i16),
        GEN_INITIAL_ATTENUATION => {
            acc.attenuation_cb = acc.attenuation_cb.saturating_add(item.amount as i16);
        }
        GEN_PAN => acc.pan_tenths = acc.pan_tenths.saturating_add(item.amount as i16),
        GEN_ATTACK_VOL_ENV => acc.attack_sec = timecents_to_secs(item.amount as i16),
        GEN_RELEASE_VOL_ENV => acc.release_sec = timecents_to_secs(item.amount as i16),
        GEN_SAMPLE_MODES => acc.sample_modes = item.amount,
        _ => {}
    }
}

fn resolve_instrument_zones(
    bag_start: usize,
    bag_end: usize,
    ibags: &[usize],
    igens: &[RawGen],
    shdr: &[Sf2SampleHeader],
) -> Vec<Sf2Zone> {
    let mut out = Vec::new();
    let mut global = ZoneAccum::default();
    for bag_idx in bag_start..bag_end {
        let Some(&raw_start) = ibags.get(bag_idx) else {
            break;
        };
        let g_start = raw_start.min(igens.len());
        let g_end = ibags.get(bag_idx + 1).copied().unwrap_or(igens.len()).min(igens.len());
        // Malformed banks can list a bag starting after it ends: skip it, never as a global zone.
        let Some(slice) = igens.get(g_start..g_end) else {
            continue;
        };
        let sample_id = slice.iter().rev().find(|g| g.oper == GEN_SAMPLE_ID).map(|g| usize::from(g.amount));
        if sample_id.is_none() && bag_idx == bag_start {
            for &g in slice {
                apply_gen(&mut global, g);
            }
            continue;
        }
        let Some(sid) = sample_id else {
            continue;
        };
        let Some(sample_hdr) = shdr.get(sid) else {
            continue;
        };
        let mut acc = global;
        for &g in slice {
            apply_gen(&mut acc, g);
        }
        let root_key = acc.overriding_root.unwrap_or(sample_hdr.original_pitch).min(127);
        out.push(Sf2Zone {
            sample_index: sid,
            lokey: acc.lokey.min(acc.hikey),
            hikey: acc.hikey.max(acc.lokey),
            lovel: acc.lovel.min(acc.hivel),
            hivel: acc.hivel.max(acc.lovel),
            root_key,
            tune_cents: f32::from(acc.coarse_tune) * 100.0 + f32::from(acc.fine_tune),
            attenuation_db: (f32::from(acc.attenuation_cb) / 10.0).clamp(0.0, 96.0),
            pan: (f32::from(acc.pan_tenths) / 500.0).clamp(-1.0, 1.0),
            attack_sec: acc.attack_sec,
            release_sec: acc.release_sec,
            loop_continuous: (acc.sample_modes & 1) != 0,
        });
    }
    out
}

fn resolve_preset_zones(
    bag_start: usize,
    bag_end: usize,
    pbags: &[usize],
    pgens: &[RawGen],
    inst_zones: &[Vec<Sf2Zone>],
) -> Vec<Sf2Zone> {
    let mut out = Vec::new();
    let mut global = ZoneAccum::default();
    for bag_idx in bag_start..bag_end {
        let Some(&raw_start) = pbags.get(bag_idx) else {
            break;
        };
        let g_start = raw_start.min(pgens.len());
        let g_end = pbags.get(bag_idx + 1).copied().unwrap_or(pgens.len()).min(pgens.len());
        // Malformed banks can list a bag starting after it ends: skip it, never as a global zone.
        let Some(slice) = pgens.get(g_start..g_end) else {
            continue;
        };
        let inst_id = slice.iter().rev().find(|g| g.oper == GEN_INSTRUMENT).map(|g| usize::from(g.amount));
        if inst_id.is_none() && bag_idx == bag_start {
            for &g in slice {
                apply_gen(&mut global, g);
            }
            continue;
        }
        let Some(iid) = inst_id else {
            continue;
        };
        let Some(base_zones) = inst_zones.get(iid) else {
            continue;
        };
        let mut p_acc = global;
        for &g in slice {
            apply_gen(&mut p_acc, g);
        }
        for z in base_zones {
            let lokey = z.lokey.max(p_acc.lokey);
            let hikey = z.hikey.min(p_acc.hikey);
            let lovel = z.lovel.max(p_acc.lovel);
            let hivel = z.hivel.min(p_acc.hivel);
            if lokey > hikey || lovel > hivel {
                continue;
            }
            let mut merged = z.clone();
            merged.lokey = lokey;
            merged.hikey = hikey;
            merged.lovel = lovel;
            merged.hivel = hivel;
            merged.tune_cents += f32::from(p_acc.coarse_tune) * 100.0 + f32::from(p_acc.fine_tune);
            merged.attenuation_db = (merged.attenuation_db + f32::from(p_acc.attenuation_cb) / 10.0).clamp(0.0, 96.0);
            merged.pan = (merged.pan + f32::from(p_acc.pan_tenths) / 500.0).clamp(-1.0, 1.0);
            out.push(merged);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + payload.len() + 1);
        out.extend_from_slice(id);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut payload = Vec::with_capacity(4 + body.len());
        payload.extend_from_slice(kind);
        payload.extend_from_slice(body);
        chunk(b"LIST", &payload)
    }

    fn fixed_str(s: &str, len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        let copy = s.len().min(len);
        b[..copy].copy_from_slice(&s.as_bytes()[..copy]);
        b
    }

    fn build_minimal_sf2() -> Vec<u8> {
        build_sf2(&[0, 1], &[0], &[0, 2], &[(GEN_KEY_RANGE, u16::from_le_bytes([36, 84])), (GEN_SAMPLE_ID, 0)])
    }

    /// One preset over one instrument over one sample. `pbag_gens` / `ibag_gens` list each bag's
    /// first generator index, terminal bag included; `pgen_insts` / `igens` exclude the terminal generator.
    fn build_sf2(pbag_gens: &[u16], pgen_insts: &[u16], ibag_gens: &[u16], igens: &[(u16, u16)]) -> Vec<u8> {
        // 4800 frames of 440 Hz 16-bit PCM sine at 48 kHz.
        let mut smpl = Vec::with_capacity(4800 * 2);
        for i in 0..4800 {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 20_000.0;
            smpl.extend_from_slice(&(v.round() as i16).to_le_bytes());
        }
        let info = list(b"INFO", &chunk(b"INAM", b"Test Grand Piano\0"));
        let sdta = list(b"sdta", &chunk(b"smpl", &smpl));

        // 1 preset + terminal EOP.
        let mut phdr = Vec::new();
        phdr.extend_from_slice(&fixed_str("Grand Piano", 20));
        phdr.extend_from_slice(&0u16.to_le_bytes()); // preset 0
        phdr.extend_from_slice(&0u16.to_le_bytes()); // bank 0
        phdr.extend_from_slice(&0u16.to_le_bytes()); // pbag 0
        phdr.extend_from_slice(&[0u8; 12]);
        phdr.extend_from_slice(&fixed_str("EOP", 20));
        phdr.extend_from_slice(&0u16.to_le_bytes());
        phdr.extend_from_slice(&0u16.to_le_bytes());
        phdr.extend_from_slice(&(pbag_gens.len() as u16 - 1).to_le_bytes());
        phdr.extend_from_slice(&[0u8; 12]);

        let mut pbag = Vec::new();
        for &g in pbag_gens {
            pbag.extend_from_slice(&g.to_le_bytes());
            pbag.extend_from_slice(&0u16.to_le_bytes());
        }

        let mut pgen = Vec::new();
        for &inst_id in pgen_insts {
            pgen.extend_from_slice(&GEN_INSTRUMENT.to_le_bytes());
            pgen.extend_from_slice(&inst_id.to_le_bytes());
        }
        pgen.extend_from_slice(&0u16.to_le_bytes());
        pgen.extend_from_slice(&0u16.to_le_bytes());

        // 1 instrument + EOI.
        let mut inst = Vec::new();
        inst.extend_from_slice(&fixed_str("PianoInst", 20));
        inst.extend_from_slice(&0u16.to_le_bytes());
        inst.extend_from_slice(&fixed_str("EOI", 20));
        inst.extend_from_slice(&(ibag_gens.len() as u16 - 1).to_le_bytes());

        let mut ibag = Vec::new();
        for &g in ibag_gens {
            ibag.extend_from_slice(&g.to_le_bytes());
            ibag.extend_from_slice(&0u16.to_le_bytes());
        }

        let mut igen = Vec::new();
        for &(oper, amount) in igens {
            igen.extend_from_slice(&oper.to_le_bytes());
            igen.extend_from_slice(&amount.to_le_bytes());
        }
        igen.extend_from_slice(&0u16.to_le_bytes());
        igen.extend_from_slice(&0u16.to_le_bytes());

        // 1 sample header + EOS.
        let mut shdr = Vec::new();
        shdr.extend_from_slice(&fixed_str("SineA4", 20));
        shdr.extend_from_slice(&0u32.to_le_bytes()); // start
        shdr.extend_from_slice(&4800u32.to_le_bytes()); // end
        shdr.extend_from_slice(&0u32.to_le_bytes()); // loop start
        shdr.extend_from_slice(&4800u32.to_le_bytes()); // loop end
        shdr.extend_from_slice(&48_000u32.to_le_bytes()); // rate
        shdr.push(69); // A4
        shdr.push(0); // pitch correction
        shdr.extend_from_slice(&0u16.to_le_bytes());
        shdr.extend_from_slice(&1u16.to_le_bytes()); // monoSample
        shdr.extend_from_slice(&fixed_str("EOS", 20));
        shdr.extend_from_slice(&[0u8; 26]);

        let mut pdta_body = Vec::new();
        pdta_body.extend_from_slice(&chunk(b"phdr", &phdr));
        pdta_body.extend_from_slice(&chunk(b"pbag", &pbag));
        pdta_body.extend_from_slice(&chunk(b"pgen", &pgen));
        pdta_body.extend_from_slice(&chunk(b"inst", &inst));
        pdta_body.extend_from_slice(&chunk(b"ibag", &ibag));
        pdta_body.extend_from_slice(&chunk(b"igen", &igen));
        pdta_body.extend_from_slice(&chunk(b"shdr", &shdr));
        let pdta = list(b"pdta", &pdta_body);

        let mut sfbk = Vec::new();
        sfbk.extend_from_slice(b"sfbk");
        sfbk.extend_from_slice(&info);
        sfbk.extend_from_slice(&sdta);
        sfbk.extend_from_slice(&pdta);
        chunk(b"RIFF", &sfbk)
    }

    #[test]
    fn parses_sf2_bank_and_renders_polyphonic_chord() {
        let bytes = build_minimal_sf2();
        let sf2 = Sf2Instrument::from_bytes(&bytes, "Fallback").unwrap();
        assert_eq!(sf2.name, "Test Grand Piano");
        assert_eq!(sf2.presets.len(), 1);
        assert_eq!(sf2.presets[0].name, "Grand Piano");
        let summary = sf2.summary();
        assert_eq!(summary.format, "sf2");
        assert_eq!(summary.key_low, Some(36));
        assert_eq!(summary.key_high, Some(84));
        assert_eq!(summary.presets, vec!["000:000 Grand Piano"]);

        let audio = sf2
            .render_notes(
                0,
                &[
                    SamplerNote { start_ms: 0.0, duration_ms: 80.0, pitch: 60, velocity: 110 },
                    SamplerNote { start_ms: 0.0, duration_ms: 80.0, pitch: 64, velocity: 100 },
                    SamplerNote { start_ms: 0.0, duration_ms: 80.0, pitch: 67, velocity: 95 },
                ],
                48_000,
            )
            .unwrap();
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.sample_rate, 48_000);
        assert!(!audio.samples.is_empty());
        let peak = audio.samples.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.1 && peak <= 1.0, "unexpected peak {peak}");
        assert!(audio.samples.iter().all(|s| s.is_finite()));
    }

    const KEYS_36_84: (u16, u16) = (GEN_KEY_RANGE, u16::from_le_bytes([36, 84]));
    const KEYS_85_96: (u16, u16) = (GEN_KEY_RANGE, u16::from_le_bytes([85, 96]));
    const SAMPLE_0: (u16, u16) = (GEN_SAMPLE_ID, 0);

    #[test]
    fn reversed_or_out_of_range_bags_fail_cleanly_instead_of_panicking() {
        let igens = [KEYS_36_84, SAMPLE_0];
        // Instrument bag starts after it ends (2 > 0), or past the generator list.
        for ibag_gens in [[2u16, 0], [99, 1]] {
            let err = Sf2Instrument::from_bytes(&build_sf2(&[0, 1], &[0], &ibag_gens, &igens), "x").unwrap_err();
            assert!(err.contains("no playable presets"), "ibag {ibag_gens:?}: {err}");
        }
        // Preset bag starts after it ends (1 > 0), or past the generator list.
        for pbag_gens in [[1u16, 0], [99, 0]] {
            let err = Sf2Instrument::from_bytes(&build_sf2(&pbag_gens, &[0], &[0, 2], &igens), "x").unwrap_err();
            assert!(err.contains("no playable presets"), "pbag {pbag_gens:?}: {err}");
        }
    }

    #[test]
    fn reversed_instrument_bag_is_skipped_and_valid_zone_survives() {
        // Bag 0 is reversed (4 > 2) and must not become a global zone; bag 1 covers generators 2..4.
        let igens = [KEYS_36_84, SAMPLE_0, KEYS_85_96, SAMPLE_0];
        let sf2 = Sf2Instrument::from_bytes(&build_sf2(&[0, 1], &[0], &[4, 2, 4], &igens), "x").unwrap();
        assert_eq!(sf2.presets.len(), 1);
        assert_eq!(sf2.presets[0].zones.len(), 1);
        assert_eq!((sf2.presets[0].zones[0].lokey, sf2.presets[0].zones[0].hikey), (85, 96));
    }

    #[test]
    fn reversed_preset_bag_is_skipped_and_valid_zone_survives() {
        // Bag 0 is reversed (2 > 1); bag 1 covers generator 1 (instrument 0).
        let sf2 = Sf2Instrument::from_bytes(&build_sf2(&[2, 1, 2], &[0, 0], &[0, 2], &[KEYS_36_84, SAMPLE_0]), "x").unwrap();
        assert_eq!(sf2.presets.len(), 1);
        assert_eq!(sf2.presets[0].zones.len(), 1);
        assert_eq!((sf2.presets[0].zones[0].lokey, sf2.presets[0].zones[0].hikey), (36, 84));
    }
}
