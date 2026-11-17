use crate::output::WritePlan;
use dawsu_foundation::{
    digest::sha256_hex,
    safe_write::{self, BackupPolicy, WriteOutcome},
};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct Receipt {
    version: u32,
    file_name: String,
    sha256: String,
    mixes: Vec<String>,
}

pub(super) struct Previous {
    pub bytes: Vec<u8>,
    pub mixes: Vec<String>,
}

pub(super) fn path(root: &Path, source: &Path) -> PathBuf {
    root.join("generated")
        .join(format!("{}.json", crate::project::path_digest(source)))
}

pub(super) fn previous(
    receipt_path: Option<&Path>,
    target: &Path,
) -> Result<Option<Previous>, String> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Reading the Generate target: {error}")),
    };
    let source_changed =
        || "The output changed since Generate. The existing difficulty was kept.".to_string();
    if !metadata.file_type().is_file() {
        return Err(source_changed());
    }
    let receipt = receipt_path.map(read).transpose()?.flatten();
    let Some(receipt) = receipt.filter(|receipt| {
        target
            .file_name()
            .is_some_and(|name| name == receipt.file_name.as_str())
    }) else {
        return Err("A difficulty already has the Generate output name. Only an earlier output tracked by this editor can be replaced.".into());
    };
    let bytes =
        fs::read(target).map_err(|error| format!("Reading the Generate target: {error}"))?;
    if sha256_hex(&bytes) != receipt.sha256 {
        return Err(source_changed());
    }
    Ok(Some(Previous {
        bytes,
        mixes: receipt.mixes,
    }))
}

fn read(path: &Path) -> Result<Option<Receipt>, String> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(None);
    };
    let record: Receipt = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Reading previous Generate tracking: {error}"))?;
    if record.version != VERSION {
        return Err("The previous Generate tracking has an unsupported version.".into());
    }
    Ok(Some(record))
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("The Generate tracking file is too large.".into());
    }
    Ok(Some(bytes))
}

pub(super) fn record(plan: &WritePlan) -> Result<(), String> {
    let Some(path) = &plan.generation else {
        return Ok(());
    };
    let [target] = plan.files.as_slice() else {
        return Err("Generate must write one difficulty.".into());
    };
    let file_name = target
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Missing generated filename.")?;
    let receipt = Receipt {
        version: VERSION,
        file_name: file_name.to_string(),
        sha256: sha256_hex(&target.file.to_bytes()),
        mixes: plan
            .mix_paths()
            .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
            .collect(),
    };
    let bytes = serde_json::to_vec(&receipt).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("The Generate tracking file is too large.".into());
    }
    let folder = path.parent().ok_or("Missing Generate tracking folder.")?;
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    if let Some(expected) = read_bytes(path)? {
        if expected == bytes {
            return Ok(());
        }
        let backups = BackupPolicy::new(folder.join("backups"), 5);
        if safe_write::replace(path, &expected, &bytes, &backups)
            .map_err(|error| error.to_string())?
            == WriteOutcome::ChangedOnDisk
        {
            return Err("Generate tracking changed while writing.".into());
        }
    } else {
        safe_write::create(path, &bytes).map_err(|error| error.to_string())?;
    }
    Ok(())
}
