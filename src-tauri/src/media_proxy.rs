//! Proxy HTTP local autenticado para fontes de vídeo.
//!
//! HTTP progressivo mantém streaming em blocos e Range; playlists HLS e manifestos DASH
//! reescrevem playlists, templates e recursos filhos para URLs locais autenticadas.
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::Response,
    routing::any,
    Router,
};
use futures::stream;
use quick_xml::{
    events::{BytesStart, BytesText, Event},
    reader::Reader,
    writer::Writer,
};
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
    force_dash: bool,
    allow_suffix: bool,
    default_query: Option<String>,
    expires: Instant,
}

#[derive(Clone)]
struct ProxyState {
    client: Client,
    sessions: Arc<RwLock<HashMap<String, Target>>>,
    #[cfg(test)]
    allow_loopback_targets: bool,
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
            #[cfg(test)]
            allow_loopback_targets: false,
        };
        let app = Router::new()
            .route("/media/{token}", any(proxy_request))
            .route("/media/{token}/{*resource}", any(proxy_child_request))
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
        if !is_safe_proxy_url(&self.state, url.as_str()).await {
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
        let force_dash = mime_type.is_some_and(|m| m.to_ascii_lowercase().contains("dash+xml"))
            || url.path().to_ascii_lowercase().ends_with(".mpd");
        sessions.insert(
            token.clone(),
            Target {
                url,
                headers,
                force_hls,
                force_dash,
                allow_suffix: false,
                default_query: None,
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
    proxy_request_inner(state, token, None, None, method, incoming).await
}

async fn proxy_child_request(
    State(state): State<ProxyState>,
    Path((token, resource)): Path<(String, String)>,
    method: Method,
    incoming: HeaderMap,
    uri: Uri,
) -> Result<Response, (StatusCode, String)> {
    proxy_request_inner(
        state,
        token,
        Some(resource),
        uri.query().map(str::to_owned),
        method,
        incoming,
    )
    .await
}

async fn proxy_request_inner(
    state: ProxyState,
    token: String,
    resource: Option<String>,
    child_query: Option<String>,
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

    let has_resource = resource.is_some();
    let mut url = if let Some(resource) = resource {
        if !target.allow_suffix || !valid_proxy_resource_path(&resource) {
            return Err((StatusCode::BAD_REQUEST, "recurso DASH inválido".into()));
        }
        let base = target.url.clone();
        let resolved = base
            .join(&resource)
            .map_err(|_| (StatusCode::BAD_REQUEST, "caminho DASH inválido".into()))?;
        if resolved.origin() != base.origin()
            || !resolved.username().is_empty()
            || resolved.password().is_some()
        {
            return Err((StatusCode::FORBIDDEN, "destino DASH bloqueado".into()));
        }
        resolved
    } else {
        target.url.clone()
    };
    if has_resource {
        if let Some(query) = child_query.as_deref().or(target.default_query.as_deref()) {
            url.set_query(Some(query));
        }
    }

    let original_origin = target.url.origin();
    let mut response = None;
    for _ in 0..=MAX_REDIRECTS {
        if !url.username().is_empty()
            || url.password().is_some()
            || !is_safe_proxy_url(&state, url.as_str()).await
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
    let declared_dash = target.force_dash
        || content_type.contains("application/dash+xml")
        || url.path().to_ascii_lowercase().ends_with(".mpd");
    let mut upstream = upstream;
    let mut builder = Response::builder().status(status);

    if declared_dash && status.is_success() {
        if method == Method::HEAD {
            return builder
                .header(header::CONTENT_TYPE, "application/dash+xml")
                .header(header::CACHE_CONTROL, "no-store")
                .header("access-control-allow-origin", "*")
                .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
                .header("access-control-allow-headers", "Range, If-Range")
                .body(Body::empty())
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
        let mut bytes = Vec::new();
        loop {
            let next = tokio::time::timeout(Duration::from_secs(45), upstream.chunk())
                .await
                .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "timeout lendo manifesto DASH".into()))?
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
            let Some(chunk) = next else { break };
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "manifesto DASH excede 4 MiB".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let manifest = String::from_utf8(bytes)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "manifesto DASH não é UTF-8".into()))?;
        let manifest_headers =
            headers_for_origin(&target.headers, url.origin() == original_origin);
        let rewritten = rewrite_mpd(&state, &url, &manifest_headers, &manifest).await?;
        return builder
            .header(header::CONTENT_TYPE, "application/dash+xml")
            .header(header::CACHE_CONTROL, "no-store")
            .header("access-control-allow-origin", "*")
            .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
            .header("access-control-allow-headers", "Range, If-Range")
            .header(
                "access-control-expose-headers",
                "Content-Length, Content-Range, Accept-Ranges",
            )
            .body(Body::from(rewritten))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    if content_type.contains("application/dash+xml")
        || url.path().to_ascii_lowercase().ends_with(".mpd")
    {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "resposta DASH inválida".into(),
        ));
    }
    let declared_hls = target.force_hls
        || content_type.contains("mpegurl")
        || url.path().to_ascii_lowercase().ends_with(".m3u8");
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
    if is_hls && status.is_success() && method != Method::HEAD {
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

fn valid_proxy_resource_path(resource: &str) -> bool {
    !resource.is_empty()
        && !resource.starts_with('/')
        && !resource.contains('\\')
        && !resource.chars().any(char::is_control)
        && resource
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Uses the production DNS/IP policy; tests may opt into literal loopback-only targets.
async fn is_safe_proxy_url(state: &ProxyState, raw_url: &str) -> bool {
    #[cfg(test)]
    if state.allow_loopback_targets {
        return Url::parse(raw_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    }
    #[cfg(not(test))]
    let _ = state;
    crate::addon::is_safe_url(raw_url).await
}


#[derive(Clone)]
struct MpdFrame {
    name: String,
    base: Url,
    is_base_url: bool,
    base_url_seen: bool,
}

fn xml_local_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name)
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn dash_error(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_GATEWAY, message.into())
}

/// Registra um caminho dinâmico DASH sob uma origem fixa. O caminho recebido posteriormente
/// é resolvido dentro dessa origem e validado novamente antes de cada requisição.
async fn register_dash_path(
    state: &ProxyState,
    base: &Url,
    reference: &str,
    headers: &[(header::HeaderName, HeaderValue)],
) -> Result<(String, Url), (StatusCode, String)> {
    let resolved = resolve_playlist_reference(base, reference)
        .map_err(|_| dash_error("referência DASH inválida"))?;
    if !is_safe_proxy_url(state, resolved.as_str()).await {
        return Err((
            StatusCode::FORBIDDEN,
            "manifesto DASH referencia destino não permitido".into(),
        ));
    }
    let mut origin_root = resolved.clone();
    origin_root.set_path("/");
    origin_root.set_query(None);
    origin_root.set_fragment(None);
    let token = Uuid::new_v4().simple().to_string();
    let mut sessions = state.sessions.write().await;
    sessions.retain(|_, v| v.expires > Instant::now());
    if sessions.len() >= 4096 {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "limite de recursos DASH atingido".into(),
        ));
    }
    let child_headers = headers_for_origin(headers, resolved.origin() == base.origin());
    sessions.insert(
        token.clone(),
        Target {
            url: origin_root,
            headers: child_headers,
            force_hls: false,
            force_dash: false,
            allow_suffix: true,
            default_query: resolved.query().map(str::to_owned),
            expires: Instant::now() + SESSION_TTL,
        },
    );
    let path = resolved.path().trim_start_matches('/');
    let mut local = format!("/media/{token}/{path}");
    if let Some(query) = resolved.query() {
        local.push('?');
        local.push_str(query);
    }
    Ok((local, resolved))
}

async fn rewrite_mpd(
    state: &ProxyState,
    manifest_url: &Url,
    headers: &[(header::HeaderName, HeaderValue)],
    manifest: &str,
) -> Result<String, (StatusCode, String)> {
    let mut reader = Reader::from_str(manifest);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut frames: Vec<MpdFrame> = Vec::new();
    let mut saw_mpd_root = false;

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(event)) => {
                let full_name = String::from_utf8_lossy(event.name().as_ref()).to_string();
                let name = xml_local_name(full_name.as_bytes());
                if frames.is_empty() {
                    if name != "MPD" {
                        return Err(dash_error("raiz do manifesto DASH inválida"));
                    }
                    saw_mpd_root = true;
                }
                if name == "BaseURL" {
                    if let Some(parent) = frames.last_mut() {
                        if parent.base_url_seen {
                            return Err(dash_error(
                                "manifesto DASH com múltiplas BaseURL no mesmo nível não suportado",
                            ));
                        }
                        parent.base_url_seen = true;
                    }
                }
                let base = frames
                    .last()
                    .map(|frame| frame.base.clone())
                    .unwrap_or_else(|| manifest_url.clone());
                let rewritten = rewrite_mpd_attributes(
                    state,
                    &base,
                    headers,
                    &name,
                    &event,
                )
                .await?;
                frames.push(MpdFrame {
                    name: name.clone(),
                    base,
                    is_base_url: name == "BaseURL",
                    base_url_seen: false,
                });
                writer
                    .write_event(Event::Start(rewritten))
                    .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
            }
            Ok(Event::Empty(event)) => {
                let full_name = String::from_utf8_lossy(event.name().as_ref()).to_string();
                let name = xml_local_name(full_name.as_bytes());
                if frames.is_empty() && name == "MPD" {
                    saw_mpd_root = true;
                }
                let base = frames
                    .last()
                    .map(|frame| frame.base.clone())
                    .unwrap_or_else(|| manifest_url.clone());
                let rewritten = rewrite_mpd_attributes(
                    state,
                    &base,
                    headers,
                    &name,
                    &event,
                    reader.decoder(),
                )
                .await?;
                writer
                    .write_event(Event::Empty(rewritten))
                    .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
            }
            Ok(Event::Text(event)) => {
                let current = frames.last();
                if current.is_some_and(|frame| frame.is_base_url || frame.name == "Location" || frame.name == "PatchLocation") {
                    let decoded = event
                        .decode()
                        .map_err(|e| dash_error(format!("texto DASH inválido: {e}")))?;
                    let unescaped = quick_xml::escape::unescape(&decoded)
                        .map_err(|_| dash_error("entidade XML não suportada no manifesto DASH"))?;
                    let original = unescaped.as_ref();
                    let trimmed = original.trim();
                    if trimmed.is_empty() {
                        writer
                            .write_event(Event::Text(event.into_owned()))
                            .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
                    } else if current.is_some_and(|frame| frame.is_base_url) {
                        let base = current.expect("BaseURL frame missing").base.clone();
                        let (local, resolved) =
                            register_dash_path(state, &base, trimmed, headers).await?;
                        let frame = frames.last_mut().expect("BaseURL frame missing");
                        frame.base = resolved;
                        let leading = original.len() - original.trim_start().len();
                        let trailing = original.len() - original.trim_end().len();
                        let end = original.len().saturating_sub(trailing);
                        let replacement = format!("{}{}{}", &original[..leading], local, &original[end..]);
                        writer
                            .write_event(Event::Text(BytesText::new(&replacement)))
                            .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
                    } else {
                        let base = current.expect("Location frame missing").base.clone();
                        let local = register_child(state, &base, trimmed, headers).await?;
                        let leading = original.len() - original.trim_start().len();
                        let trailing = original.len() - original.trim_end().len();
                        let end = original.len().saturating_sub(trailing);
                        let replacement = format!("{}{}{}", &original[..leading], local, &original[end..]);
                        writer
                            .write_event(Event::Text(BytesText::new(&replacement)))
                            .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
                    }
                } else {
                    writer
                        .write_event(Event::Text(event.into_owned()))
                        .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
                }
            }
            Ok(Event::End(event)) => {
                if let Some(frame) = frames.pop() {
                    if frame.is_base_url {
                        if let Some(parent) = frames.last_mut() {
                            parent.base = frame.base;
                        }
                    }
                }
                writer
                    .write_event(Event::End(event.into_owned()))
                    .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
            }
            Ok(Event::CData(event)) => {
                if frames.last().is_some_and(|frame| frame.is_base_url) {
                    let decoded = event
                        .decode()
                        .map_err(|e| dash_error(format!("CDATA DASH inválido: {e}")))?;
                    let trimmed = decoded.trim();
                    if trimmed.is_empty() {
                        writer
                            .write_event(Event::CData(event.into_owned()))
                            .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
                    } else {
                        let base = frames.last().expect("BaseURL frame missing").base.clone();
                        let (local, resolved) =
                            register_dash_path(state, &base, trimmed, headers).await?;
                        frames.last_mut().expect("BaseURL frame missing").base = resolved;
                        writer
                            .write_event(Event::Text(BytesText::new(&local)))
                            .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
                    }
                } else {
                    writer
                        .write_event(Event::CData(event.into_owned()))
                        .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
                }
            }
            Ok(event) => {
                writer
                    .write_event(event.into_owned())
                    .map_err(|e| dash_error(format!("não foi possível serializar DASH: {e}")))?;
            }
            Err(error) => return Err(dash_error(format!("manifesto DASH malformado: {error}"))),
        }
    }

    if !saw_mpd_root || !frames.is_empty() {
        return Err(dash_error("manifesto DASH incompleto"));
    }
    String::from_utf8(writer.into_inner())
        .map_err(|_| dash_error("manifesto DASH serializado não é UTF-8"))
}

async fn rewrite_mpd_attributes(
    state: &ProxyState,
    base: &Url,
    headers: &[(header::HeaderName, HeaderValue)],
    element: &str,
    event: &BytesStart<'_>,
) -> Result<BytesStart<'static>, (StatusCode, String)> {
    let name = String::from_utf8_lossy(event.name().as_ref()).to_string();
    let mut output = BytesStart::new(name.as_str());
    for attribute in event.attributes() {
        let attribute = attribute.map_err(|_| dash_error("atributo XML inválido no manifesto DASH"))?;
        let key = String::from_utf8_lossy(attribute.key.as_ref()).to_string();
        let local_key = xml_local_name(key.as_bytes());
        let value = attribute
            .normalized_value()
            .map_err(|_| dash_error("valor de atributo XML inválido no manifesto DASH"))?
            .into_owned();
        let is_template = element == "SegmentTemplate"
            && matches!(local_key.as_str(), "media" | "initialization" | "index");
        let is_static = (element == "SegmentURL" && matches!(local_key.as_str(), "media" | "index"))
            || (matches!(element, "Initialization" | "RepresentationIndex")
                && local_key == "sourceURL")
            || (element == "Period" && local_key == "href")
            || (element == "UTCTiming" && local_key == "value"
                && (value.starts_with("http://") || value.starts_with("https://")));
        let rewritten = if value.trim().is_empty() {
            value
        } else if is_template {
            register_dash_path(state, base, &value, headers).await?.0
        } else if is_static {
            register_child(state, base, &value, headers).await?
        } else {
            value
        };
        output.push_attribute((key.as_str(), rewritten.as_str()));
    }
    Ok(output)
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

/// Extrai as referências de uma linha HLS e seus intervalos de bytes.
fn playlist_line_references(line: &str) -> Result<Vec<(std::ops::Range<usize>, String)>, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if !trimmed.starts_with('#') {
        let start = line.find(trimmed).ok_or("linha HLS inválida")?;
        return Ok(vec![(start..start + trimmed.len(), trimmed.to_string())]);
    }

    let mut references = Vec::new();
    let mut search_from = 0;
    while let Some(relative) = line[search_from..].find("URI=\"") {
        let start = search_from + relative + 5;
        let end = line[start..]
            .find('"')
            .map(|n| start + n)
            .ok_or("atributo URI inválido na playlist")?;
        references.push((start..end, line[start..end].to_string()));
        search_from = end + 1;
    }
    Ok(references)
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
        let references = playlist_line_references(line).map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "atributo URI inválido na playlist".into(),
            )
        })?;
        let mut rewritten = line.to_string();
        // Substitui do fim para o início para preservar os offsets originais.
        for (range, original) in references.into_iter().rev() {
            let local = register_child(state, base, &original, headers).await?;
            rewritten.replace_range(range, &local);
        }
        output.push(rewritten);
    }
    Ok(format!("{}\n", output.join("\n")))
}

/// Resolve uma referência HLS e rejeita esquemas, hosts e credenciais que não sejam HTTP(S).
/// A validação DNS/IP é feita separadamente por is_safe_url antes do registro.
fn resolve_playlist_reference(base: &Url, reference: &str) -> Result<Url, String> {
    let url = base
        .join(reference)
        .map_err(|_| "referência HLS inválida".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("referência HLS não permitida".into());
    }
    Ok(url)
}

async fn register_child(
    state: &ProxyState,
    base: &Url,
    reference: &str,
    headers: &[(header::HeaderName, HeaderValue)],
) -> Result<String, (StatusCode, String)> {
    let url = resolve_playlist_reference(base, reference).map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "URI filha inválida na playlist".into(),
        )
    })?;
    if !is_safe_proxy_url(state, url.as_str()).await {
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
            force_dash: false,
            allow_suffix: false,
            default_query: None,
            expires: Instant::now() + SESSION_TTL,
        },
    );
    Ok(format!("/media/{token}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_test_state() -> ProxyState {
        ProxyState {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            allow_loopback_targets: true,
        }
    }

    async fn start_test_server(app: Router) -> SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        address
    }

    async fn add_test_target(state: &ProxyState, url: Url, force_hls: bool) -> String {
        let token = Uuid::new_v4().simple().to_string();
        state.sessions.write().await.insert(
            token.clone(),
            Target {
                url,
                headers: Vec::new(),
                force_hls,
                force_dash: false,
                allow_suffix: false,
                default_query: None,
                expires: Instant::now() + SESSION_TTL,
            },
        );
        token
    }

    async fn request_test_resource(
        state: &ProxyState,
        token: String,
        headers: HeaderMap,
    ) -> Response {
        proxy_request(State(state.clone()), Path(token), Method::GET, headers)
            .await
            .unwrap()
    }

    async fn request_test_child_resource(
        state: &ProxyState,
        token: String,
        resource: String,
        query: Option<String>,
    ) -> Response {
        proxy_request_inner(
            state.clone(),
            token,
            Some(resource),
            query,
            Method::GET,
            HeaderMap::new(),
        )
        .await
        .unwrap()
    }

    fn rewritten_token(text: &str) -> String {
        let marker = "/media/";
        let start = text.find(marker).expect("proxy URL missing") + marker.len();
        text[start..]
            .split(|ch: char| !ch.is_ascii_hexdigit())
            .next()
            .unwrap()
            .to_string()
    }

    async fn master_playlist() -> &'static str {
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1200000\nvariant.m3u8\n"
    }

    async fn variant_playlist() -> &'static str {
        "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:4,\nsegment.ts\n#EXT-X-ENDLIST\n"
    }

    async fn test_segment(headers: HeaderMap) -> Response {
        if headers
            .get(header::RANGE)
            .is_some_and(|value| value == "bytes=1-3")
        {
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, "video/mp2t")
                .header(header::CONTENT_RANGE, "bytes 1-3/6")
                .header(header::ACCEPT_RANGES, "bytes")
                .body(Body::from("bcd"))
                .unwrap()
        } else {
            Response::builder()
                .header(header::CONTENT_TYPE, "video/mp2t")
                .body(Body::from("abcdef"))
                .unwrap()
        }
    }

    async fn capture_redirected_playlist_headers(
        State(seen): State<Arc<std::sync::Mutex<Vec<bool>>>>,
        headers: HeaderMap,
    ) -> &'static str {
        seen.lock().unwrap().push(
            headers.contains_key(header::AUTHORIZATION) || headers.contains_key(header::COOKIE),
        );
        "#EXTM3U\nsegment.ts\n"
    }

    async fn capture_redirected_segment_headers(
        State(seen): State<Arc<std::sync::Mutex<Vec<bool>>>>,
        headers: HeaderMap,
    ) -> &'static str {
        seen.lock().unwrap().push(
            headers.contains_key(header::AUTHORIZATION) || headers.contains_key(header::COOKIE),
        );
        "segment"
    }

    #[tokio::test]
    async fn hls_redirect_to_another_origin_drops_authorization_and_cookie() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let destination = Router::new()
            .route(
                "/redirected.m3u8",
                axum::routing::get(capture_redirected_playlist_headers),
            )
            .route(
                "/segment.ts",
                axum::routing::get(capture_redirected_segment_headers),
            )
            .with_state(seen.clone());
        let destination_address = start_test_server(destination).await;

        let redirect_target = format!("http://{destination_address}/redirected.m3u8");
        let redirect = Router::new().route(
            "/start.m3u8",
            axum::routing::get(move || async move {
                axum::response::Redirect::temporary(&redirect_target)
            }),
        );
        let redirect_address = start_test_server(redirect).await;

        let state = local_test_state();
        let token = Uuid::new_v4().simple().to_string();
        state.sessions.write().await.insert(
            token.clone(),
            Target {
                url: Url::parse(&format!("http://{redirect_address}/start.m3u8")).unwrap(),
                force_hls: true,
                force_dash: false,
                allow_suffix: false,
                default_query: None,
                headers: vec![
                    (
                        header::AUTHORIZATION,
                        HeaderValue::from_static("Bearer integration-test-secret"),
                    ),
                    (
                        header::COOKIE,
                        HeaderValue::from_static("session=integration-test-secret"),
                    ),
                ],
                force_hls: true,
                expires: Instant::now() + SESSION_TTL,
            },
        );

        let playlist_response = request_test_resource(&state, token, HeaderMap::new()).await;
        assert_eq!(playlist_response.status(), StatusCode::OK);
        let playlist_body = axum::body::to_bytes(playlist_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let playlist = String::from_utf8(playlist_body.to_vec()).unwrap();
        let segment_token = rewritten_token(
            playlist
                .lines()
                .find(|line| line.starts_with("/media/"))
                .expect("redirected segment URI missing"),
        );

        let segment_response = request_test_resource(&state, segment_token, HeaderMap::new()).await;
        assert_eq!(segment_response.status(), StatusCode::OK);
        assert_eq!(
            &axum::body::to_bytes(segment_response.into_body(), 1024)
                .await
                .unwrap()[..],
            b"segment"
        );
        assert_eq!(*seen.lock().unwrap(), vec![false, false]);
    }

    #[tokio::test]
    async fn hls_proxy_integration_rewrites_master_variant_key_and_segment_range() {
        let origin = Router::new()
            .route("/master.m3u8", axum::routing::get(master_playlist))
            .route("/variant.m3u8", axum::routing::get(variant_playlist))
            .route("/segment.ts", axum::routing::get(test_segment))
            .route(
                "/key.bin",
                axum::routing::get(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/octet-stream")],
                        "0123456789abcdef",
                    )
                }),
            );
        let origin_address = start_test_server(origin).await;
        let state = local_test_state();
        let master_url = Url::parse(&format!("http://{origin_address}/master.m3u8")).unwrap();
        let master_token = add_test_target(&state, master_url, true).await;

        let master_response = request_test_resource(&state, master_token, HeaderMap::new()).await;
        assert_eq!(master_response.status(), StatusCode::OK);
        let master_body = axum::body::to_bytes(master_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let master_text = String::from_utf8(master_body.to_vec()).unwrap();
        assert!(master_text.contains("#EXTM3U"));
        let variant_token = rewritten_token(&master_text);

        let variant_response = request_test_resource(&state, variant_token, HeaderMap::new()).await;
        assert_eq!(variant_response.status(), StatusCode::OK);
        let variant_body = axum::body::to_bytes(variant_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let variant_text = String::from_utf8(variant_body.to_vec()).unwrap();
        assert!(variant_text.contains("#EXT-X-KEY:METHOD=AES-128"));
        let key_token = rewritten_token(
            variant_text
                .split("URI=\"")
                .nth(1)
                .expect("AES-128 key URI missing"),
        );
        let segment_token = rewritten_token(
            variant_text
                .lines()
                .find(|line| line.starts_with("/media/"))
                .expect("segment URI missing"),
        );

        let key_response = request_test_resource(&state, key_token, HeaderMap::new()).await;
        assert_eq!(key_response.status(), StatusCode::OK);
        let key_body = axum::body::to_bytes(key_response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&key_body[..], b"0123456789abcdef");

        let mut range_headers = HeaderMap::new();
        range_headers.insert(header::RANGE, HeaderValue::from_static("bytes=1-3"));
        let segment_response = request_test_resource(&state, segment_token, range_headers).await;
        assert_eq!(segment_response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            segment_response
                .headers()
                .get(header::CONTENT_RANGE)
                .unwrap(),
            "bytes 1-3/6"
        );
        let segment_body = axum::body::to_bytes(segment_response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&segment_body[..], b"bcd");
    }

    #[tokio::test]
    async fn dash_proxy_rewrites_base_url_and_dynamic_segment_templates() {
        const MANIFEST: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static">
  <Period>
    <BaseURL>video/</BaseURL>
    <AdaptationSet>
      <Representation id="v1">
        <SegmentTemplate timescale="1" initialization="init.mp4" media="chunk-$Number$.m4s" startNumber="1">
          <SegmentTimeline><S t="0" d="4" r="1"/></SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
        let origin = Router::new()
            .route(
                "/manifest.mpd",
                axum::routing::get(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/dash+xml")],
                        MANIFEST,
                    )
                }),
            )
            .route(
                "/video/chunk-1.m4s",
                axum::routing::get(|| async { "segment-one" }),
            )
            .route(
                "/video/init.mp4",
                axum::routing::get(|| async { "initialization" }),
            );
        let origin_address = start_test_server(origin).await;
        let state = local_test_state();
        let manifest_url =
            Url::parse(&format!("http://{origin_address}/manifest.mpd")).unwrap();
        let manifest_token = add_test_target(&state, manifest_url, false).await;

        let manifest_response =
            request_test_resource(&state, manifest_token, HeaderMap::new()).await;
        assert_eq!(manifest_response.status(), StatusCode::OK);
        assert_eq!(
            manifest_response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/dash+xml"
        );
        let body = axum::body::to_bytes(manifest_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let rewritten = String::from_utf8(body.to_vec()).unwrap();
        assert!(rewritten.contains("<BaseURL>/media/"));
        assert!(rewritten.contains("$Number$"));

        let media_start = rewritten.find("media=\"").unwrap() + "media=\"".len();
        let media_end = rewritten[media_start..].find('"').unwrap() + media_start;
        let media_template = &rewritten[media_start..media_end];
        let media_url = media_template.replace("$Number$", "1");
        let (segment_token, segment_path, segment_query) = dash_local_parts(&media_url);
        let segment_response = request_test_child_resource(
            &state,
            segment_token,
            segment_path,
            segment_query,
        )
        .await;
        assert_eq!(segment_response.status(), StatusCode::OK);
        assert_eq!(
            &axum::body::to_bytes(segment_response.into_body(), 1024)
                .await
                .unwrap()[..],
            b"segment-one"
        );

        let init_start =
            rewritten.find("initialization=\"").unwrap() + "initialization=\"".len();
        let init_end = rewritten[init_start..].find('"').unwrap() + init_start;
        let (init_token, init_path, init_query) =
            dash_local_parts(&rewritten[init_start..init_end]);
        let init_response =
            request_test_child_resource(&state, init_token, init_path, init_query).await;
        assert_eq!(init_response.status(), StatusCode::OK);
        assert_eq!(
            &axum::body::to_bytes(init_response.into_body(), 1024)
                .await
                .unwrap()[..],
            b"initialization"
        );
    }

    fn dash_local_parts(local: &str) -> (String, String, Option<String>) {
        let parsed = Url::parse(&format!("http://proxy.invalid{local}")).unwrap();
        let mut parts = parsed.path().trim_start_matches('/').splitn(3, '/');
        assert_eq!(parts.next(), Some("media"));
        let token = parts.next().expect("DASH proxy token missing").to_string();
        let resource = parts.next().expect("DASH resource path missing").to_string();
        (token, resource, parsed.query().map(str::to_owned))
    }

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
    fn hls_relative_references_resolve_against_playlist_directory() {
        let base = Url::parse("https://cdn.example/live/master.m3u8").unwrap();
        let resolved = resolve_playlist_reference(&base, "720p/variant.m3u8").unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://cdn.example/live/720p/variant.m3u8"
        );
    }

    #[test]
    fn hls_root_relative_and_protocol_relative_references_resolve() {
        let base = Url::parse("https://cdn.example/live/master.m3u8").unwrap();
        assert_eq!(
            resolve_playlist_reference(&base, "/segments/one.ts")
                .unwrap()
                .as_str(),
            "https://cdn.example/segments/one.ts"
        );
        assert_eq!(
            resolve_playlist_reference(&base, "//media.example/segment.ts")
                .unwrap()
                .as_str(),
            "https://media.example/segment.ts"
        );
    }

    #[test]
    fn hls_query_only_reference_keeps_current_resource_path() {
        let base = Url::parse("https://cdn.example/live/index.m3u8?token=old").unwrap();
        let resolved = resolve_playlist_reference(&base, "?token=new").unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://cdn.example/live/index.m3u8?token=new"
        );
    }

    #[test]
    fn hls_references_reject_non_http_schemes_and_embedded_credentials() {
        let base = Url::parse("https://cdn.example/live/master.m3u8").unwrap();
        assert!(resolve_playlist_reference(&base, "javascript:alert(1)").is_err());
        assert!(resolve_playlist_reference(&base, "data:text/plain,test").is_err());
        assert!(
            resolve_playlist_reference(&base, "https://user:pass@cdn.example/segment.ts").is_err()
        );
    }

    #[test]
    fn hls_media_playlist_extracts_segments_and_whitespace_safely() {
        let line = "  segments/part-001.ts?token=a  ";
        let refs = playlist_line_references(line).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].1, "segments/part-001.ts?token=a");
        assert_eq!(&line[refs[0].0.clone()], refs[0].1);
        assert!(playlist_line_references("   ").unwrap().is_empty());
    }

    #[test]
    fn hls_master_playlist_extracts_nested_variants_and_renditions() {
        let variant = playlist_line_references("#EXT-X-STREAM-INF:BANDWIDTH=2500000").unwrap();
        assert!(variant.is_empty());
        let rendition = playlist_line_references(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",URI=\"audio/pt.m3u8\"",
        )
        .unwrap();
        assert_eq!(rendition.len(), 1);
        assert_eq!(rendition[0].1, "audio/pt.m3u8");

        let iframe = playlist_line_references(
            "#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=800000,URI=\"iframe/low.m3u8\"",
        )
        .unwrap();
        assert_eq!(iframe[0].1, "iframe/low.m3u8");
        let nested = playlist_line_references("video/1080p/index.m3u8").unwrap();
        assert_eq!(nested[0].1, "video/1080p/index.m3u8");
    }

    #[test]
    fn hls_key_map_part_and_preload_uri_attributes_are_extracted() {
        let lines = [
            "#EXT-X-KEY:METHOD=AES-128,URI=\"keys/key.bin\",IV=0x01",
            "#EXT-X-SESSION-KEY:METHOD=AES-128,URI=\"keys/session.bin\"",
            "#EXT-X-MAP:URI=\"init/init.mp4\",BYTERANGE=\"720@0\"",
            "#EXT-X-PART:DURATION=0.5,URI=\"parts/part-01.m4s\"",
            "#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"parts/part-02.m4s\"",
            "#EXT-X-RENDITION-REPORT:URI=\"../audio/index.m3u8\"",
        ];
        let expected = [
            "keys/key.bin",
            "keys/session.bin",
            "init/init.mp4",
            "parts/part-01.m4s",
            "parts/part-02.m4s",
            "../audio/index.m3u8",
        ];
        for (line, expected) in lines.into_iter().zip(expected) {
            let refs = playlist_line_references(line).unwrap();
            assert_eq!(refs.len(), 1, "{line}");
            assert_eq!(refs[0].1, expected, "{line}");
        }
    }

    #[test]
    fn hls_uri_attributes_reject_unterminated_quotes() {
        assert!(playlist_line_references("#EXT-X-KEY:METHOD=AES-128,URI=\"keys/key.bin").is_err());
    }

    #[test]
    fn redirected_hls_playlist_does_not_inherit_sensitive_headers() {
        let headers = vec![
            (
                header::AUTHORIZATION,
                HeaderValue::from_static("Bearer secret"),
            ),
            (header::COOKIE, HeaderValue::from_static("session=secret")),
            (
                header::REFERER,
                HeaderValue::from_static("https://origin.example/"),
            ),
        ];

        let same_origin = headers_for_origin(&headers, true);
        assert_eq!(same_origin.len(), 3);

        let redirected = headers_for_origin(&headers, false);
        assert_eq!(redirected.len(), 1);
        assert_eq!(redirected[0].0, header::REFERER);
    }
}
