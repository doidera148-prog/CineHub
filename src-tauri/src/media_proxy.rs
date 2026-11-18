//! Proxy HTTP local autenticado para fontes de vídeo.
//!
//! HTTP progressivo mantém streaming em blocos e Range; playlists HLS e todos os recursos
//! filhos são reescritos para URLs locais autenticadas. DASH permanece desabilitado.
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::{Body, Bytes},
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
    force_hls: bool,
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
            .await
            .map_err(|e| format!("proxy de mídia: {e}"))?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        // Sem timeout total: streams podem durar horas. DNS continua fixado em IPs públicos.
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("CineHub/0.2 media-proxy")
            .dns_resolver(Arc::new(crate::addon::PublicResolver))
            .build()
            .map_err(|e| format!("cliente do proxy: {e}"))?;
        let state = ProxyState {
            client,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        };
        let app = Router::new()
            .route("/media/{token}", any(proxy_request))
            .with_state(state.clone());
        tauri::async_runtime::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("servidor proxy de mídia encerrou: {e}");
            }
        });
        Ok(Arc::new(Self { address, state }))
    }

    /// Registra uma URL validada e retorna uma URL local imprevisível.
    pub async fn register(
        &self,
        raw_url: &str,
        hints: Option<&Value>,
        mime_type: Option<&str>,
    ) -> Result<String, String> {
        let url = Url::parse(raw_url.trim()).map_err(|_| "URL de mídia inválida".to_string())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(
                "somente fontes HTTP/HTTPS válidas, sem credenciais embutidas, podem usar o proxy"
                    .into(),
            );
        }
        if url.path().to_ascii_lowercase().ends_with(".mpd") {
            return Err("DASH ainda não está habilitado no proxy".into());
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
        let force_hls = mime_type.is_some_and(|m| m.to_ascii_lowercase().contains("mpegurl"))
            || url.path().to_ascii_lowercase().ends_with(".m3u8");
        sessions.insert(
            token.clone(),
            Target {
                url,
                headers,
                force_hls,
                expires: Instant::now() + SESSION_TTL,
            },
        );
        Ok(format!(
            "http://{}:{}/media/{}",
            self.address.ip(),
            self.address.port(),
            token
        ))
    }
}

/// Apenas cabeçalhos de requisição explicitamente úteis; bloqueia framing e hop-by-hop.
fn parse_request_headers(
    hints: Option<&Value>,
) -> Result<Vec<(header::HeaderName, HeaderValue)>, String> {
    let Some(obj) = hints
        .and_then(|v| v.get("proxyHeaders"))
        .and_then(|v| v.get("request"))
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    let allowed = ["authorization", "cookie", "origin", "referer", "user-agent"];
    let mut out = Vec::new();
    for (name, value) in obj {
        if !allowed.iter().any(|v| name.eq_ignore_ascii_case(v)) {
            continue;
        }
        let value = value
            .as_str()
            .ok_or("proxyHeaders contém valor não textual")?;
        if value.len() > 4096 {
            return Err("proxyHeaders excede o limite permitido".into());
        }
        let name = header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "nome de cabeçalho inválido")?;
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
    if method != Method::GET && method != Method::HEAD && method != Method::OPTIONS {
        return Err((
            StatusCode::METHOD_NOT_ALLOWED,
            "método não permitido".into(),
        ));
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
    }
    .ok_or((StatusCode::NOT_FOUND, "sessão expirada".into()))?;

    let original_origin = target.url.origin();
    let mut url = target.url;
    let mut response = None;
    for _ in 0..=MAX_REDIRECTS {
        if !url.username().is_empty()
            || url.password().is_some()
            || !crate::addon::is_safe_url(url.as_str()).await
        {
            return Err((StatusCode::FORBIDDEN, "redirecionamento bloqueado".into()));
        }
        let mut request = match method {
            Method::HEAD => state.client.head(url.clone()),
            _ => state.client.get(url.clone()),
        };
        for (name, value) in &target.headers {
            let sensitive = *name == header::AUTHORIZATION || *name == header::COOKIE;
            if !sensitive || url.origin() == original_origin {
                request = request.header(name, value);
            }
        }
        if let Some(range) = incoming.get(header::RANGE) {
            request = request.header(header::RANGE, range);
        }
        if let Some(if_range) = incoming.get(header::IF_RANGE) {
            request = request.header(header::IF_RANGE, if_range);
        }
        let upstream = request
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("falha na fonte: {e}")))?;
        if upstream.status().is_redirection() {
            let location = upstream
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or((StatusCode::BAD_GATEWAY, "redirect sem Location".into()))?;
            url = url
                .join(location)
                .map_err(|_| (StatusCode::BAD_GATEWAY, "redirect inválido".into()))?;
            continue;
        }
        response = Some(upstream);
        break;
    }
    let upstream = response.ok_or((StatusCode::BAD_GATEWAY, "redirecionamentos demais".into()))?;
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("application/dash+xml")
        || url.path().to_ascii_lowercase().ends_with(".mpd")
    {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "manifestos DASH ainda não são suportados pelo proxy seguro".into(),
        ));
    }
    let declared_hls = target.force_hls
        || content_type.contains("mpegurl")
        || url.path().to_ascii_lowercase().ends_with(".m3u8");
    let mut upstream = upstream;
    // Inspeciona um prefixo pequeno para detectar playlists HLS servidas sem extensão/MIME.
    // O prefixo é preservado e reenviado intacto quando o recurso é mídia binária.
    let mut prefix = Vec::new();
    if !declared_hls && status.is_success() && method != Method::HEAD {
        while prefix.len() < 10 {
            let next = tokio::time::timeout(Duration::from_secs(45), upstream.chunk())
                .await
                .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "timeout lendo mídia".into()))?
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
            let Some(chunk) = next else { break };
            prefix.extend_from_slice(&chunk);
        }
    }
    let is_hls = declared_hls
        || prefix.starts_with(b"#EXTM3U")
        || prefix.starts_with(b"\xef\xbb\xbf#EXTM3U");
    let mut builder = Response::builder().status(status);
    if is_hls && status.is_success() && method != Method::HEAD {
        // Limita durante a leitura: não usar bytes() aqui, pois poderia alocar uma resposta
        // ilimitada antes de verificar o tamanho.
        let mut bytes = prefix;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "playlist HLS excede 4 MiB".into(),
            ));
        }
        loop {
            let next = tokio::time::timeout(Duration::from_secs(45), upstream.chunk())
                .await
                .map_err(|_| {
                    (
                        StatusCode::GATEWAY_TIMEOUT,
                        "timeout lendo playlist HLS".into(),
                    )
                })?
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
            let Some(chunk) = next else { break };
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "playlist HLS excede 4 MiB".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let playlist = String::from_utf8(bytes)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "playlist HLS não é UTF-8".into()))?;
        // Uma playlist alcançada por redirect cross-origin não pode herdar Authorization/Cookie
        // da origem inicial ao registrar segmentos e playlists filhas.
        let playlist_headers = headers_for_origin(&target.headers, url.origin() == original_origin);
        let rewritten = rewrite_playlist(&state, &url, &playlist_headers, &playlist).await?;
        builder = builder.header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl");
        builder = builder.header(header::CACHE_CONTROL, "no-store");
        builder = builder
            .header("access-control-allow-origin", "*")
            .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
            .header("access-control-allow-headers", "Range, If-Range")
            .header(
                "access-control-expose-headers",
                "Content-Length, Content-Range, Accept-Ranges",
            );
        return builder
            .body(Body::from(rewritten))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::ETAG,
        header::LAST_MODIFIED,
    ] {
        if let Some(value) = upstream.headers().get(&name) {
            builder = builder.header(name, value);
        }
    }
    // O token é uma credencial local; não há cookies de sessão e o CORS não permite credenciais.
    builder = builder
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
        .header("access-control-allow-headers", "Range, If-Range")
        .header(
            "access-control-expose-headers",
            "Content-Length, Content-Range, Accept-Ranges",
        );
    let body = if method == Method::HEAD || status == StatusCode::NO_CONTENT {
        Body::empty()
    } else {
        let pending = if prefix.is_empty() {
            None
        } else {
            Some(Bytes::from(prefix))
        };
        let chunks = stream::unfold((upstream, pending), |(mut resp, pending)| async move {
            if let Some(first) = pending {
                return Some((Ok::<_, std::io::Error>(first), (resp, None)));
            }
            match tokio::time::timeout(Duration::from_secs(45), resp.chunk()).await {
                Ok(Ok(Some(bytes))) => Some((Ok(bytes), (resp, None))),
                Ok(Ok(None)) => None,
                Ok(Err(e)) => Some((Err(std::io::Error::other(e.to_string())), (resp, None))),
                Err(_) => Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timeout lendo mídia",
                    )),
                    (resp, None),
                )),
            }
        });
        Body::from_stream(chunks)
    };
    builder
        .body(body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}


/// Preserva cabeçalhos sensíveis somente quando o recurso continua na origem inicial.
fn headers_for_origin(
    headers: &[(header::HeaderName, HeaderValue)],
    same_origin: bool,
) -> Vec<(header::HeaderName, HeaderValue)> {
    if same_origin {
        return headers.to_vec();
    }
    headers
        .iter()
        .filter(|(name, _)| *name != header::AUTHORIZATION && *name != header::COOKIE)
        .cloned()
        .collect()
}

async fn rewrite_playlist(
    state: &ProxyState,
    base: &Url,
    headers: &[(header::HeaderName, HeaderValue)],
    playlist: &str,
) -> Result<String, (StatusCode, String)> {
    let playlist = playlist.trim_start_matches('\u{feff}');
    let mut output = Vec::new();
    for line in playlist.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            output.push(line.to_string());
        } else if trimmed.starts_with('#') {
            let mut rewritten = line.to_string();
            let mut search_from = 0;
            while let Some(relative) = rewritten[search_from..].find("URI=\"") {
                let start = search_from + relative + 5;
                let end = rewritten[start..].find('"').map(|n| start + n).ok_or((
                    StatusCode::BAD_GATEWAY,
                    "atributo URI inválido na playlist".into(),
                ))?;
                let original = rewritten[start..end].to_string();
                let local = register_child(state, base, &original, headers).await?;
                rewritten.replace_range(start..end, &local);
                search_from = start + local.len() + 1;
            }
            output.push(rewritten);
        } else {
            output.push(register_child(state, base, trimmed, headers).await?);
        }
    }
    Ok(format!("{}\n", output.join("\n")))
}

async fn register_child(
    state: &ProxyState,
    base: &Url,
    reference: &str,
    headers: &[(header::HeaderName, HeaderValue)],
) -> Result<String, (StatusCode, String)> {
    let url = base.join(reference).map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "URI filha inválida na playlist".into(),
        )
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || !crate::addon::is_safe_url(url.as_str()).await
    {
        return Err((
            StatusCode::FORBIDDEN,
            "playlist referencia destino não permitido".into(),
        ));
    }
    let token = Uuid::new_v4().simple().to_string();
    let mut sessions = state.sessions.write().await;
    sessions.retain(|_, v| v.expires > Instant::now());
    if sessions.len() >= 4096 {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "limite de recursos HLS atingido".into(),
        ));
    }
    let force_hls = url.path().to_ascii_lowercase().ends_with(".m3u8");
    let child_headers = if url.origin() == base.origin() {
        headers.to_vec()
    } else {
        headers
            .iter()
            .filter(|(name, _)| *name != header::AUTHORIZATION && *name != header::COOKIE)
            .cloned()
            .collect()
    };
    sessions.insert(
        token.clone(),
        Target {
            url,
            headers: child_headers,
            force_hls,
            expires: Instant::now() + SESSION_TTL,
        },
    );
    Ok(format!("/media/{token}"))
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

    #[test]
    fn redirected_hls_playlist_does_not_inherit_sensitive_headers() {
        let headers = vec![
            (header::AUTHORIZATION, HeaderValue::from_static("Bearer secret")),
            (header::COOKIE, HeaderValue::from_static("session=secret")),
            (header::REFERER, HeaderValue::from_static("https://origin.example/")),
        ];

        let same_origin = headers_for_origin(&headers, true);
        assert_eq!(same_origin.len(), 3);

        let redirected = headers_for_origin(&headers, false);
        assert_eq!(redirected.len(), 1);
        assert_eq!(redirected[0].0, header::REFERER);
    }
}
