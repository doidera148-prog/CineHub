//! Motor BitTorrent e servidor HTTP local com suporte a solicitações Range.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, Response, StatusCode},
    routing::get,
    Router,
};
use librqbit::{
    api::TorrentIdOrHash, AddTorrent, DhtSessionConfig, ManagedTorrent, Session, SessionOptions,
};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::{OnceCell, RwLock},
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const MAX_TORRENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_MAGNET_BYTES: usize = 16 * 1024;
const OPEN_TIMEOUT: Duration = Duration::from_secs(90);
/// Bound individual range reads to keep seeks/buffering responsive on low-memory devices.
const MAX_RANGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
struct StreamFile {
    torrent: Arc<ManagedTorrent>,
    file_index: usize,
    size: u64,
    mime_type: &'static str,
    videos: Vec<Candidate>,
}

/// Um arquivo de vídeo dentro do torrent, para a interface escolher episódio ou idioma.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoFile {
    pub index: usize,
    pub name: String,
    pub size_bytes: u64,
}

impl From<&Candidate> for VideoFile {
    fn from(c: &Candidate) -> Self {
        Self {
            index: c.index,
            name: c.name.clone(),
            size_bytes: c.size,
        }
    }
}

#[derive(Clone)]
struct HttpState {
    files: Arc<RwLock<HashMap<String, StreamFile>>>,
}

pub struct TorrentEngine {
    session: OnceCell<Arc<Session>>,
    download_dir: PathBuf,
    files: Arc<RwLock<HashMap<String, StreamFile>>>,
    active: tokio::sync::Mutex<Option<usize>>,
    listener: Mutex<Option<std::net::TcpListener>>,
    base_url: String,
    server_failed: Arc<AtomicBool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentNetworkStats {
    pub active: bool,
    pub state: String,
    pub download_speed_bytes: u64,
    pub upload_speed_bytes: u64,
    pub peers: usize,
    pub peers_seen: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
}

pub struct TorrentPlayback {
    pub token: String,
    pub stream_url: String,
    pub name: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub videos: Vec<VideoFile>,
}

#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    index: usize,
    name: String,
    size: u64,
    mime: &'static str,
}

struct Plan {
    name: String,
    file_index: usize,
    file_name: String,
    size: u64,
    mime: &'static str,
    videos: Vec<Candidate>,
}

impl TorrentEngine {
    pub fn new(download_dir: PathBuf) -> Result<Self, String> {
        if download_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&download_dir) {
                eprintln!("aviso: não consegui limpar a pasta de torrents: {e}");
            }
        }
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
            active: tokio::sync::Mutex::new(None),
            listener: Mutex::new(Some(listener)),
            base_url: format!("http://{addr}"),
            server_failed: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The local media endpoint binds only to loopback; stream URLs include a random token.
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
            .route("/stream/{token}", get(stream_file).head(head_stream_file))
            .with_state(HttpState {
                files: self.files.clone(),
            });
        let result = axum::serve(listener, router)
            .await
            .map_err(|e| format!("servidor de mídia parou: {e}"));
        if result.is_err() {
            self.server_failed.store(true, Ordering::Relaxed);
        }
        result
    }

    async fn session(&self) -> Result<Arc<Session>, String> {
        self.session
            .get_or_try_init(|| async {
                let opts = SessionOptions {
                    disable_local_service_discovery: true,
                    dht: Some(DhtSessionConfig {
                        persistence: None,
                        ..DhtSessionConfig::default()
                    }),
                    ..SessionOptions::default()
                };
                Session::new_with_opts(self.download_dir.clone(), opts)
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
        if !magnet.starts_with("magnet:?") || magnet.len() > MAX_MAGNET_BYTES {
            return Err("link magnet inválido ou grande demais".into());
        }
        self.open(AddTorrent::from_url(magnet), file_index).await
    }

    pub async fn add_torrent_file(
        &self,
        bytes: Vec<u8>,
        file_index: Option<usize>,
    ) -> Result<TorrentPlayback, String> {
        if bytes.is_empty() || bytes.len() > MAX_TORRENT_BYTES {
            return Err("arquivo .torrent vazio ou maior que 8 MiB".into());
        }
        self.open(AddTorrent::from_bytes(bytes), file_index).await
    }

    async fn open(
        &self,
        add: AddTorrent<'_>,
        requested: Option<usize>,
    ) -> Result<TorrentPlayback, String> {
        if self.server_failed.load(Ordering::Relaxed) {
            return Err("o servidor de mídia parou; reinicie o CineHub".into());
        }
        let mut active = self.active.lock().await;
        let session = self.session().await?;

        let added = tokio::time::timeout(OPEN_TIMEOUT, session.add_torrent(add, None))
            .await
            .map_err(|_| "o torrent não respondeu a tempo (sem fontes ou sem conexão)".to_string())?
            .map_err(|e| format!("não foi possível abrir o torrent: {e:#}"))?;
        let handle = added
            .into_handle()
            .ok_or_else(|| "o torrent não produziu um arquivo reproduzível".to_string())?;
        let id = handle.id();

        match tokio::time::timeout(OPEN_TIMEOUT, handle.wait_until_initialized()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = session.delete(TorrentIdOrHash::Id(id), true).await;
                return Err(format!("não foi possível inicializar o torrent: {e:#}"));
            }
            Err(_) => {
                let _ = session.delete(TorrentIdOrHash::Id(id), true).await;
                return Err("a inicialização do torrent demorou demais".into());
            }
        }

        let plan = match prepare(&session, &handle, requested).await {
            Ok(plan) => plan,
            Err(e) => {
                if *active != Some(id) {
                    let _ = session.delete(TorrentIdOrHash::Id(id), true).await;
                }
                return Err(e);
            }
        };

        if let Some(previous) = active.replace(id).filter(|prev| *prev != id) {
            let _ = session.delete(TorrentIdOrHash::Id(previous), true).await;
        }
        let token = Uuid::new_v4().simple().to_string();
        {
            let mut files = self.files.write().await;
            files.clear();
            files.insert(
                token.clone(),
                StreamFile {
                    torrent: handle,
                    file_index: plan.file_index,
                    size: plan.size,
                    mime_type: plan.mime,
                    videos: plan.videos.clone(),
                },
            );
        }

        Ok(TorrentPlayback {
            stream_url: format!("{}/stream/{token}", self.base_url),
            token,
            name: plan.name,
            file_name: plan.file_name,
            mime_type: plan.mime.into(),
            size_bytes: plan.size,
            videos: plan.videos.iter().map(VideoFile::from).collect(),
        })
    }

    /// Snapshot leve da atividade de rede do torrent atualmente aberto.
    pub async fn network_stats(&self) -> TorrentNetworkStats {
        let handle = {
            let files = self.files.read().await;
            files.values().next().map(|entry| entry.torrent.clone())
        };
        let Some(handle) = handle else {
            return TorrentNetworkStats {
                active: false,
                state: "inativo".into(),
                download_speed_bytes: 0,
                upload_speed_bytes: 0,
                peers: 0,
                peers_seen: 0,
                downloaded_bytes: 0,
                total_bytes: 0,
                error: None,
            };
        };
        let stats = handle.stats();
        let (download_speed_bytes, upload_speed_bytes, peers, peers_seen) =
            match &stats.live {
                Some(live) => (
                    (live.download_speed.mbps.max(0.0) * 125_000.0) as u64,
                    (live.upload_speed.mbps.max(0.0) * 125_000.0) as u64,
                    live.snapshot.peer_stats.live as usize,
                    live.snapshot.peer_stats.seen as usize,
                ),
                None => (0, 0, 0, 0),
            };
        TorrentNetworkStats {
            active: true,
            state: stats.state.to_string(),
            download_speed_bytes,
            upload_speed_bytes,
            peers,
            peers_seen,
            downloaded_bytes: stats.progress_bytes,
            total_bytes: stats.total_bytes,
            error: stats.error.clone(),
        }
    }

    /// Troca o arquivo que toca dentro de um torrent já aberto (outro episódio, outro idioma).
    pub async fn select_file(&self, token: &str, index: usize) -> Result<TorrentPlayback, String> {
        const GONE: &str = "o torrent não está mais aberto; abra de novo";
        let (torrent, videos) = {
            let files = self.files.read().await;
            let entry = files.get(token).ok_or(GONE)?;
            (entry.torrent.clone(), entry.videos.clone())
        };
        let chosen = pick_video(&videos, Some(index))?;
        let session = self.session().await?;
        session
            .update_only_files(&torrent, &HashSet::from([chosen.index]))
            .await
            .map_err(|e| format!("não foi possível selecionar o arquivo: {e:#}"))?;
        let mut files = self.files.write().await;
        let entry = files.get_mut(token).ok_or(GONE)?;
        entry.file_index = chosen.index;
        entry.size = chosen.size;
        entry.mime_type = chosen.mime;
        Ok(TorrentPlayback {
            // O índice na URL faz o player recarregar o arquivo novo, e não reaproveitar o antigo.
            stream_url: format!("{}/stream/{token}?file={}", self.base_url, chosen.index),
            token: token.to_string(),
            name: torrent.name().unwrap_or_else(|| "Torrent".into()),
            file_name: chosen.name.clone(),
            mime_type: chosen.mime.into(),
            size_bytes: chosen.size,
            videos: videos.iter().map(VideoFile::from).collect(),
        })
    }
}

async fn prepare(
    session: &Arc<Session>,
    handle: &Arc<ManagedTorrent>,
    requested: Option<usize>,
) -> Result<Plan, String> {
    let files = handle
        .with_metadata(|metadata| {
            metadata
                .info
                .iter_file_details()
                .enumerate()
                .map(|(index, file)| (index, file.filename.to_string(), file.len))
                .collect::<Vec<_>>()
        })
        .map_err(|e| format!("metadados do torrent indisponíveis: {e:#}"))?;
    let videos = video_candidates(files);
    let chosen = pick_video(&videos, requested)?;
    session
        .update_only_files(handle, &HashSet::from([chosen.index]))
        .await
        .map_err(|e| format!("não foi possível selecionar o arquivo: {e:#}"))?;
    Ok(Plan {
        name: handle.name().unwrap_or_else(|| "Torrent".into()),
        file_index: chosen.index,
        file_name: chosen.name,
        size: chosen.size,
        mime: chosen.mime,
        videos,
    })
}

fn video_candidates(files: impl IntoIterator<Item = (usize, String, u64)>) -> Vec<Candidate> {
    files
        .into_iter()
        .filter_map(|(index, name, size)| {
            let extension = name
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
                .unwrap_or_default();
            let mime = mime_for_extension(&extension)?;
            (size > 0).then_some(Candidate {
                index,
                name,
                size,
                mime,
            })
        })
        .collect()
}

fn is_auxiliary_video(name: &str) -> bool {
    let basename = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let tokens: Vec<String> = basename
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect();
    [
        "sample",
        "trailer",
        "teaser",
        "extra",
        "extras",
        "featurette",
        "featurettes",
    ]
    .iter()
    .any(|word| tokens.iter().any(|token| token == word))
        || tokens.windows(2).any(|pair| pair == ["behind", "scenes"])
}

fn pick_video(candidates: &[Candidate], requested: Option<usize>) -> Result<Candidate, String> {
    match requested {
        Some(index) => candidates
            .iter()
            .find(|candidate| candidate.index == index)
            .cloned()
            .ok_or_else(|| "o índice informado não corresponde a um arquivo de vídeo".to_string()),
        None => candidates
            .iter()
            .max_by_key(|candidate| (!is_auxiliary_video(&candidate.name), candidate.size))
            .cloned()
            .ok_or_else(|| "este torrent não contém um formato de vídeo compatível".to_string()),
    }
}

async fn stream_file(
    State(state): State<HttpState>,
    AxumPath(token): AxumPath<String>,
    headers: HeaderMap,
) -> Response<Body> {
    let Some(file) = state.files.read().await.get(&token).cloned() else {
        return response_error(StatusCode::NOT_FOUND, "stream não encontrado");
    };
    if file.size == 0 {
        return media_response(StatusCode::OK, file.mime_type, 0, None, Body::empty());
    }
    let Ok((start, end, partial)) = parse_range(headers.get(header::RANGE), file.size) else {
        return range_not_satisfiable(file.size);
    };
    let mut stream = match file.torrent.clone().stream(file.file_index).await {
        Ok(stream) => stream,
        Err(error) => {
            return response_error(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("stream torrent: {error:#}"),
            )
        }
    };
    if let Err(error) = stream.seek(std::io::SeekFrom::Start(start)).await {
        return response_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("seek torrent: {error}"),
        );
    }
    let length = end - start + 1;
    let body = Body::from_stream(ReaderStream::new(stream.take(length)));
    media_response(
        if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        },
        file.mime_type,
        length,
        partial.then_some((start, end, file.size)),
        body,
    )
}

async fn head_stream_file(
    State(state): State<HttpState>,
    AxumPath(token): AxumPath<String>,
    headers: HeaderMap,
) -> Response<Body> {
    let Some(file) = state.files.read().await.get(&token).cloned() else {
        return response_error(StatusCode::NOT_FOUND, "stream não encontrado");
    };
    if file.size == 0 {
        return media_response(StatusCode::OK, file.mime_type, 0, None, Body::empty());
    }
    let Ok((start, end, partial)) = parse_range(headers.get(header::RANGE), file.size) else {
        return range_not_satisfiable(file.size);
    };
    media_response(
        if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        },
        file.mime_type,
        end - start + 1,
        partial.then_some((start, end, file.size)),
        Body::empty(),
    )
}

fn media_response(
    status: StatusCode,
    mime: &str,
    length: u64,
    range: Option<(u64, u64, u64)>,
    body: Body,
) -> Response<Body> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, length.to_string())
        .header(header::ACCEPT_RANGES, "bytes");
    if let Some((start, end, size)) = range {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
    }
    builder
        .body(body)
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn range_not_satisfiable(size: u64) -> Response<Body> {
    Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(header::CONTENT_RANGE, format!("bytes */{size}"))
        .header(header::ACCEPT_RANGES, "bytes")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Returns start, inclusive end, and whether the client requested a partial response.
fn parse_range(value: Option<&axum::http::HeaderValue>, size: u64) -> Result<(u64, u64, bool), ()> {
    if size == 0 {
        return Err(());
    }
    let Some(value) = value else {
        return Ok((0, size - 1, false));
    };
    let range = value
        .to_str()
        .map_err(|_| ())?
        .strip_prefix("bytes=")
        .ok_or(())?;
    if range.contains(',') {
        return Err(());
    }
    let (start, end) = range.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix: u64 = end.parse().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        return Ok((size.saturating_sub(suffix), size - 1, true));
    }
    let start: u64 = start.parse().map_err(|_| ())?;
    let requested_end = if end.is_empty() {
        size - 1
    } else {
        end.parse().map_err(|_| ())?
    };
    if start >= size || requested_end < start {
        return Err(());
    }
    let bounded_end = requested_end
        .min(size - 1)
        .min(start.saturating_add(MAX_RANGE_BYTES - 1));
    Ok((start, bounded_end, true))
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
        "mpeg" | "mpg" | "m2v" => Some("video/mpeg"),
        "ts" | "m2ts" => Some("video/mp2t"),
        "ogv" => Some("video/ogg"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(index: usize, size: u64, name: &str) -> Candidate {
        let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
        Candidate {
            index,
            name: name.into(),
            size,
            mime: mime_for_extension(extension).unwrap_or("video/mp4"),
        }
    }

    #[test]
    fn byte_ranges_are_valid_and_bounded() {
        assert_eq!(parse_range(None, 100), Ok((0, 99, false)));
        let exact = axum::http::HeaderValue::from_static("bytes=10-19");
        assert_eq!(parse_range(Some(&exact), 100), Ok((10, 19, true)));
        let suffix = axum::http::HeaderValue::from_static("bytes=-10");
        assert_eq!(parse_range(Some(&suffix), 100), Ok((90, 99, true)));
        let open = axum::http::HeaderValue::from_static("bytes=50-");
        assert_eq!(parse_range(Some(&open), 100), Ok((50, 99, true)));
        let huge = axum::http::HeaderValue::from_static("bytes=0-99999999");
        assert_eq!(
            parse_range(Some(&huge), MAX_RANGE_BYTES * 3),
            Ok((0, MAX_RANGE_BYTES - 1, true))
        );
        let invalid = axum::http::HeaderValue::from_static("bytes=100-");
        assert!(parse_range(Some(&invalid), 100).is_err());
        assert!(parse_range(None, 0).is_err());
    }

    #[test]
    fn recognizes_supported_video_types_and_ignores_others() {
        assert_eq!(mime_for_extension("mp4"), Some("video/mp4"));
        assert_eq!(mime_for_extension("mkv"), Some("video/x-matroska"));
        assert_eq!(mime_for_extension("m2ts"), Some("video/mp2t"));
        assert_eq!(mime_for_extension("txt"), None);
        let found = video_candidates(vec![
            (0, "readme.txt".into(), 20),
            (1, "empty.mp4".into(), 0),
            (2, "film.MKV".into(), 30),
        ]);
        assert_eq!(found.iter().map(|c| c.index).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn trailer_detection_only_checks_video_basename() {
        assert!(is_auxiliary_video("Show/trailer-cut.mp4"));
        assert!(!is_auxiliary_video("Extras/Series.S01E01.mkv"));
        let files = [
            candidate(0, 10, "sample.mp4"),
            candidate(1, 200, "trailer.mp4"),
            candidate(2, 100, "Movie.mkv"),
        ];
        assert_eq!(pick_video(&files, None).map(|c| c.index), Ok(2));
    }

    #[test]
    fn requested_index_must_be_a_video() {
        let files = [candidate(0, 10, "a.mp4"), candidate(2, 50, "b.mp4")];
        assert_eq!(pick_video(&files, Some(2)).map(|c| c.index), Ok(2));
        assert!(pick_video(&files, Some(1)).is_err());
        assert!(pick_video(&[], None).is_err());
    }
}
