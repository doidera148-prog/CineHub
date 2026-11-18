//! Comandos chamados pela interface web com `invoke`.

use std::{future::Future, time::Duration};

use futures::future::join_all;
use futures::stream::{self, StreamExt};
use serde_json::{json, Value};
use tauri::{Manager, State};

use crate::{
    addon::{self, CatalogDef, Installed},
    store, AppState,
};

/// Tamanho máximo de uma legenda baixada (2 MiB).
const MAX_SUBTITLE: usize = 2 * 1024 * 1024;

/// Tempo máximo para um addon responder. Um addon lento não segura a tela indefinidamente.
const ADDON_TIMEOUT: Duration = Duration::from_secs(15);

/// Aplica o tempo máximo a uma chamada a addon.
async fn bounded<T>(fut: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::time::timeout(ADDON_TIMEOUT, fut)
        .await
        .unwrap_or_else(|_| Err("o addon demorou demais para responder".into()))
}

#[tauri::command]
pub async fn list_addons(state: State<'_, AppState>) -> Result<Value, String> {
    let list = state.addons.read().await;
    Ok(Value::Array(
        list.iter()
            .map(|a| json!({ "transportUrl": a.transport_url, "manifest": a.manifest }))
            .collect(),
    ))
}

#[tauri::command]
pub async fn install_addon(state: State<'_, AppState>, url: String) -> Result<Value, String> {
    let url = normalize_url(&url)?;
    // Só endereços públicos: um addon não pode apontar o app para a rede interna.
    if !addon::is_safe_url(&url).await {
        return Err(
            "endereço não permitido: informe um addon em endereço público (não local nem da rede interna)"
                .into(),
        );
    }
    let manifest = addon::fetch_manifest(&state.http, &url).await?;
    let installed = Installed {
        transport_url: url,
        manifest,
    };

    // Uma gravação por vez, para que a última a salvar sempre tenha a lista mais recente.
    let _save = state.save_lock.lock().await;
    let snapshot = {
        let mut list = state.addons.write().await;
        // Reinstalar o mesmo endereço atualiza o addon. Outro endereço com o mesmo id não
        // substitui o existente sem aviso: ele é recusado.
        if let Some(other) = list.iter().find(|a| {
            a.manifest.id == installed.manifest.id && a.transport_url != installed.transport_url
        }) {
            return Err(format!(
                "já existe um addon com este id ({}). Remova-o antes de instalar outro endereço.",
                other.manifest.name
            ));
        }
        list.retain(|a| a.transport_url != installed.transport_url);
        list.push(installed.clone());
        list.clone()
    };
    store::save(&state.store_path, &snapshot).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "name": installed.manifest.name }))
}

#[tauri::command]
pub async fn remove_addon(state: State<'_, AppState>, url: String) -> Result<Value, String> {
    let _save = state.save_lock.lock().await;
    let snapshot = {
        let mut list = state.addons.write().await;
        list.retain(|a| a.transport_url != url);
        list.clone()
    };
    store::save(&state.store_path, &snapshot).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

/// Página inicial: um trilho por catálogo de cada addon, para o tipo pedido.
#[tauri::command]
pub async fn home(state: State<'_, AppState>, kind: String) -> Result<Value, String> {
    let addons: Vec<Installed> = state.addons.read().await.clone();
    let mut jobs = Vec::new();
    for a in &addons {
        for c in &a.manifest.catalogs {
            if catalog_matches_kind(a, c, &kind) && !c.requires_extra() {
                jobs.push((a.clone(), c.clone()));
            }
        }
    }
    let rows = run_catalogs(&state, jobs, String::new()).await;
    Ok(json!({ "rows": rows }))
}

/// Addons de anime frequentemente publicam seus catálogos usando o tipo Stremio
/// "series". Na aba Anime, inclui esses catálogos quando o próprio addon ou catálogo
/// se identifica explicitamente como anime, mantendo a rota original "series".
fn catalog_matches_kind(addon: &Installed, catalog: &CatalogDef, requested: &str) -> bool {
    if catalog.kind == requested {
        return true;
    }
    if requested != "anime" || !matches!(catalog.kind.as_str(), "series" | "movie") {
        return false;
    }
    let addon_name = addon.manifest.name.to_ascii_lowercase();
    let catalog_name = catalog.name.as_deref().unwrap_or_default().to_ascii_lowercase();
    let catalog_id = catalog.id.to_ascii_lowercase();
    addon_name.contains("anime")
        || catalog_name.contains("anime")
        || catalog_id.contains("anime")
}

/// Tamanho máximo do texto de busca, em caracteres.
const MAX_QUERY_CHARS: usize = 200;

/// Busca em todos os catálogos que aceitam o parâmetro "search".
#[tauri::command]
pub async fn search(state: State<'_, AppState>, q: String) -> Result<Value, String> {
    let needle: String = q.trim().chars().take(MAX_QUERY_CHARS).collect();
    if needle.is_empty() {
        return Ok(json!({ "rows": [] }));
    }
    let addons: Vec<Installed> = state.addons.read().await.clone();
    let mut jobs = Vec::new();
    for a in &addons {
        for c in &a.manifest.catalogs {
            if c.supports_search() && matches!(c.kind.as_str(), "movie" | "series" | "anime") {
                jobs.push((a.clone(), c.clone()));
            }
        }
    }
    let extra = format!("search={}", addon::enc(&needle));
    let rows = run_catalogs(&state, jobs, extra).await;
    Ok(json!({ "rows": rows }))
}

/// Máximo de catálogos consultados ao mesmo tempo.
const MAX_PARALLEL: usize = 6;

async fn run_catalogs(
    state: &AppState,
    jobs: Vec<(Installed, CatalogDef)>,
    extra: String,
) -> Vec<Value> {
    stream::iter(jobs.into_iter().map(|(a, c)| {
        let http = state.http.clone();
        let cache = state.cache.clone();
        let extra = extra.clone();
        async move {
            let (metas, error) =
                match bounded(addon::catalog(&http, &cache, &a, &c.kind, &c.id, &extra)).await {
                    Ok(m) => (m, None),
                    Err(e) => (Vec::new(), Some(format!("{}: {e}", a.manifest.name))),
                };
            json!({
                "title": c.name.clone().unwrap_or_else(|| c.id.clone()),
                "addon": a.manifest.name,
                "transportUrl": a.transport_url,
                "type": c.kind,
                "catalogId": c.id,
                "metas": metas,
                "error": error,
            })
        }
    }))
    .buffered(MAX_PARALLEL)
    .collect()
    .await
}

/// Próxima página de um catálogo (paginação com `skip`), usada no botão "Carregar mais".
#[tauri::command]
pub async fn catalog_page(
    state: State<'_, AppState>,
    transport_url: String,
    kind: String,
    catalog_id: String,
    skip: u32,
) -> Result<Value, String> {
    let addons = state.addons.read().await;
    let a = addons
        .iter()
        .find(|a| a.transport_url == transport_url)
        .ok_or("addon não está instalado")?
        .clone();
    drop(addons);
    let extra = if skip > 0 {
        format!("skip={skip}")
    } else {
        String::new()
    };
    let metas = bounded(addon::catalog(
        &state.http,
        &state.cache,
        &a,
        &kind,
        &catalog_id,
        &extra,
    ))
    .await?;
    Ok(json!({ "metas": metas }))
}

/// Metadados: consulta em paralelo os addons com o recurso "meta" e mescla as respostas.
/// O primeiro addon tem prioridade; campos vazios são preenchidos com os dos demais.
#[tauri::command]
pub async fn meta(state: State<'_, AppState>, kind: String, id: String) -> Result<Value, String> {
    let addons: Vec<Installed> = state.addons.read().await.clone();
    let jobs = addons
        .iter()
        .filter(|a| a.supports("meta", &kind, &id))
        .map(|a| {
            let http = state.http.clone();
            let cache = state.cache.clone();
            let (kind, id, a) = (kind.clone(), id.clone(), a.clone());
            async move {
                let result = bounded(addon::meta(&http, &cache, &a, &kind, &id)).await;
                (a.manifest.name.clone(), result)
            }
        });

    let mut merged: Option<Value> = None;
    let mut first_name = String::new();
    for (name, result) in join_all(jobs).await {
        let Ok(Some(m)) = result else { continue };
        match merged.as_mut() {
            None => {
                first_name = name;
                merged = Some(m);
            }
            Some(base) => merge_missing(base, &m),
        }
    }
    match merged {
        Some(m) => Ok(json!({ "meta": m, "addon": first_name })),
        None => Ok(json!({ "meta": null })),
    }
}

/// Copia para `base` os campos que estão vazios nele e presentes em `extra`.
fn merge_missing(base: &mut Value, extra: &Value) {
    let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) else {
        return;
    };
    for (k, v) in e {
        let empty = match b.get(k) {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.is_empty(),
            Some(Value::Array(a)) => a.is_empty(),
            _ => false,
        };
        if empty && !v.is_null() {
            b.insert(k.clone(), v.clone());
        }
    }
}

/// Fontes: junta as fontes de todos os addons com o recurso "stream", e as legendas
/// de addons com o recurso "subtitles". Cada fonte recebe um aviso quando o app não consegue tocá-la.
#[tauri::command]
pub async fn streams(
    state: State<'_, AppState>,
    kind: String,
    id: String,
) -> Result<Value, String> {
    let addons: Vec<Installed> = state.addons.read().await.clone();

    let stream_jobs = addons
        .iter()
        .filter(|a| a.supports("stream", &kind, &id))
        .map(|a| {
            let http = state.http.clone();
            let cache = state.cache.clone();
            let (kind, id, a) = (kind.clone(), id.clone(), a.clone());
            async move {
                match bounded(addon::streams(&http, &cache, &a, &kind, &id)).await {
                    Ok(list) => (
                        list.into_iter()
                            .map(|mut s| {
                                annotate(&mut s, &a.manifest.name);
                                s
                            })
                            .collect::<Vec<Value>>(),
                        None,
                    ),
                    Err(e) => (
                        Vec::new(),
                        Some(json!({ "addon": a.manifest.name, "error": e })),
                    ),
                }
            }
        });
    let mut streams: Vec<Value> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    for (list, error) in join_all(stream_jobs).await {
        streams.extend(list);
        errors.extend(error);
    }

    let sub_jobs = addons
        .iter()
        .filter(|a| a.supports("subtitles", &kind, &id))
        .map(|a| {
            let http = state.http.clone();
            let cache = state.cache.clone();
            let (kind, id, a) = (kind.clone(), id.clone(), a.clone());
            async move {
                bounded(addon::subtitles(&http, &cache, &a, &kind, &id))
                    .await
                    .unwrap_or_default()
            }
        });
    let subtitles: Vec<Value> = join_all(sub_jobs)
        .await
        .into_iter()
        .flatten()
        .filter(|s| s.get("url").and_then(Value::as_str).is_some())
        .collect();

    Ok(json!({ "streams": streams, "subtitles": subtitles, "errors": errors }))
}

/// Marca o que o app consegue fazer com cada fonte e adiciona o nome do addon.
fn annotate(s: &mut Value, addon_name: &str) {
    let hints = s.get("behaviorHints").cloned().unwrap_or(Value::Null);
    let needs_headers = hints.get("proxyHeaders").is_some_and(|v| !v.is_null());
    let not_web_ready = hints
        .get("notWebReady")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let url = s
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let external = s
        .get("externalUrl")
        .and_then(Value::as_str)
        .map(str::to_string);
    let yt_id = s.get("ytId").and_then(Value::as_str).map(str::to_string);
    let has_info_hash = s.get("infoHash").is_some_and(|v| !v.is_null());
    let has_magnet = s
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|u| u.starts_with("magnet:?"));

    // Só links web: qualquer outro esquema (javascript:, file:, intent:…) é descartado.
    let external = external.filter(|u| is_http_url(u)).or_else(|| {
        yt_id
            .as_ref()
            .filter(|id| {
                id.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .map(|id| format!("https://www.youtube.com/watch?v={id}"))
    });

    let note: Option<&str> = if !url.is_empty() && !is_app_url(&url) {
        Some("link não suportado pelo app")
    } else if !url.is_empty() {
        if needs_headers {
            Some("exige cabeçalhos especiais; pode não tocar")
        } else if not_web_ready || url.to_lowercase().contains(".mkv") {
            Some("formato pode não tocar no player")
        } else {
            None
        }
    } else if external.is_some() {
        Some("abre no navegador")
    } else if has_info_hash || has_magnet {
        Some("torrent: será resolvido pelo motor local")
    } else {
        Some("sem link reproduzível")
    };

    if let Some(o) = s.as_object_mut() {
        // Um link com esquema desconhecido (file:, intent:, javascript:…) é descartado.
        if !url.is_empty() && !is_app_url(&url) {
            o.remove("url");
        }
        o.insert("addonName".into(), json!(addon_name));
        o.insert("note".into(), note.map_or(Value::Null, Value::from));
        o.insert(
            "externalUrl".into(),
            external.map_or(Value::Null, Value::from),
        );
    }
}

/// Esquemas que o app reproduz: http(s) e magnet (este último vai para o motor torrent).
/// Aceita somente URLs HTTP(S) sintaticamente válidas, com host, ou magnets válidos.
/// Não bloqueia IPs privados aqui: fontes locais podem ser intencionais e o torrent usa loopback.
fn is_http_url(raw: &str) -> bool {
    let raw = raw.trim();
    let Some((scheme, remainder)) = raw.split_once("://") else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    // Url::parse normaliza barras extras (https:///host vira https://host).
    // Exigir autoridade explícita evita aceitar URLs malformadas após essa normalização.
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
}

/// Esquemas que o app reproduz: HTTP(S) com autoridade válida e magnet para o motor torrent.
fn is_app_url(raw: &str) -> bool {
    if raw.trim().starts_with("magnet:?") {
        return reqwest::Url::parse(raw.trim()).is_ok_and(|url| url.scheme() == "magnet");
    }
    is_http_url(raw)
}

/// Legenda: baixa o arquivo (só URLs públicas, validando cada redirecionamento,
/// com limite de tamanho) e devolve em VTT.
#[tauri::command]
pub async fn subtitle(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let bytes = addon::fetch_public_capped(&state.sub_http, &url, MAX_SUBTITLE).await?;
    Ok(srt_to_vtt(&String::from_utf8_lossy(&bytes)))
}

/// Cria uma URL local temporária para uma fonte HTTP progressiva.
/// Torrent não passa por este comando; HLS é recusado até a reescrita integral das playlists.
#[tauri::command]
pub async fn media_proxy_url(
    state: State<'_, AppState>,
    url: String,
    behavior_hints: Option<Value>,
    mime_type: Option<String>,
) -> Result<String, String> {
    let proxy = state
        .media_proxy
        .get_or_init(|| async { crate::media_proxy::MediaProxy::start().await })
        .await
        .as_ref()
        .map_err(Clone::clone)?;
    proxy
        .register(&url, behavior_hints.as_ref(), mime_type.as_deref())
        .await
}

/// Retorna estatísticas do torrent ativo, sem iniciar uma sessão nova.
#[tauri::command]
pub async fn torrent_stats(
    state: State<'_, AppState>,
) -> Result<crate::torrent::TorrentNetworkStats, String> {
    let engine = state
        .torrent
        .as_ref()
        .ok_or("o motor de torrent não está disponível neste aparelho")?;
    Ok(engine.network_stats().await)
}

/// Abre um torrent (magnet ou bytes de .torrent) e devolve a URL local do arquivo de vídeo.
/// Recebe exatamente um dos dois. Só um torrent fica ativo por vez.
#[tauri::command]
pub async fn torrent_open(
    state: State<'_, AppState>,
    magnet: Option<String>,
    torrent_bytes: Option<Vec<u8>>,
    file_index: Option<usize>,
) -> Result<Value, String> {
    let engine = state
        .torrent
        .as_ref()
        .ok_or("o motor de torrent não está disponível neste aparelho")?;
    match (magnet, torrent_bytes) {
        (Some(m), None) => {
            let p = engine.add_magnet(m, file_index).await?;
            Ok(json!({
                "token": p.token,
                "streamUrl": p.stream_url,
                "name": p.name,
                "fileName": p.file_name,
                "mimeType": p.mime_type,
                "sizeBytes": p.size_bytes,
                "videos": p.videos,
            }))
        }
        (None, Some(bytes)) => {
            let p = engine.add_torrent_file(bytes, file_index).await?;
            Ok(json!({
                "token": p.token,
                "streamUrl": p.stream_url,
                "name": p.name,
                "fileName": p.file_name,
                "mimeType": p.mime_type,
                "sizeBytes": p.size_bytes,
                "videos": p.videos,
            }))
        }
        _ => Err("forneça exatamente um magnet ou um arquivo .torrent".into()),
    }
}

/// Troca o arquivo de vídeo de um torrent já aberto (episódio ou idioma escolhido pelo usuário).
#[tauri::command]
pub async fn torrent_select_file(
    state: State<'_, AppState>,
    token: String,
    file_index: usize,
) -> Result<Value, String> {
    let engine = state
        .torrent
        .as_ref()
        .ok_or("o motor de torrent não está disponível neste aparelho")?;
    let p = engine.select_file(&token, file_index).await?;
    Ok(json!({
        "token": p.token,
        "streamUrl": p.stream_url,
        "name": p.name,
        "fileName": p.file_name,
        "mimeType": p.mime_type,
        "sizeBytes": p.size_bytes,
        "videos": p.videos,
    }))
}

/// Encerra um torrent e invalida o token de reprodução local.
#[tauri::command]
pub async fn torrent_close(state: State<'_, AppState>, token: String) -> Result<bool, String> {
    let engine = state
        .torrent
        .as_ref()
        .ok_or("o motor de torrent não está disponível neste aparelho")?;
    engine.close(&token).await
}

/// Tamanho máximo de um backup da biblioteca (2 MiB).
const MAX_EXPORT_BYTES: usize = 2 * 1024 * 1024;

/// Salva o backup da biblioteca na pasta de dados do app e devolve o caminho do arquivo.
/// (No Android essa pasta é privada do app; o caminho é mostrado ao usuário.)
#[tauri::command]
pub async fn save_export(app: tauri::AppHandle, content: String) -> Result<String, String> {
    if content.len() > MAX_EXPORT_BYTES {
        return Err("backup grande demais para salvar".into());
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("pasta de dados: {e}"))?
        .join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("pasta de exportação: {e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("cinehub-dados-{stamp}.json"));
    std::fs::write(&path, content).map_err(|e| format!("não foi possível salvar: {e}"))?;
    Ok(path.display().to_string())
}

/// Grava o backup ou o diagnóstico no caminho escolhido pelo usuário no diálogo "Salvar como" (desktop).
/// Só aceita arquivos .json (backup) ou .txt (diagnóstico): não é um gravador genérico de arquivos.
#[tauri::command]
pub async fn write_export(path: String, content: String) -> Result<(), String> {
    if content.len() > MAX_EXPORT_BYTES {
        return Err("backup grande demais para salvar".into());
    }
    let target = std::path::Path::new(&path);
    let allowed = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("txt"));
    if !allowed {
        return Err("o arquivo precisa ser .json (backup) ou .txt (diagnóstico)".into());
    }
    std::fs::write(target, content).map_err(|e| format!("não foi possível salvar: {e}"))
}

pub(crate) fn srt_to_vtt(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with("WEBVTT") {
        return text.to_string();
    }
    let body = text
        .lines()
        .map(|l| {
            if l.contains("-->") {
                l.replace(',', ".")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("WEBVTT\n\n{body}\n")
}

/// Aceita a URL do manifest, "stremio://..." ou a base do addon (acrescenta /manifest.json).
fn normalize_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2048 {
        return Err("informe um endereço válido (máximo de 2048 caracteres)".into());
    }
    let normalized = raw
        .strip_prefix("stremio://")
        .map(|rest| format!("https://{rest}"))
        .unwrap_or_else(|| raw.to_string());
    let mut url =
        reqwest::Url::parse(&normalized).map_err(|_| "endereço do addon inválido".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("use um endereço http(s) público, sem usuário ou senha na URL".into());
    }
    // Acrescenta o caminho antes da query: /manifest.json?key=x precisa manter o parâmetro.
    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/manifest.json") && path != "manifest.json" {
        let next = if path.is_empty() {
            "/manifest.json".to_string()
        } else {
            format!("{path}/manifest.json")
        };
        url.set_path(&next);
    }
    // Fragmentos nunca são enviados ao servidor e só criam URLs duplicadas no armazenamento.
    url.set_fragment(None);
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anime_page_includes_series_catalogs_from_anime_addons() {
        let addon = Installed {
            transport_url: "https://anime.example/manifest.json".into(),
            manifest: addon::Manifest {
                id: "anime.catalogs".into(),
                version: "1".into(),
                name: "Anime Catalogs".into(),
                description: None,
                logo: None,
                types: vec!["series".into()],
                id_prefixes: None,
                resources: vec![],
                catalogs: vec![],
            },
        };
        let catalog = CatalogDef {
            kind: "series".into(),
            id: "popular".into(),
            name: Some("Mais populares".into()),
            extra: vec![],
        };
        assert!(catalog_matches_kind(&addon, &catalog, "anime"));
        assert!(!catalog_matches_kind(&addon, &catalog, "movie"));
        assert!(catalog_matches_kind(&addon, &catalog, "series"));
    }

    #[test]
    fn anime_page_does_not_include_unrelated_series_catalogs() {
        let addon = Installed {
            transport_url: "https://catalog.example/manifest.json".into(),
            manifest: addon::Manifest {
                id: "catalog.series".into(),
                version: "1".into(),
                name: "Series Catalog".into(),
                description: None,
                logo: None,
                types: vec!["series".into()],
                id_prefixes: None,
                resources: vec![],
                catalogs: vec![],
            },
        };
        let catalog = CatalogDef {
            kind: "series".into(),
            id: "popular".into(),
            name: Some("Mais populares".into()),
            extra: vec![],
        };
        assert!(!catalog_matches_kind(&addon, &catalog, "anime"));
    }

    #[test]
    fn normalizes_install_urls() {
        assert_eq!(
            normalize_url("stremio://addon.example/manifest.json").unwrap(),
            "https://addon.example/manifest.json"
        );
        assert_eq!(
            normalize_url("https://a.example/").unwrap(),
            "https://a.example/manifest.json"
        );
        assert!(normalize_url("ftp://x").is_err());
        assert_eq!(
            normalize_url("https://a.example?token=abc").unwrap(),
            "https://a.example/manifest.json?token=abc"
        );
        assert_eq!(
            normalize_url("https://a.example/manifest.json?token=abc").unwrap(),
            "https://a.example/manifest.json?token=abc"
        );
        assert_eq!(
            normalize_url("https://a.example/addon/manifest.json#ignored").unwrap(),
            "https://a.example/addon/manifest.json"
        );
        assert!(normalize_url("https://user:pass@a.example").is_err());
        assert!(normalize_url("https://a.example/manifest.json?token=abc").is_ok());
        assert!(normalize_url(&"x".repeat(2049)).is_err());
    }

    #[test]
    fn literal_checks_cover_mapped_ipv6() {
        assert!(!addon::literal_host_is_public(
            "http://[::ffff:127.0.0.1]/x"
        ));
        assert!(!addon::literal_host_is_public(
            "http://[::ffff:192.168.0.1]/x"
        ));
        assert!(!addon::literal_host_is_public("http://localhost/x"));
        assert!(!addon::literal_host_is_public("http://127.0.0.1/x"));
        assert!(addon::literal_host_is_public("http://93.184.216.34/x"));
    }

    #[test]
    fn converts_srt_to_vtt() {
        let vtt = srt_to_vtt("1\n00:00:01,000 --> 00:00:02,500\nOlá\n");
        assert!(vtt.starts_with("WEBVTT"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:02.500"));
    }

    #[test]
    fn external_links_must_be_web_urls() {
        let mut bad = json!({ "externalUrl": "javascript:alert(1)" });
        annotate(&mut bad, "A");
        assert!(bad["externalUrl"].is_null());

        let mut good = json!({ "externalUrl": "https://x.example/a" });
        annotate(&mut good, "A");
        assert_eq!(good["externalUrl"], "https://x.example/a");
    }

    #[test]
    fn stream_urls_require_a_valid_http_authority_but_keep_local_sources() {
        assert!(is_app_url("https://cdn.example/video.m3u8?token=a"));
        assert!(is_app_url("http://192.168.1.20:8080/video.mp4"));
        assert!(is_app_url("http://127.0.0.1:9000/stream/token"));
        assert!(is_app_url(
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789"
        ));
        assert!(!is_app_url("https:///missing-host/video.mp4"));
        assert!(!is_app_url("http://"));
        assert!(!is_app_url("file:///video.mp4"));
        assert!(!is_app_url("javascript:alert(1)"));
    }

    #[test]
    fn malformed_stream_urls_are_removed_without_rejecting_lan_urls() {
        let mut malformed = json!({ "url": "https:///missing-host/video.mp4" });
        annotate(&mut malformed, "A");
        assert!(malformed.get("url").is_none());

        let mut lan = json!({ "url": "http://192.168.1.20:8080/video.mp4" });
        annotate(&mut lan, "A");
        assert_eq!(lan["url"], "http://192.168.1.20:8080/video.mp4");
    }

    #[test]
    fn youtube_trailers_become_links_only_for_valid_ids() {
        let mut ok = json!({ "ytId": "abc-_1" });
        annotate(&mut ok, "A");
        assert_eq!(ok["externalUrl"], "https://www.youtube.com/watch?v=abc-_1");

        let mut bad = json!({ "ytId": "a/b" });
        annotate(&mut bad, "A");
        assert!(bad["externalUrl"].is_null());
    }

    #[test]
    fn merge_fills_only_empty_fields() {
        let mut base = json!({ "name": "A", "poster": "", "genres": [] });
        let extra = json!({ "name": "B", "poster": "p.jpg", "genres": ["x"] });
        merge_missing(&mut base, &extra);
        assert_eq!(base["name"], "A");
        assert_eq!(base["poster"], "p.jpg");
        assert_eq!(base["genres"][0], "x");
    }
}
