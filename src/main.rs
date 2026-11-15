//! Addon do Stremio que serve a sua biblioteca local de filmes e séries (pt-BR).
//!
//! Endpoints do protocolo de addons do Stremio:
//!   GET /manifest.json
//!   GET /catalog/{tipo}/{id}.json[/search=texto].json
//!   GET /meta/{tipo}/{id}.json
//!   GET /stream/{tipo}/{id}.json
//! Endpoints auxiliares, usados pelos links de stream:
//!   GET /media/{id}   (arquivo de vídeo, com suporte a Range)
//!   GET /subs/{id}    (legenda em português, quando existir)

mod library;

use std::{env, path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use library::Library;
use serde_json::{json, Value};
use tower::ServiceExt;
use tower_http::{cors::CorsLayer, services::ServeFile};

#[derive(Clone)]
struct AppState {
    lib: Arc<Library>,
    /// URL pública do addon, sem barra final (ex.: http://192.168.0.10:7000)
    base: String,
}

#[tokio::main]
async fn main() {
    let dir = PathBuf::from(env::var("MEDIA_DIR").unwrap_or_else(|_| "./media".into()));
    let port: u16 = env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7000);
    let base = env::var("BASE_URL")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{port}"))
        .trim_end_matches('/')
        .to_string();

    let lib = library::scan(&dir);
    eprintln!(
        "Biblioteca: {} filmes, {} séries, {} arquivos em {}",
        lib.movies.len(),
        lib.series.len(),
        lib.media.len(),
        dir.display()
    );
    eprintln!("Instale no Stremio: {base}/manifest.json");

    let state = AppState { lib: Arc::new(lib), base };

    let app = Router::new()
        .route("/manifest.json", get(manifest))
        .route("/catalog/:ty/*rest", get(catalog))
        .route("/meta/:ty/*rest", get(meta))
        .route("/stream/:ty/*rest", get(stream))
        .route("/media/:id", get(media))
        .route("/subs/:id", get(subs))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("porta indisponível");
    axum::serve(listener, app).await.expect("servidor encerrou com erro");
}

async fn manifest(State(st): State<AppState>) -> Json<Value> {
    let _ = &st;
    Json(json!({
        "id": "com.cinehub.biblioteca-local",
        "version": "0.1.0",
        "name": "Biblioteca Local",
        "description": "Filmes e séries da sua biblioteca local, com legendas em português quando disponíveis.",
        "resources": ["catalog", "meta", "stream"],
        "types": ["movie", "series"],
        "idPrefixes": ["local:"],
        "catalogs": [
            {
                "type": "movie",
                "id": "local-filmes",
                "name": "Filmes locais",
                "extra": [{ "name": "search", "isRequired": false }]
            },
            {
                "type": "series",
                "id": "local-series",
                "name": "Séries locais",
                "extra": [{ "name": "search", "isRequired": false }]
            }
        ],
        "behaviorHints": { "configurable": false, "configurationRequired": false }
    }))
}

/// /catalog/{tipo}/{rest} onde rest = "{id}.json" ou "{id}/search=texto.json"
async fn catalog(State(st): State<AppState>, Path((ty, rest)): Path<(String, String)>) -> Json<Value> {
    let rest = strip_json(&rest);
    let (id, extra) = rest.split_once('/').unwrap_or((rest, ""));
    let search = extra_value(extra, "search").map(|s| s.to_lowercase());
    let matches = |title: &str| search.as_ref().map_or(true, |q| title.to_lowercase().contains(q));

    let metas: Vec<Value> = match (ty.as_str(), id) {
        ("movie", "local-filmes") => st
            .lib
            .movies
            .iter()
            .filter(|m| matches(&m.title))
            .map(|m| {
                json!({
                    "id": m.id,
                    "type": "movie",
                    "name": m.title,
                    "releaseInfo": m.year.map(|y| y.to_string()),
                })
            })
            .collect(),
        ("series", "local-series") => st
            .lib
            .series
            .values()
            .filter(|s| matches(&s.title))
            .map(|s| json!({ "id": s.id, "type": "series", "name": s.title }))
            .collect(),
        _ => Vec::new(),
    };

    Json(json!({ "metas": metas }))
}

async fn meta(State(st): State<AppState>, Path((ty, rest)): Path<(String, String)>) -> Json<Value> {
    let id = strip_json(&rest);
    let meta = match ty.as_str() {
        "movie" => st.lib.movies.iter().find(|m| m.id == id).map(|m| {
            json!({
                "id": m.id,
                "type": "movie",
                "name": m.title,
                "releaseInfo": m.year.map(|y| y.to_string()),
                "description": "Arquivo da sua biblioteca local.",
            })
        }),
        "series" => st.lib.series.values().find(|s| s.id == id).map(|s| {
            let videos: Vec<Value> = s
                .episodes
                .iter()
                .map(|((season, ep), vid)| {
                    json!({
                        "id": vid,
                        "title": format!("Episódio {ep}"),
                        "season": season,
                        "episode": ep,
                    })
                })
                .collect();
            json!({
                "id": s.id,
                "type": "series",
                "name": s.title,
                "description": "Série da sua biblioteca local.",
                "videos": videos,
            })
        }),
        _ => None,
    };
    Json(json!({ "meta": meta }))
}

/// Para filmes o id é "local:m:..." e para episódios "local:s:...:T:E".
async fn stream(State(st): State<AppState>, Path((_ty, rest)): Path<(String, String)>) -> Json<Value> {
    let id = strip_json(&rest);
    if !st.lib.media.contains_key(id) {
        return Json(json!({ "streams": [] }));
    }

    let quality = st.lib.quality.get(id).cloned().unwrap_or_else(|| "Arquivo".to_string());
    let file = st
        .lib
        .media
        .get(id)
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let subtitles: Vec<Value> = if st.lib.subs.contains_key(id) {
        vec![json!({
            "id": format!("{id}:pt"),
            "url": format!("{}/subs/{}", st.base, enc(id)),
            "lang": "por",
        })]
    } else {
        Vec::new()
    };

    Json(json!({
        "streams": [{
            "name": "Biblioteca Local",
            "title": format!("{quality}\n{file}"),
            "url": format!("{}/media/{}", st.base, enc(id)),
            "subtitles": subtitles,
        }]
    }))
}

async fn media(State(st): State<AppState>, Path(id): Path<String>, req: Request) -> Response {
    match st.lib.media.get(&id) {
        Some(path) => serve_file(path.clone(), req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn subs(State(st): State<AppState>, Path(id): Path<String>, req: Request) -> Response {
    match st.lib.subs.get(&id) {
        Some(path) => serve_file(path.clone(), req).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Entrega o arquivo com suporte a cabeçalhos Range, necessário para o player avançar e voltar.
async fn serve_file(path: PathBuf, req: Request) -> Response {
    match ServeFile::new(path).oneshot(req).await {
        Ok(res) => res.map(Body::new),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn strip_json(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

fn extra_value(extra: &str, key: &str) -> Option<String> {
    extra.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| decode(v))
    })
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn enc(s: &str) -> String {
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
