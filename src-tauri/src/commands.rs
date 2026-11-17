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
            if c.kind == kind && !c.requires_extra() {
                jobs.push((a.clone(), c.clone()));
            }
        }
    }
    let rows = run_catalogs(&state, jobs, String::new()).await;
    Ok(json!({ "rows": rows }))
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
            if c.supports_search() && (c.kind == "movie" || c.kind == "series") {
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
    let external = external
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .or_else(|| {
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
fn is_app_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://") || url.starts_with("magnet:?")
}

/// Legenda: baixa o arquivo (só URLs públicas, validando cada redirecionamento,
/// com limite de tamanho) e devolve em VTT.
#[tauri::command]
pub async fn subtitle(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let bytes = addon::fetch_public_capped(&state.sub_http, &url, MAX_SUBTITLE).await?;
    Ok(srt_to_vtt(&String::from_utf8_lossy(&bytes)))
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
                "streamUrl": p.stream_url,
                "name": p.name,
                "fileName": p.file_name,
                "mimeType": p.mime_type,
                "sizeBytes": p.size_bytes,
            }))
        }
        (None, Some(bytes)) => {
            let p = engine.add_torrent_file(bytes, file_index).await?;
            Ok(json!({
                "streamUrl": p.stream_url,
                "name": p.name,
                "fileName": p.file_name,
                "mimeType": p.mime_type,
                "sizeBytes": p.size_bytes,
            }))
        }
        _ => Err("forneça exatamente um magnet ou um arquivo .torrent".into()),
    }
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

/// Grava o backup no caminho escolhido pelo usuário no diálogo "Salvar como" (desktop).
/// Só aceita arquivos .json de backup: não é um gravador genérico de arquivos.
#[tauri::command]
pub async fn write_export(path: String, content: String) -> Result<(), String> {
    if content.len() > MAX_EXPORT_BYTES {
        return Err("backup grande demais para salvar".into());
    }
    let target = std::path::Path::new(&path);
    let is_json = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if !is_json {
        return Err("o backup precisa ser salvo como arquivo .json".into());
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
    let mut url = raw.trim().to_string();
    if let Some(rest) = url.strip_prefix("stremio://") {
        url = format!("https://{rest}");
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("A URL precisa começar com https://, http:// ou stremio://".into());
    }
    if !url.ends_with("manifest.json") {
        url = format!("{}/manifest.json", url.trim_end_matches('/'));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

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
