use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Location {
    root: PathBuf,
    path: PathBuf,
}

impl Location {
    pub fn new(root: PathBuf, path: PathBuf) -> Self {
        Self { root, path }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn resolve(&self, configured: Option<&Path>) -> Result<PathBuf, String> {
        let configured = configured.ok_or("Choose a hitsound library folder first.")?;
        if self.root != configured {
            return Err("The hitsound library folder changed. Choose the sample again.".into());
        }
        if self
            .path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("Choose a path inside the hitsound library.".into());
        }
        let root = configured
            .canonicalize()
            .map_err(|e| format!("Opening hitsound library: {e}"))?;
        let path = root
            .join(&self.path)
            .canonicalize()
            .map_err(|e| format!("Opening library entry: {e}"))?;
        if !path.starts_with(&root) {
            return Err("The entry leaves the hitsound library folder.".into());
        }
        Ok(path)
    }

    pub fn entries(&self, configured: Option<&Path>) -> Result<Vec<(String, bool)>, String> {
        let path = self.resolve(configured)?;
        let entries =
            std::fs::read_dir(path).map_err(|e| format!("Reading hitsound library: {e}"))?;
        Ok(entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let kind = entry.file_type().ok()?;
                let name = entry.file_name().into_string().ok()?;
                if name.contains('\\')
                    || (!kind.is_dir() && !(kind.is_file() && is_sample(Path::new(&name))))
                {
                    return None;
                }
                Some((name, kind.is_dir()))
            })
            .collect())
    }

    pub fn sample(&self, configured: Option<&Path>) -> Result<PathBuf, String> {
        let path = self.resolve(configured)?;
        if !path.is_file() || !is_sample(&path) {
            return Err("Choose a WAV, OGG, or MP3 sample.".into());
        }
        Ok(path)
    }
}

fn is_sample(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ["wav", "ogg", "mp3"]
            .iter()
            .any(|sample| ext.eq_ignore_ascii_case(sample))
    })
}
