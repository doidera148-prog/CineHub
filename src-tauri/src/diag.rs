//! Registro de diagnóstico local. Fica no aparelho e só sai quando o usuário exporta.
//!
//! Nada é enviado pela rede. Antes de gravar, cada mensagem passa por `redact`, que troca
//! endereços (URLs, magnets, IPs), hashes de torrent e valores de chaves secretas por marcadores.
//! Mesmo assim, quem registra deve mandar só a descrição do erro, nunca títulos ou dados pessoais.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::Manager;

/// Quando o arquivo atual passa deste tamanho, ele vira `.1` (e o `.1` antigo é descartado).
const MAX_LOG_BYTES: u64 = 512 * 1024;
const LOG_FILE: &str = "cinehub.log";
const OLD_LOG_FILE: &str = "cinehub.log.1";
/// Tamanho máximo de uma mensagem gravada.
const MAX_MESSAGE_CHARS: usize = 500;
/// Partes do nome de um parâmetro que indicam segredo (ex.: `?apikey=…`).
const SECRET_KEYS: &[&str] = &["key", "token", "secret", "pass", "auth", "sig"];

static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Remove o que identifica o usuário ou dá acesso a algo: URLs, magnets, IPs, hashes e segredos.
pub fn redact(input: &str) -> String {
    let joined = input
        .split_whitespace()
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(MAX_MESSAGE_CHARS).collect()
}

fn redact_token(tok: &str) -> String {
    let lower = tok.to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        return "<magnet>".into();
    }
    if tok.contains("://") || lower.starts_with("www.") {
        return "<url>".into();
    }
    if let Some((key, _)) = tok.split_once('=') {
        let k = key.to_ascii_lowercase();
        if SECRET_KEYS.iter().any(|s| k.contains(s)) {
            return format!("{key}=<redigido>");
        }
    }
    let core = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.');
    if is_hex_hash(core) {
        return "<hash>".into();
    }
    if is_ip(core) {
        return "<ip>".into();
    }
    tok.to_string()
}

/// Hash de torrent (SHA-1 em hex, 40 caracteres; aceita também 32 de hashes mais antigos).
fn is_hex_hash(s: &str) -> bool {
    (s.len() == 40 || s.len() == 32) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_ip(s: &str) -> bool {
    s.parse::<std::net::IpAddr>().is_ok()
}

/// Grava uma linha no registro. O nível aceita só INFO, WARN ou ERROR.
pub fn append(dir: &Path, level: &str, message: &str) -> std::io::Result<()> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(dir)?;
    let current = dir.join(LOG_FILE);
    if std::fs::metadata(&current).map(|m| m.len()).unwrap_or(0) >= MAX_LOG_BYTES {
        let old = dir.join(OLD_LOG_FILE);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&current, &old)?;
    }
    let level = match level {
        "WARN" | "ERROR" => level,
        _ => "INFO",
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&current)?;
    writeln!(file, "{} {level} {}", unix_secs(), redact(message))
}

/// Junta o arquivo antigo e o atual, com um cabeçalho que diz o que é o relatório.
pub fn report(dir: &Path) -> String {
    let mut body = String::new();
    for name in [OLD_LOG_FILE, LOG_FILE] {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            body.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    format!(
        "CineHub — diagnóstico\nVersão: {}\nSistema: {}\nGerado em (unix): {}\n\
         Endereços, magnets, IPs, hashes e segredos são removidos antes de gravar.\n---\n{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        unix_secs(),
        body
    )
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn log_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("logs"))
        .map_err(|e| format!("pasta de dados: {e}"))
}

/// Registra uma mensagem vinda do front-end (erros de reprodução, falhas de addon etc.).
#[tauri::command]
pub fn log_event(app: tauri::AppHandle, level: String, message: String) -> Result<(), String> {
    let dir = log_dir(&app)?;
    append(&dir, &level, &message).map_err(|e| format!("registro: {e}"))
}

/// Texto do diagnóstico, para o usuário salvar com o diálogo "Salvar como".
#[tauri::command]
pub fn diagnostics_text(app: tauri::AppHandle) -> Result<String, String> {
    Ok(report(&log_dir(&app)?))
}

/// Salva o diagnóstico na pasta de dados do app (Android, onde não há diálogo de salvar).
#[tauri::command]
pub fn save_diagnostics(app: tauri::AppHandle) -> Result<String, String> {
    let text = report(&log_dir(&app)?);
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("pasta de dados: {e}"))?
        .join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("pasta de exportação: {e}"))?;
    let path = dir.join(format!("cinehub-diagnostico-{}.txt", unix_secs()));
    std::fs::write(&path, text).map_err(|e| format!("não foi possível salvar: {e}"))?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("cinehub-diag-{name}-{nanos}"))
    }

    #[test]
    fn redacts_addresses_hashes_and_secrets() {
        assert_eq!(
            redact("falha em https://addon.example/cfg/TOKEN123/stream.json"),
            "falha em <url>"
        );
        assert_eq!(redact("abre magnet:?xt=urn:btih:abc"), "abre <magnet>");
        assert_eq!(
            redact("hash 0123456789abcdef0123456789abcdef01234567 ok"),
            "hash <hash> ok"
        );
        assert_eq!(
            redact("conexão com 192.168.0.10 caiu"),
            "conexão com <ip> caiu"
        );
        assert_eq!(
            redact("conexão com 2001:db8::1 caiu"),
            "conexão com <ip> caiu"
        );
        assert_eq!(
            redact("conexão com [2001:db8::1] caiu"),
            "conexão com <ip> caiu"
        );
        assert_eq!(redact("erro apikey=abc123 x"), "erro apikey=<redigido> x");
    }

    #[test]
    fn keeps_plain_error_text() {
        assert_eq!(redact("HTTP 404 Not Found"), "HTTP 404 Not Found");
        assert_eq!(redact("1.5 MB em 2 s"), "1.5 MB em 2 s");
    }

    #[test]
    fn caps_message_length() {
        let long = "a".repeat(2000);
        assert_eq!(redact(&long).chars().count(), MAX_MESSAGE_CHARS);
    }

    #[test]
    fn unknown_levels_become_info() {
        let dir = temp_dir("level");
        append(&dir, "DEBUG", "x").unwrap();
        let text = report(&dir);
        assert!(text.contains(" INFO x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotates_when_the_file_is_full() {
        let dir = temp_dir("rotate");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOG_FILE), vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        append(&dir, "ERROR", "depois da rotação").unwrap();
        assert!(dir.join(OLD_LOG_FILE).exists());
        let current = std::fs::read_to_string(dir.join(LOG_FILE)).unwrap();
        assert!(current.contains("ERROR depois da rotação"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
