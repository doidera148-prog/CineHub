//! Persistência dos addons instalados em um arquivo JSON.

use std::io;
use std::path::Path;

use crate::addon::Installed;

pub fn load(path: &Path) -> Vec<Installed> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, list: &[Installed]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec_pretty(list).map_err(io::Error::other)?;
    std::fs::write(path, bytes)
}
