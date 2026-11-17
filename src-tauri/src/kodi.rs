//! Integração opcional com uma instância Kodi na rede local via JSON-RPC.
//!
//! O CineHub não executa extensões Kodi: ele solicita à instância Kodi configurada pelo usuário
//! que liste extensões e navegue/reproduza os itens. Não há descoberta automática da rede.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde_json::{json, Value};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

fn kodi_url(endpoint: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(endpoint.trim())
        .map_err(|_| "Informe um endereço Kodi válido, por exemplo http://192.168.1.20:8080".to_string())?;
    if url.scheme() != "http"
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/" | "/jsonrpc")
    {
        return Err("Use o endereço HTTP local do Kodi, sem credenciais, parâmetros ou outros caminhos".into());
    }

    // Aceitar apenas IPs locais fornecidos explicitamente pelo usuário evita enviar credenciais
    // para hosts públicos e reduz riscos de DNS rebinding/SSRF. Nenhum host ou porta é varrido.
    let host = url
        .host_str()
        .ok_or_else(|| "O endereço do Kodi precisa conter um IP local".to_string())?;
    let ip: IpAddr = host
        .parse()
        .map_err(|_| "Use o IP local do Kodi (nomes de domínio não são aceitos)".to_string())?;
    let local = match ip {
        IpAddr::V4(ip) => {
            ip.is_private() || ip.is_loopback() || ip.is_link_local()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
        }
    };
    if !local {
        return Err("Por segurança, a conexão só é permitida a um IP privado/local".into());
    }
    url.set_path("/jsonrpc");
    Ok(url)
}

fn validate_directory(directory: &str) -> Result<(), String> {
    if directory == "" || directory.len() > 4096 || !directory.starts_with("plugin://") {
        return Err("O caminho precisa ser um diretório plugin:// do Kodi".into());
    }
    Ok(())
}

async fn rpc(
    endpoint: &str,
    username: Option<String>,
    password: Option<String>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let url = kodi_url(endpoint)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Não consegui preparar a conexão Kodi: {e}"))?;
    let mut request = client.post(url).json(&json!({
        "jsonrpc": "2.0",
        "id": "cinehub",
        "method": method,
        "params": params,
    }));
    if let Some(user) = username.filter(|u| !u.is_empty()) {
        request = request.basic_auth(user, password);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Falha ao conectar ao Kodi: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Kodi respondeu com HTTP {}", response.status()));
    }
    if response.content_length().is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) {
        return Err("Resposta do Kodi excede 2 MiB".into());
    }
    let bytes = response.bytes().await.map_err(|e| format!("Resposta Kodi inválida: {e}"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("Resposta do Kodi excede 2 MiB".into());
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| "Kodi retornou JSON inválido".to_string())?;
    if let Some(error) = body.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("erro JSON-RPC do Kodi")
            .to_string());
    }
    Ok(body.get("result").cloned().unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn kodi_addons(
    endpoint: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<Value, String> {
    rpc(
        &endpoint,
        username,
        password,
        "Addons.GetAddons",
        json!({
            "type": "xbmc.addon.video",
            "enabled": true,
            "properties": ["name", "version", "summary"]
        }),
    )
    .await
}

#[tauri::command]
pub async fn kodi_browse(
    endpoint: String,
    username: Option<String>,
    password: Option<String>,
    directory: String,
) -> Result<Value, String> {
    validate_directory(&directory)?;
    rpc(
        &endpoint,
        username,
        password,
        "Files.GetDirectory",
        json!({
            "directory": directory,
            "media": "video",
            "properties": ["title", "file", "filetype", "thumbnail", "plot", "season", "episode"]
        }),
    )
    .await
}

#[tauri::command]
pub async fn kodi_play(
    endpoint: String,
    username: Option<String>,
    password: Option<String>,
    file: String,
) -> Result<Value, String> {
    if file.len() > 8192
        || !(file.starts_with("plugin://") || file.starts_with("http://") || file.starts_with("https://"))
    {
        return Err("O item precisa ser um link plugin://, HTTP ou HTTPS do Kodi".into());
    }
    rpc(
        &endpoint,
        username,
        password,
        "Player.Open",
        json!({ "item": { "file": file } }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_explicit_local_kodi_addresses_only() {
        assert!(kodi_url("http://192.168.1.20:8080").is_ok());
        assert!(kodi_url("http://127.0.0.1:8080/jsonrpc").is_ok());
        assert!(kodi_url("http://kodi.local:8080").is_err());
        assert!(kodi_url("http://example.com:8080").is_err());
        assert!(kodi_url("https://192.168.1.20:8080").is_err());
        assert!(kodi_url("http://user:pass@192.168.1.20:8080").is_err());
    }

    #[test]
    fn only_kodi_plugin_directories_are_browsable() {
        assert!(validate_directory("plugin://plugin.video.example/").is_ok());
        assert!(validate_directory("/home/user").is_err());
        assert!(validate_directory("https://example.com").is_err());
    }
}

// Keep these imports exercised on targets where only one address family is used.
const _: Option<Ipv4Addr> = None;
const _: Option<Ipv6Addr> = None;
