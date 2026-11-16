//! Motor BitTorrent e servidor HTTP local com suporte a solicitações Range.

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, Response, StatusCode},
    routing::get,
    Router,
};
use librqbit::{AddTorrent, AddTorrentOptions, ManagedTorrentHandle, Session};
use serde::Serialize;
use tokio::{io::AsyncSeekExt, sync::{OnceCell, RwLock}};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const MAX_TORRENT_BYTES: usize = 8 * 1024 * 1024;
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov", "avi", "mpeg", "mpg", "ts"];

#[derive(Clone)]
struct StreamFile {
    torrent: ManagedTorrentHandle,
    file_index: usize,
    size: u64,
    mime_type: &'static str,
}

#[derive(Clone)]
struct HttpState {
    files: Arc<RwLock<HashMap<String, StreamFile>>>,
}

/// Motor compartilhado pela aplicação Tauri.
pub struct TorrentEngine {
    session: OnceCell<Arc<Session>>,
    download_dir: PathBuf,
    files: Arc<RwLock<HashMap<String, StreamFile>>>,
    listener: Mutex<Option<std::net::TcpListener>>,
    base_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentPlayback {
    pub stream_url: String,
    pub name: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

impl TorrentEngine {
    pub fn new(download_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&download_dir).map_err(|e| format!("pasta de torrents: {e}"))?;
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("não foi possível iniciar o servidor de mídia: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("não foi possível configurar servidor de mídia: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("endereço do servidor de mídia: {e}"))?;

        Ok(Self {
            session: OnceCell::new(),
            download_dir,
            files: Arc::new(RwLock::new(HashMap::new())),
            listener: Mutex::new(Some(listener)),
            base_url: format!("http://{addr}"),
        })
    }

    /// Inicia o servidor apenas em loopback; a URL do stream inclui um token aleatório.
    pub async fn serve(self: Arc<Self>) -> Result<(), String> {
        let listener = self
            .listener
            .lock()
            .map_err(|_| "estado do servidor indisponível".to_string())?
            .take()
            .ok_or_else(|| "servidor de mídia já iniciado".to_string())?;
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|e| format!("listener do servidor de mídia: {e}"))?;
        let router = Router::new()
            .route("/stream/{token}", get(stream_file))
            .with_state(HttpState { files: self.files.clone() });
        axum::serve(listener, router)
            .await
            .map_err(|e| format!("servidor de mídia parou: {e}"))
    }

    async fn session(&self) -> Result<Arc<Session>, String> {
        self.session
            .get_or_try_init(|| async {
                Session::new(self.download_dir.clone())
                    .await
                    .map_err(|e| format!("não foi possível iniciar o motor torrent: {e:#}"))
            })
            .await
            .cloned()
    }

    pub async fn add_magnet(
        &self,
        magnet: String,
        file_index: Option<usize>,
    ) -> Result<TorrentPlayback, String> {
        let magnet = magnet.trim();
        if !magnet.starts_with("magnet:?") || magnet.len() > 16 * 1024 {
            return Err("link magnet inválido ou grande demais".into());
        }
        let session = self.session().await?;
        let handle = session
            .add_torrent(AddTorrent::from_url(magnet), None)
            .await
            .map_err(|e| format!("não foi possível abrir o magnet: {e:#}"))?
            .into_handle()
            .ok_or_else(|| "o magnet não produziu um torrent reproduzível".to_string())?;
        self.make_playback(handle, file_index).await
    }

    pub async fn add_torrent_file(
        &self,
        bytes: Vec<u8>,
        file_index: Option<usize>,
    ) -> Result<TorrentPlayback, String> {
        if bytes.is_empty() || bytes.len() > MAX_TORRENT_BYTES {
            return Err("arquivo .torrent vazio ou maior que 8 MiB".into());
        }
        let session = self.session().await?;
        let handle = session
            .add_torrent(AddTorrent::from_bytes(bytes), Some(AddTorrentOptions::default()))
            .await
            .map_err(|e| format!("não foi possível abrir o arquivo .torrent: {e:#}"))?
            .into_handle()
            .ok_or_else(|| "o arquivo .torrent não produziu um torrent reproduzível".to_string())?;
        self.make_playback(handle, file_index).await
    }

    async fn make_playback(
        &self,
        handle: ManagedTorrentHandle,
        requested_index: Option<usize>,
    ) -> Result<TorrentPlayback, String> {
        let available = handle
            .with_metadata(|metadata| {
                metadata
                    .info
                    .iter_file_details()
                    .enumerate()
                    .map(|(index, file)| {
                        let name = file.filename.to_string();
                        let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                        (index, name, file.len, mime_for_extension(&extension))
                    })
                    .collect::<Vec<_>>()
            })
            .map_err(|e| format!("metadados do torrent indisponíveis: {e:#}"))?;

        let chosen = if let Some(index) = requested_index {
            available
                .iter()
                .find(|(candidate, _, _, mime)| *candidate == index && mime.is_some())
                .cloned()
                .ok_or_else(|| "o índice informado não corresponde a um arquivo de vídeo".to_string())?
        } else {
            available
                .iter()
                .filter(|(_, _, _, mime)| mime.is_some())
                .max_by_key(|(_, _, size, _)| *size)
                .cloned()
                .ok_or_else(|| "este torrent não contém um formato de vídeo compatível".to_string())?
        };
        let (file_index, file_name, size, mime) = chosen;
        let mime = mime.expect("arquivo de vídeo validado");
        let token = Uuid::new_v4().simple().to_string();
        self.files.write().await.insert(
            token.clone(),
            StreamFile { torrent: handle.clone(), file_index, size, mime_type: mime },
        );

        Ok(TorrentPlayback {
            stream_url: format!("{}/stream/{token}", self.base_url),
            name: handle.name().unwrap_or_else(|| "Torrent".into()),
            file_name,
            mime_type: mime.into(),
            size_bytes: size,
        })
    }
}

async fn stream_file(
    State(state): State<HttpState>,
    AxumPath(token): AxumPath<String>,
    headers: HeaderMap,
) -> Response<Body> {
    let file = match state.files.read().await.get(&token).cloned() {
        Some(file) => file,
        None => return response_error(StatusCode::NOT_FOUND, "stream não encontrado"),
    };

    let (start, end, partial) = match parse_range(headers.get(header::RANGE), file.size) {
        Ok(range) => range,
        Err(()) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", file.size))
                .header(header::ACCEPT_RANGES, "bytes")
                .body(Body::empty())
                .unwrap_or_else(|_| Response::new(Body::empty()));
        }
    };

    let mut stream = match file.torrent.stream(file.file_index).await {
        Ok(stream) => stream,
        Err(e) => return response_error(StatusCode::SERVICE_UNAVAILABLE, &format!("stream torrent: {e:#}")),
    };
    if let Err(e) = stream.seek(std::io::SeekFrom::Start(start)).await {
        return response_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("seek torrent: {e}"));
    }
    let length = end.saturating_sub(start).saturating_add(1);
    let body = Body::from_stream(ReaderStream::new(stream.take(length)));
    let status = if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, file.mime_type)
        .header(header::CONTENT_LENGTH, length.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    if partial {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{}", file.size));
    }
    builder.body(body).unwrap_or_else(|_| Response::new(Body::empty()))
}

fn parse_range(value: Option<&axum::http::HeaderValue>, size: u64) -> Result<(u64, u64, bool), ()> {
    if size == 0 {
        return Ok((0, 0, false));
    }
    let Some(value) = value else { return Ok((0, size - 1, false)); };
    let value = value.to_str().map_err(|_| ())?;
    let range = value.strip_prefix("bytes=").ok_or(())?;
    if range.contains(',') { return Err(()); }
    let (start, end) = range.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix: u64 = end.parse().map_err(|_| ())?;
        if suffix == 0 { return Err(()); }
        let start = size.saturating_sub(suffix);
        return Ok((start, size - 1, true));
    }
    let start: u64 = start.parse().map_err(|_| ())?;
    let end = if end.is_empty() { size - 1 } else { end.parse().map_err(|_| ())? };
    if start >= size || end < start { return Err(()); }
    Ok((start, end.min(size - 1), true))
}

fn response_error(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(message.to_owned()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn mime_for_extension(extension: &str) -> Option<&'static str> {
    match extension {
        "mp4" | "m4v" => Some("video/mp4"),
        "mkv" => Some("video/x-matroska"),
        "webm" => Some("video/webm"),
        "mov" => Some("video/quicktime"),
        "avi" => Some("video/x-msvideo"),
        "mpeg" | "mpg" => Some("video/mpeg"),
        "ts" => Some("video/mp2t"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parser_handles_full_and_partial_requests() {
        assert_eq!(parse_range(None, 100), Ok((0, 99, false)));
        let range = axum::http::HeaderValue::from_static("bytes=10-19");
        assert_eq!(parse_range(Some(&range), 100), Ok((10, 19, true)));
        let suffix = axum::http::HeaderValue::from_static("bytes=-10");
        assert_eq!(parse_range(Some(&suffix), 100), Ok((90, 99, true)));
        let invalid = axum::http::HeaderValue::from_static("bytes=100-");
        assert!(parse_range(Some(&invalid), 100).is_err());
    }

    #[test]
    fn accepts_common_video_mime_types() {
        assert_eq!(mime_for_extension("mp4"), Some("video/mp4"));
        assert_eq!(mime_for_extension("mkv"), Some("video/x-matroska"));
        assert_eq!(mime_for_extension("txt"), None);
    }
}
