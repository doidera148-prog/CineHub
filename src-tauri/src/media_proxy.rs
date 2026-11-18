//! Proxy HTTP local autenticado para fontes de vídeo.
//!
//! A primeira etapa cobre HTTP progressivo e Range. HLS (reescrita de playlists e recursos
//! filhos) é deliberadamente recusado até existir tratamento específico para cada URI.
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::{Duration, Instant}};

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::Response,
    routing::any,
    Router,
};
use futures::stream;
use reqwest::{Client, Url};
use serde_json::Value;
use tokio::{net::TcpListener, sync::RwLock};
use uuid::Uuid;

const MAX_REDIRECTS: usize = 3;
const SESSION_TTL: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Clone)]
struct Target {
    url: Url,
    headers: Vec<(header::HeaderName, HeaderValue)>,
    expires: Instant,
}

#[derive(Clone)]
struct ProxyState {
    client: Client,
    sessions: Arc<RwLock<HashMap<String, Target>>>,
}

pub struct MediaProxy {
    address: SocketAddr,
    state: ProxyState,
}

impl MediaProxy {
    /// Inicia em loopback e porta efêmera; nunca expõe o proxy à rede local.
    pub async fn start() -> Result<Arc<Self>, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await.map_err(|e| format!("proxy de mídia: {e}"))?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        // Sem timeout total: streams podem durar horas. DNS continua fixado em IPs públicos.
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(12))
            .timeout(None)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("CineHub/0.2 media-proxy")
            .dns_resolver(Arc::new(crate::addon::PublicResolver))
            .build().map_err(|e| format!("cliente do proxy: {e}"))?;
        let state = ProxyState { client, sessions: Arc::new(RwLock::new(HashMap::new())) };
        let app = Router::new().route("/media/{token}", any(proxy_request)).with_state(state.clone());
        tauri::async_runtime::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("servidor proxy de mídia encerrou: {e}");
            }
        });
        Ok(Arc::new(Self { address, state }))
    }

    /// Registra uma URL validada e retorna uma URL local imprevisível.
    pub async fn register(&self, raw_url: &str, hints: Option<&Value>) -> Result<String, String> {
        let url = Url::parse(raw_url.trim()).map_err(|_| "URL de mídia inválida".to_string())?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err("somente fontes HTTP/HTTPS podem usar o proxy".into());
        }
        if url.path().to_ascii_lowercase().ends_with(".m3u8")
            || url.query_pairs().any(|(k, v)| k.eq_ignore_ascii_case("type") && v.eq_ignore_ascii_case("hls"))
        {
            return Err("HLS ainda não está habilitado no proxy: playlists e segmentos precisam de reescrita segura".into());
        }
        if !crate::addon::is_safe_url(url.as_str()).await {
            return Err("fonte bloqueada: o proxy só aceita destinos públicos".into());
        }
        let headers = parse_request_headers(hints)?;
        let token = Uuid::new_v4().simple().to_string();
        let mut sessions = self.state.sessions.write().await;
        sessions.retain(|_, v| v.expires > Instant::now());
        if sessions.len() >= 128 {
            return Err("limite de sessões do proxy atingido".into());
        }
        sessions.insert(token.clone(), Target { url, headers, expires: Instant::now() + SESSION_TTL });
        Ok(format!("http://{}:{}/media/{}", self.address.ip(), self.address.port(), token))
    }
}

/// Apenas cabeçalhos de requisição explicitamente úteis; bloqueia framing e hop-by-hop.
fn parse_request_headers(hints: Option<&Value>) -> Result<Vec<(header::HeaderName, HeaderValue)>, String> {
    let Some(obj) = hints.and_then(|v| v.get("proxyHeaders")).and_then(|v| v.get("request")).and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let allowed = ["authorization", "cookie", "origin", "referer", "user-agent"];
    let mut out = Vec::new();
    for (name, value) in obj {
        if !allowed.iter().any(|v| name.eq_ignore_ascii_case(v)) { continue; }
        let value = value.as_str().ok_or("proxyHeaders contém valor não textual")?;
        if value.len() > 4096 { return Err("proxyHeaders excede o limite permitido".into()); }
        let name = header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| "nome de cabeçalho inválido")?;
        let value = HeaderValue::from_str(value).map_err(|_| "valor de cabeçalho inválido")?;
        out.push((name, value));
    }
    Ok(out)
}

async fn proxy_request(
    State(state): State<ProxyState>,
    Path(token): Path<String>,
    method: Method,
    incoming: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((StatusCode::NOT_FOUND, "não encontrado".into()));
    }
    if method == Method::OPTIONS {
        return Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header("access-control-allow-origin", "*")
            .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
            .header("access-control-allow-headers", "Range, If-Range")
            .body(Body::empty())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }
    let target = {
        let mut sessions = state.sessions.write().await;
        sessions.retain(|_, v| v.expires > Instant::now());
        sessions.get(&token).cloned()
    }.ok_or((StatusCode::NOT_FOUND, "sessão expirada".into()))?;

    let mut url = target.url;
    let mut response = None;
    for _ in 0..=MAX_REDIRECTS {
        if !crate::addon::is_safe_url(url.as_str()).await {
            return Err((StatusCode::FORBIDDEN, "redirecionamento bloqueado".into()));
        }
        let mut request = match method {
            Method::HEAD => state.client.head(url.clone()),
            _ => state.client.get(url.clone()),
        };
        for (name, value) in &target.headers { request = request.header(name, value); }
        if let Some(range) = incoming.get(header::RANGE) { request = request.header(header::RANGE, range); }
        if let Some(if_range) = incoming.get(header::IF_RANGE) { request = request.header(header::IF_RANGE, if_range); }
        let upstream = request.send().await.map_err(|e| (StatusCode::BAD_GATEWAY, format!("falha na fonte: {e}")))?;
        if upstream.status().is_redirection() {
            let location = upstream.headers().get(header::LOCATION)
                .and_then(|v| v.to_str().ok()).ok_or((StatusCode::BAD_GATEWAY, "redirect sem Location".into()))?;
            url = url.join(location).map_err(|_| (StatusCode::BAD_GATEWAY, "redirect inválido".into()))?;
            continue;
        }
        response = Some(upstream);
        break;
    }
    let upstream = response.ok_or((StatusCode::BAD_GATEWAY, "redirecionamentos demais".into()))?;
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for name in [header::CONTENT_TYPE, header::CONTENT_LENGTH, header::CONTENT_RANGE, header::ACCEPT_RANGES, header::ETAG, header::LAST_MODIFIED] {
        if let Some(value) = upstream.headers().get(&name) { builder = builder.header(name, value); }
    }
    // O token é uma credencial local; não há cookies de sessão e o CORS não permite credenciais.
    builder = builder.header("access-control-allow-origin", "*")
        .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
        .header("access-control-allow-headers", "Range, If-Range")
        .header("access-control-expose-headers", "Content-Length, Content-Range, Accept-Ranges");
    let body = if method == Method::HEAD || status == StatusCode::NO_CONTENT {
        Body::empty()
    } else {
        let chunks = stream::unfold(upstream, |mut resp| async move {
            match resp.chunk().await {
                Ok(Some(bytes)) => Some((Ok::<_, std::io::Error>(bytes), resp)),
                Ok(None) => None,
                Err(e) => Some((Err(std::io::Error::other(e.to_string())), resp)),
            }
        });
        Body::from_stream(chunks)
    };
    builder.body(body).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_headers_rejects_untrusted_framing_headers() {
        let hints = serde_json::json!({"proxyHeaders":{"request":{
            "Host":"internal.example", "Connection":"keep-alive", "Referer":"https://example.com/"
        }}});
        let parsed = parse_request_headers(Some(&hints)).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, header::REFERER);
    }

    #[test]
    fn proxy_headers_rejects_invalid_values() {
        let hints = serde_json::json!({"proxyHeaders":{"request":{"Referer":42}}});
        assert!(parse_request_headers(Some(&hints)).is_err());
    }
}
