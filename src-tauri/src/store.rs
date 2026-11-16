//! Persistência dos addons instalados em um arquivo JSON.

use std::io;
use std::path::Path;

use crate::addon::Installed;

/// Lê a lista salva. Se o arquivo existir mas não puder ser lido, ele é guardado como
/// `addons.json.corrupt` (para não perder a lista sem aviso) e a lista começa vazia.
pub fn load(path: &Path) -> Vec<Installed> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str(&text) {
        Ok(list) => list,
        Err(e) => {
            let backup = path.with_extension("json.corrupt");
            match std::fs::rename(path, &backup) {
                Ok(()) => eprintln!(
                    "aviso: addons.json corrompido ({e}); cópia guardada em {}",
                    backup.display()
                ),
                Err(err) => eprintln!(
                    "aviso: addons.json corrompido ({e}) e não foi possível guardá-lo: {err}"
                ),
            }
            Vec::new()
        }
    }
}

pub fn save(path: &Path, list: &[Installed]) -> io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec_pretty(list).map_err(io::Error::other)?;
    // Grava em um arquivo temporário, força a gravação no disco e troca de uma vez:
    // uma falha no meio (inclusive queda de energia) não corrompe a lista.
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("cinehub-store-{}-{nanos}", std::process::id()));
        let path = dir.join("addons.json");

        save(&path, &[]).expect("grava a lista");
        assert!(path.exists());
        assert!(
            !dir.join("addons.json.tmp").exists(),
            "arquivo temporário não foi removido"
        );
        assert!(load(&path).is_empty());

        // Um arquivo corrompido não derruba o app: a lista volta vazia e o conteúdo é guardado.
        std::fs::write(&path, b"{ isso nao e json").unwrap();
        assert!(load(&path).is_empty());
        assert!(
            !path.exists(),
            "o arquivo corrompido deveria ter sido movido"
        );
        let backup = dir.join("addons.json.corrupt");
        assert_eq!(std::fs::read(&backup).unwrap(), b"{ isso nao e json");

        std::fs::remove_dir_all(&dir).ok();
    }
}
