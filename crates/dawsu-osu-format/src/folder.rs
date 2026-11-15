//! Mapset folders (Responsibility 9): listing the files, finding a name case-insensitively
//! like Windows, and refusing paths from a map that would leave the folder (invariant 9).
//! Which file plays belongs to the hitsound rules (ADR 0036).

use crate::Error;
use std::fs;
use std::path::Path;

/// Every file under `dir`, as paths relative to it with `/`, sorted.
pub fn list_folder(dir: &Path) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|e| Error::io("listing the folder", dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io("listing the folder", dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let kind = entry.file_type().map_err(|e| Error::io("listing the folder", entry.path(), e))?;
        if kind.is_dir() {
            walk(&entry.path(), &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

/// The listed name that matches `name` ignoring case and slash direction, as Windows finds files.
pub fn find_case_insensitive<'a>(listing: &'a [String], name: &str) -> Option<&'a str> {
    let wanted = name.replace('\\', "/").to_lowercase();
    listing.iter().find(|n| n.to_lowercase() == wanted).map(String::as_str)
}

/// A path from a map that is refused: a normal answer, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRefusal {
    pub path: String,
    pub reason: &'static str,
}

/// Checks a path from a map by string rules alone, before it reaches the file system (invariant 9).
pub fn check_map_path(path: &str) -> Result<(), PathRefusal> {
    let refuse = |reason| Err(PathRefusal { path: path.to_string(), reason });
    if path.is_empty() {
        return refuse("the path is empty");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return refuse("absolute and network paths are not allowed");
    }
    if path.contains(':') {
        return refuse("drive letters and colons are not allowed");
    }
    let mut depth: i32 = 0;
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return refuse("the path climbs out of the folder");
                }
            }
            _ => depth += 1,
        }
    }
    Ok(())
}
