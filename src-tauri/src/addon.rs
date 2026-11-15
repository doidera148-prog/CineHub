//! Cliente do protocolo de addons do Stremio.
//!
//! Um addon é um servidor HTTP que publica `manifest.json` e responde às rotas:
//!   /catalog/{tipo}/{id}.json            (ou /catalog/{tipo}/{id}/search=texto.json)
//!   /meta/{tipo}/{id}.json
//!   /stream/{tipo}/{id}.json

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub id_prefixes: Option<Vec<String>>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub catalogs: Vec<CatalogDef>,
}

/// Recurso pode ser apenas o nome ("meta") ou um objeto com tipos e prefixos próprios.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Resource {
    Name(String),
    Detailed(ResourceDetail),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDetail {
    pub name: String,
    #[serde(default)]
    pub types: Option<Vec<String>>,
    #[serde(default)]
    pub id_prefixes: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extra: Vec<ExtraDef>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtraDef {
    pub name: String,
    #[serde(default)]
    pub is_required: bool,
}

impl CatalogDef {
    /// Catálogos que exigem um parâmetro (ex.: busca) não entram na página inicial.
    pub fn requires_extra(&self) -> bool {
        self.extra.iter().any(|e| e.is_required)
    }

    pub fn supports_search(&self) -> bool {
        self.extra.iter().any(|e| e.name == "search")
    }
}

/// Addon instalado: URL do manifest e o manifest já lido.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Installed {
    pub transport_url: String,
    pub manifest: Manifest,
}

impl Installed {
    /// Endereço base das rotas do addon (o manifest sem "/manifest.json").
    pub fn base(&self) -> String {
        self.transport_url
            .strip_suffix("/manifest.json")
            .unwrap_or(&self.transport_url)
            .to_string()
    }

    /// Verifica se o addon responde ao recurso para este tipo e id, como o Stremio faz.
    pub fn supports(&self, resource: &str, kind: &str, id: &str) -> bool {
        let m = &self.manifest;
        m.resources.iter().any(|r| {
            let (name, types, prefixes) = match r {
                Resource::Name(n) => (n.as_str(), None, None),
                Resource::Detailed(d) => (d.name.as_str(), d.types.as_ref(), d.id_prefixes.as_ref()),
            };
            if name != resource {
                return false;
            }
            let types = types.unwrap_or(&m.types);
            let prefixes = prefixes.or(m.id_prefixes.as_ref());
            types.iter().any(|t| t == kind)
                && prefixes.map_or(true, |p| p.iter().any(|pre| id.starts_with(pre.as_str())))
        })
    }
}

pub fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("CineHub/0.2")
        .build()
        .expect("não foi possível criar o cliente HTTP")
}

/// Tamanho máximo de uma resposta de addon (8 MiB). Evita que um addon mal configurado
/// consuma toda a memória do aparelho.
pub const MAX_BODY: usize = 8 * 1024 * 1024;

const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_MAX_ENTRIES: usize = 500;

/// Cache em memória das respostas dos addons, para não refazer as mesmas chamadas.
#[derive(Default)]
pub struct Cache {
    entries: Mutex<HashMap<String, (Instant, Value)>>,
}

impl Cache {
    fn get(&self, key: &str) -> Option<Value> {
        let map = self.entries.lock().ok()?;
        let (at, value) = map.get(key)?;
        (at.elapsed() < CACHE_TTL).then(|| value.clone())
    }

    fn put(&self, key: String, value: Value) {
        if let Ok(mut map) = self.entries.lock() {
            if map.len() >= CACHE_MAX_ENTRIES {
                map.clear();
            }
            map.insert(key, (Instant::now(), value));
        }
    }
}

pub async fn fetch_manifest(client: &Client, url: &str) -> Result<Manifest, String> {
    let body = read_capped(client, url, MAX_BODY).await?;
    serde_json::from_slice(&body).map_err(|e| format!("manifest inválido: {e}"))
}

/// Lê um recurso JSON de addon usando o cache.
async fn fetch_value(client: &Client, cache: &Cache, url: &str) -> Result<Value, String> {
    if let Some(v) = cache.get(url) {
        return Ok(v);
    }
    let body = read_capped(client, url, MAX_BODY).await?;
    let value: Value = serde_json::from_slice(&body).map_err(|e| format!("resposta inválida: {e}"))?;
    cache.put(url.to_string(), value.clone());
    Ok(value)
}

/// Lê um catálogo. `extra` vem já codificado, ex.: "search=dark" ou vazio.
pub async fn catalog(
    client: &Client,
    cache: &Cache,
    a: &Installed,
    kind: &str,
    id: &str,
    extra: &str,
) -> Result<Vec<Value>, String> {
    let url = if extra.is_empty() {
        format!("{}/catalog/{}/{}.json", a.base(), enc(kind), enc(id))
    } else {
        format!("{}/catalog/{}/{}/{}.json", a.base(), enc(kind), enc(id), extra)
    };
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("metas").and_then(Value::as_array).cloned().unwrap_or_default())
}

pub async fn meta(client: &Client, cache: &Cache, a: &Installed, kind: &str, id: &str) -> Result<Option<Value>, String> {
    let url = format!("{}/meta/{}/{}.json", a.base(), enc(kind), enc(id));
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("meta").filter(|m| !m.is_null()).cloned())
}

pub async fn streams(client: &Client, cache: &Cache, a: &Installed, kind: &str, id: &str) -> Result<Vec<Value>, String> {
    let url = format!("{}/stream/{}/{}.json", a.base(), enc(kind), enc(id));
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("streams").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// Legendas externas do recurso "subtitles" (rota /subtitles/{tipo}/{id}.json).
pub async fn subtitles(client: &Client, cache: &Cache, a: &Installed, kind: &str, id: &str) -> Result<Vec<Value>, String> {
    let url = format!("{}/subtitles/{}/{}.json", a.base(), enc(kind), enc(id));
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("subtitles").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// Baixa um recurso até `max` bytes. Respostas maiores são recusadas.
pub async fn read_capped(client: &Client, url: &str, max: usize) -> Result<Vec<u8>, String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("falha ao conectar: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("falha ao ler resposta: {e}"))? {
        if body.len() + chunk.len() > max {
            return Err("resposta grande demais".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Aceita só http(s) com host público. Bloqueia localhost e endereços privados,
/// para que o comando de legenda não sirva de porta de acesso à rede interna.
pub fn is_safe_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_lowercase();
    if host == "localhost" || host.ends_with(".local") {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation())
        }
        Ok(IpAddr::V6(v6)) => {
            let first = v6.segments()[0];
            let unique_local = (first & 0xfe00) == 0xfc00;
            let link_local = (first & 0xffc0) == 0xfe80;
            !(v6.is_loopback() || v6.is_unspecified() || unique_local || link_local)
        }
        Err(_) => true,
    }
}

/// Codifica um segmento de URL, mantendo ":" (usado em ids como "tt123:1:2").
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(json: &str) -> Installed {
        Installed {
            transport_url: "https://addon.example/manifest.json".into(),
            manifest: serde_json::from_str(json).unwrap(),
        }
    }

    #[test]
    fn resource_matching_follows_stremio_rules() {
        let a = installed(
            r#"{"id":"x","version":"1","name":"X","types":["movie","series"],
                "idPrefixes":["tt"],
                "resources":["catalog","meta",{"name":"stream","types":["movie"],"idPrefixes":["tt"]}]}"#,
        );
        assert!(a.supports("meta", "series", "tt123"));
        assert!(!a.supports("meta", "series", "xx1"));
        assert!(a.supports("stream", "movie", "tt1"));
        assert!(!a.supports("stream", "series", "tt1"));
    }

    #[test]
    fn base_and_encoding() {
        let a = installed(r#"{"id":"x","version":"1","name":"X"}"#);
        assert_eq!(a.base(), "https://addon.example");
        assert_eq!(enc("tt1:1:2"), "tt1:1:2");
        assert_eq!(enc("ação"), "a%C3%A7%C3%A3o");
    }

    #[test]
    fn catalog_flags() {
        let c: CatalogDef = serde_json::from_str(
            r#"{"type":"movie","id":"s","extra":[{"name":"search","isRequired":true}]}"#,
        )
        .unwrap();
        assert!(c.requires_extra());
        assert!(c.supports_search());
    }
}
