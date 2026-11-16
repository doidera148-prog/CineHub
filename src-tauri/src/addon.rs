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
                Resource::Detailed(d) => {
                    (d.name.as_str(), d.types.as_ref(), d.id_prefixes.as_ref())
                }
            };
            if name != resource {
                return false;
            }
            let types = types.unwrap_or(&m.types);
            let prefixes = prefixes.or(m.id_prefixes.as_ref());
            types.iter().any(|t| t == kind)
                && prefixes.is_none_or(|p| p.iter().any(|pre| id.starts_with(pre.as_str())))
        })
    }
}

/// Máximo de redirecionamentos seguidos ao falar com um addon.
const ADDON_MAX_REDIRECTS: usize = 5;

/// Cliente para os addons. Segue redirecionamentos, mas só até um limite e só para
/// endereços que não sejam locais ou privados (checagem dos literais, sem rede).
pub fn client() -> Client {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= ADDON_MAX_REDIRECTS {
            attempt.error("redirecionamentos demais")
        } else if !literal_host_is_public(attempt.url().as_str()) {
            attempt.error("redirecionamento para endereço não permitido")
        } else {
            attempt.follow()
        }
    });
    Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("CineHub/0.2")
        .redirect(policy)
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
    let value: Value =
        serde_json::from_slice(&body).map_err(|e| format!("resposta inválida: {e}"))?;
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
        format!(
            "{}/catalog/{}/{}/{}.json",
            a.base(),
            enc(kind),
            enc(id),
            extra
        )
    };
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("metas")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

pub async fn meta(
    client: &Client,
    cache: &Cache,
    a: &Installed,
    kind: &str,
    id: &str,
) -> Result<Option<Value>, String> {
    let url = format!("{}/meta/{}/{}.json", a.base(), enc(kind), enc(id));
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("meta").filter(|m| !m.is_null()).cloned())
}

pub async fn streams(
    client: &Client,
    cache: &Cache,
    a: &Installed,
    kind: &str,
    id: &str,
) -> Result<Vec<Value>, String> {
    let url = format!("{}/stream/{}/{}.json", a.base(), enc(kind), enc(id));
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Legendas externas do recurso "subtitles" (rota /subtitles/{tipo}/{id}.json).
pub async fn subtitles(
    client: &Client,
    cache: &Cache,
    a: &Installed,
    kind: &str,
    id: &str,
) -> Result<Vec<Value>, String> {
    let url = format!("{}/subtitles/{}/{}.json", a.base(), enc(kind), enc(id));
    let v = fetch_value(client, cache, &url).await?;
    Ok(v.get("subtitles")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Baixa um recurso até `max` bytes. Respostas maiores são recusadas.
pub async fn read_capped(client: &Client, url: &str, max: usize) -> Result<Vec<u8>, String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("falha ao conectar: {e}"))?;
    read_body(&mut resp, max).await
}

/// Limite de redirecionamentos seguidos ao baixar uma legenda.
const MAX_REDIRECTS: usize = 3;

/// Cliente que NÃO segue redirecionamentos sozinho: cada destino é validado antes.
pub fn client_no_redirect() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("CineHub/0.2")
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("não foi possível criar o cliente HTTP")
}

/// true se o endereço IP é público (não loopback, privado, link-local, etc.).
/// IPv6 mapeado para IPv4 (::ffff:a.b.c.d) é avaliado como o IPv4 correspondente.
pub fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast())
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            let unique_local = (first & 0xfe00) == 0xfc00;
            let link_local = (first & 0xffc0) == 0xfe80;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || unique_local
                || link_local)
        }
    }
}

/// Checagem só dos literais (sem rede): esquema, nomes locais e IPs escritos diretamente.
pub fn literal_host_is_public(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_lowercase();
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".localhost") {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => ip_is_public(ip),
        Err(_) => true,
    }
}

/// Checagem completa: os literais e também o DNS. Todo endereço que o nome resolve
/// precisa ser público. (Um DNS que muda a resposta depois ainda pode burlar isso.)
pub async fn is_safe_url(raw: &str) -> bool {
    if !literal_host_is_public(raw) {
        return false;
    }
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    if host.parse::<IpAddr>().is_ok() {
        return true; // IP literal já verificado acima
    }
    let port = url.port_or_known_default().unwrap_or(443);
    match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            !addrs.is_empty() && addrs.iter().all(|a| ip_is_public(a.ip()))
        }
        Err(_) => false,
    }
}

/// Baixa uma legenda pública, validando cada redirecionamento e limitando o tamanho.
pub async fn fetch_public_capped(
    client: &Client,
    raw: &str,
    max: usize,
) -> Result<Vec<u8>, String> {
    let mut current = Url::parse(raw).map_err(|_| "URL inválida".to_string())?;
    for _ in 0..=MAX_REDIRECTS {
        if !is_safe_url(current.as_str()).await {
            return Err("destino bloqueado por segurança".into());
        }
        let mut resp = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| format!("falha ao conectar: {e}"))?;
        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "redirecionamento sem destino".to_string())?;
            current = current
                .join(location)
                .map_err(|_| "redirecionamento inválido".to_string())?;
            continue;
        }
        return read_body(&mut resp, max).await;
    }
    Err("redirecionamentos demais".into())
}

async fn read_body(resp: &mut reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("falha ao ler resposta: {e}"))?
    {
        if body.len() + chunk.len() > max {
            return Err("resposta grande demais".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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
