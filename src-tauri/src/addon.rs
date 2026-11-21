//! Cliente do protocolo de addons do Stremio.
//!
//! Um addon é um servidor HTTP que publica `manifest.json` e responde às rotas:
//!   /catalog/{tipo}/{id}.json            (ou /catalog/{tipo}/{id}/search=texto.json)
//!   /meta/{tipo}/{id}.json
//!   /stream/{tipo}/{id}.json

use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
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
        // Query e fragmento não fazem parte do caminho das rotas: "…/manifest.json?key=x"
        // vira "…" (antes, o sufixo não era encontrado e as rotas ficavam quebradas).
        let without_query = self
            .transport_url
            .split(['?', '#'])
            .next()
            .unwrap_or(&self.transport_url);
        without_query
            .strip_suffix("/manifest.json")
            .unwrap_or(without_query)
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

/// Identificação enviada aos addons: sempre a versão real do app.
const USER_AGENT: &str = concat!("CineHub/", env!("CARGO_PKG_VERSION"));

/// Máximo de redirecionamentos seguidos ao falar com um addon.
const ADDON_MAX_REDIRECTS: usize = 5;

/// Resolvedor de DNS que entrega só endereços públicos. Como a conexão usa estes endereços,
/// um nome que aponte para a rede interna é recusado mesmo que a checagem anterior tenha passado.
pub(crate) struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| ip_is_public(a.ip()))
                .collect();
            if found.is_empty() {
                return Err("endereço não permitido ou inexistente".into());
            }
            Ok(Box::new(found.into_iter()) as Addrs)
        })
    }
}

/// Cliente para os addons. Segue redirecionamentos, mas só até um limite e só para
/// endereços públicos (literais checados aqui; nomes checados pelo resolvedor).
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
        .user_agent(USER_AGENT)
        .redirect(policy)
        .dns_resolver(Arc::new(PublicResolver))
        .build()
        .expect("não foi possível criar o cliente HTTP")
}

/// Tamanho máximo de uma resposta de addon (8 MiB). Evita que um addon mal configurado
/// consuma toda a memória do aparelho.
pub const MAX_BODY: usize = 8 * 1024 * 1024;

const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_MAX_ENTRIES: usize = 500;
/// Limite do cache pelo tamanho das respostas originais (o valor já decodificado ocupa mais).
const CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Recurso que o addon respondeu 404 (ou equivalente): não é pedido de novo por um tempo.
const MISS_TTL: Duration = Duration::from_secs(60);
/// Tentativas em erros passageiros (conexão ou 5xx) e a pausa entre elas.
const ATTEMPTS: u32 = 2;
const RETRY_DELAY: Duration = Duration::from_millis(400);

struct CacheEntry {
    /// Quando foi gravada: junto com `ttl`, define a validade.
    stored: Instant,
    /// Validade: o `max-age` do addon, limitado por CACHE_TTL.
    ttl: Duration,
    /// Último uso (contador): define quem sai primeiro quando o cache enche (LRU).
    used: u64,
    value: Value,
    size: usize,
}

#[derive(Default)]
struct CacheInner {
    map: HashMap<String, CacheEntry>,
    misses: HashMap<String, Instant>,
    bytes: usize,
    tick: u64,
}

/// Cache em memória das respostas dos addons, para não refazer as mesmas chamadas.
/// Quando enche, sai primeiro o item menos usado recentemente (e não tudo de uma vez).
#[derive(Default)]
pub struct Cache {
    inner: Mutex<CacheInner>,
}

impl Cache {
    fn get(&self, key: &str) -> Option<Value> {
        let mut inner = self.inner.lock().ok()?;
        inner.tick += 1;
        let tick = inner.tick;
        let entry = inner.map.get_mut(key)?;
        if entry.stored.elapsed() >= entry.ttl {
            return None;
        }
        entry.used = tick;
        Some(entry.value.clone())
    }

    /// Lembra que o recurso não existe, para não pedir de novo logo em seguida.
    fn note_miss(&self, key: String) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.misses.len() >= CACHE_MAX_ENTRIES {
            inner.misses.clear();
        }
        inner.misses.insert(key, Instant::now());
    }

    fn recently_missing(&self, key: &str) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|i| i.misses.get(key).map(|t| t.elapsed() < MISS_TTL))
            .unwrap_or(false)
    }

    fn put(&self, key: String, value: Value, size: usize, ttl: Duration) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        // Regravar a mesma chave substitui a antiga antes de medir o espaço.
        if let Some(old) = inner.map.remove(&key) {
            inner.bytes -= old.size;
        }
        while !inner.map.is_empty()
            && (inner.map.len() >= CACHE_MAX_ENTRIES || inner.bytes + size > CACHE_MAX_BYTES)
        {
            let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(removed) = inner.map.remove(&oldest) {
                inner.bytes -= removed.size;
            }
        }
        inner.tick += 1;
        let used = inner.tick;
        inner.bytes += size;
        inner.map.insert(
            key,
            CacheEntry {
                stored: Instant::now(),
                ttl,
                used,
                value,
                size,
            },
        );
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().map(|i| i.map.len()).unwrap_or(0)
    }
}

pub async fn fetch_manifest(client: &Client, url: &str) -> Result<Manifest, String> {
    let body = read_capped(client, url, MAX_BODY).await?;
    serde_json::from_slice(&body).map_err(|e| format!("manifest inválido: {e}"))
}

/// Falha ao buscar um recurso, classificada para decidir entre tentar de novo, guardar o 404 ou só relatar.
enum Failure {
    /// Conexão ou 5xx: pode melhorar numa nova tentativa.
    Transient(String),
    /// 4xx: o recurso não existe para este addon.
    NotFound(String),
    /// Outros erros (corpo grande demais, status inesperado).
    Other(String),
}

impl Failure {
    fn message(self) -> String {
        match self {
            Failure::Transient(m) | Failure::NotFound(m) | Failure::Other(m) => m,
        }
    }
}

/// Corpo de um recurso e até quanto tempo pode ficar no cache (zero = não guardar).
struct Fetched {
    body: Vec<u8>,
    ttl: Duration,
}

/// Validade pelo `Cache-Control` do addon: `max-age` (limitado por CACHE_TTL);
/// `no-store` e `no-cache` impedem o cache; sem cabeçalho vale CACHE_TTL.
fn ttl_for(headers: &reqwest::header::HeaderMap) -> Duration {
    let Some(cc) = headers
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase)
    else {
        return CACHE_TTL;
    };
    if cc.contains("no-store") || cc.contains("no-cache") {
        return Duration::ZERO;
    }
    cc.split(',')
        .map(str::trim)
        .find_map(|d| d.strip_prefix("max-age="))
        .and_then(|v| v.parse::<u64>().ok())
        .map(|secs| Duration::from_secs(secs).min(CACHE_TTL))
        .unwrap_or(CACHE_TTL)
}

async fn fetch_once(client: &Client, url: &str, max: usize) -> Result<Fetched, Failure> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| Failure::Transient(format!("falha ao conectar: {e}")))?;
    let status = resp.status();
    if status.is_server_error() {
        return Err(Failure::Transient(format!("HTTP {status}")));
    }
    if status.is_client_error() {
        return Err(Failure::NotFound(format!("HTTP {status}")));
    }
    let ttl = ttl_for(resp.headers());
    let body = read_body(&mut resp, max).await.map_err(Failure::Other)?;
    Ok(Fetched { body, ttl })
}

/// Busca com novas tentativas só para erros passageiros.
async fn fetch_retrying(client: &Client, url: &str, max: usize) -> Result<Fetched, Failure> {
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(RETRY_DELAY).await;
        }
        match fetch_once(client, url, max).await {
            Ok(f) => return Ok(f),
            Err(Failure::Transient(m)) => last = Some(Failure::Transient(m)),
            Err(other) => return Err(other),
        }
    }
    Err(last.unwrap_or_else(|| Failure::Transient("sem resposta".into())))
}

/// Lê um recurso JSON de addon usando o cache (respeitando Cache-Control e os 404 recentes).
async fn fetch_value(client: &Client, cache: &Cache, url: &str) -> Result<Value, String> {
    if let Some(v) = cache.get(url) {
        return Ok(v);
    }
    if cache.recently_missing(url) {
        return Err("o addon não tem este recurso (tentativa recente)".into());
    }
    let fetched = match fetch_retrying(client, url, MAX_BODY).await {
        Ok(f) => f,
        Err(Failure::NotFound(m)) => {
            cache.note_miss(url.to_string());
            return Err(m);
        }
        Err(f) => return Err(f.message()),
    };
    let value: Value =
        serde_json::from_slice(&fetched.body).map_err(|e| format!("resposta inválida: {e}"))?;
    if !fetched.ttl.is_zero() {
        cache.put(
            url.to_string(),
            value.clone(),
            fetched.body.len(),
            fetched.ttl,
        );
    }
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
    fetch_retrying(client, url, max)
        .await
        .map(|f| f.body)
        .map_err(Failure::message)
}

/// Limite de redirecionamentos seguidos ao baixar uma legenda.
const MAX_REDIRECTS: usize = 3;

/// Cliente que NÃO segue redirecionamentos sozinho: cada destino é validado antes.
pub fn client_no_redirect() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(PublicResolver))
        .build()
        .expect("não foi possível criar o cliente HTTP")
}

/// true se o endereço IP é público (não loopback, privado, reservado, etc.).
/// IPv6 mapeado para IPv4 (::ffff:a.b.c.d) é avaliado como o IPv4 correspondente.
pub fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            let reserved = a == 0                        // 0.0.0.0/8
                || (a == 100 && (b & 0xc0) == 64)        // 100.64.0.0/10 (CGNAT, ex.: Tailscale)
                || (a == 192 && b == 0 && c == 0)        // 192.0.0.0/24 (protocolos IETF)
                || (a == 198 && (b & 0xfe) == 18)        // 198.18.0.0/15 (testes de rede)
                || a >= 240; // 240.0.0.0/4 (reservado)
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || reserved)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_is_public(IpAddr::V4(v4));
            }
            let segs = v6.segments();
            let unique_local = (segs[0] & 0xfe00) == 0xfc00;
            let link_local = (segs[0] & 0xffc0) == 0xfe80;
            let site_local = (segs[0] & 0xffc0) == 0xfec0;
            let documentation = segs[0] == 0x2001 && segs[1] == 0x0db8;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || unique_local
                || link_local
                || site_local
                || documentation)
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

/// Checagem completa: o host precisa resolver para pelo menos um IP público.
/// Respostas DNS mistas podem ocorrer em redes móveis, DNS filtrado ou split-horizon;
/// isso não abre acesso à rede privada porque PublicResolver filtra novamente os IPs
/// no momento da conexão HTTP e só entrega endereços públicos ao cliente.
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
        Ok(addrs) => addrs.into_iter().any(|addr| ip_is_public(addr.ip())),
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
    if resp.content_length().is_some_and(|length| length > max as u64) {
        return Err("resposta grande demais".into());
    }
    let capacity = resp
        .content_length()
        .unwrap_or(0)
        .min(max as u64) as usize;
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("falha ao ler resposta: {e}"))?
    {
        if body.len().saturating_add(chunk.len()) > max {
            return Err("resposta grande demais".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Lê uma resposta já recebida sem permitir que seu corpo ultrapasse o limite.
pub async fn read_response_capped(
    resp: &mut reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, String> {
    read_body(resp, max).await
}

/// Codifica um segmento de URL, mantendo ":" (usado em ids como "tt123:1:2").
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
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

    #[tokio::test]
    async fn response_reader_rejects_chunked_body_over_limit() {
        use axum::{
            body::{Body, Bytes},
            http::Response,
            routing::get,
            Router,
        };

        let app = Router::new().route("/", get(|| async {
            let chunks = futures::stream::iter(vec![
                Ok::<_, std::io::Error>(Bytes::from_static(b"1234")),
                Ok::<_, std::io::Error>(Bytes::from_static(b"5678")),
            ]);
            Response::new(Body::from_stream(chunks))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let mut response = client
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.content_length(), None);
        assert!(read_body(&mut response, 6).await.is_err());

        server.abort();
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
    fn base_ignores_query_and_fragment() {
        let mut a = installed(r#"{"id":"x","version":"1","name":"X"}"#);
        a.transport_url = "https://addon.example/manifest.json?key=abc".into();
        assert_eq!(a.base(), "https://addon.example");
        a.transport_url = "https://addon.example/cfg/tok/manifest.json#x".into();
        assert_eq!(a.base(), "https://addon.example/cfg/tok");
    }

    #[test]
    fn cache_evicts_least_recently_used_first() {
        let cache = Cache::default();
        for i in 0..CACHE_MAX_ENTRIES {
            cache.put(format!("k{i}"), Value::Null, 1, CACHE_TTL);
        }
        // Usa k0: ele passa a ser o mais recente, então quem sai é k1.
        assert!(cache.get("k0").is_some());
        cache.put("novo".into(), Value::Null, 1, CACHE_TTL);
        assert_eq!(cache.len(), CACHE_MAX_ENTRIES);
        assert!(cache.get("k0").is_some());
        assert!(cache.get("k1").is_none());
        assert!(cache.get("novo").is_some());
    }

    #[test]
    fn cache_control_sets_how_long_a_response_is_kept() {
        use reqwest::header::{HeaderMap, HeaderValue, CACHE_CONTROL};
        let mut h = HeaderMap::new();
        assert_eq!(ttl_for(&h), CACHE_TTL);
        h.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120"),
        );
        assert_eq!(ttl_for(&h), Duration::from_secs(120));
        h.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=99999"));
        assert_eq!(ttl_for(&h), CACHE_TTL);
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        assert_eq!(ttl_for(&h), Duration::ZERO);
    }

    #[test]
    fn missing_resources_are_remembered_briefly() {
        let cache = Cache::default();
        assert!(!cache.recently_missing("https://a/x.json"));
        cache.note_miss("https://a/x.json".into());
        assert!(cache.recently_missing("https://a/x.json"));
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
