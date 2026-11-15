//! Comandos chamados pela interface web com `invoke`.

use futures::future::join_all;
use serde_json::{json, Value};
use tauri::State;

use crate::{
    addon::{self, CatalogDef, Installed},
    store, AppState,
};

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
    let manifest = addon::fetch_manifest(&state.http, &url).await?;
    let installed = Installed {
        transport_url: url,
        manifest,
    };

    let mut list = state.addons.write().await;
    list.retain(|a| a.transport_url != installed.transport_url && a.manifest.id != installed.manifest.id);
    list.push(installed.clone());
    store::save(&state.store_path, &list).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "name": installed.manifest.name }))
}

#[tauri::command]
pub async fn remove_addon(state: State<'_, AppState>, url: String) -> Result<Value, String> {
    let mut list = state.addons.write().await;
    list.retain(|a| a.transport_url != url);
    store::save(&state.store_path, &list).map_err(|e| e.to_string())?;
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
    let rows = run_catalogs(&state.http, jobs, String::new()).await;
    Ok(json!({ "rows": rows }))
}

/// Busca em todos os catálogos que aceitam o parâmetro "search".
#[tauri::command]
pub async fn search(state: State<'_, AppState>, q: String) -> Result<Value, String> {
    let needle = q.trim().to_string();
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
    let rows = run_catalogs(&state.http, jobs, extra).await;
    Ok(json!({ "rows": rows }))
}

async fn run_catalogs(http: &reqwest::Client, jobs: Vec<(Installed, CatalogDef)>, extra: String) -> Vec<Value> {
    join_all(jobs.into_iter().map(|(a, c)| {
        let http = http.clone();
        let extra = extra.clone();
        async move {
            let (metas, error) = match addon::catalog(&http, &a, &c.kind, &c.id, &extra).await {
                Ok(m) => (m, None),
                Err(e) => (Vec::new(), Some(e)),
            };
            json!({
                "title": c.name.clone().unwrap_or_else(|| c.id.clone()),
                "addon": a.manifest.name,
                "type": c.kind,
                "catalogId": c.id,
                "metas": metas,
                "error": error,
            })
        }
    }))
    .await
}

/// Metadados: pergunta ao primeiro addon que responde ao recurso "meta" para este id.
#[tauri::command]
pub async fn meta(state: State<'_, AppState>, kind: String, id: String) -> Result<Value, String> {
    let addons: Vec<Installed> = state.addons.read().await.clone();
    for a in addons.iter().filter(|a| a.supports("meta", &kind, &id)) {
        if let Ok(Some(m)) = addon::meta(&state.http, a, &kind, &id).await {
            return Ok(json!({ "meta": m, "addon": a.manifest.name }));
        }
    }
    Ok(json!({ "meta": null }))
}

/// Fontes: junta as respostas de todos os addons que respondem ao recurso "stream".
#[tauri::command]
pub async fn streams(state: State<'_, AppState>, kind: String, id: String) -> Result<Value, String> {
    let addons: Vec<Installed> = state.addons.read().await.clone();
    let jobs = addons
        .into_iter()
        .filter(|a| a.supports("stream", &kind, &id))
        .map(|a| {
            let http = state.http.clone();
            let kind = kind.clone();
            let id = id.clone();
            async move {
                let list = addon::streams(&http, &a, &kind, &id).await.unwrap_or_default();
                list.into_iter()
                    .map(|mut s| {
                        if let Some(o) = s.as_object_mut() {
                            o.insert("addonName".into(), json!(a.manifest.name));
                        }
                        s
                    })
                    .collect::<Vec<Value>>()
            }
        });
    let all: Vec<Value> = join_all(jobs).await.into_iter().flatten().collect();
    Ok(json!({ "streams": all }))
}

/// Legenda: baixa o arquivo e devolve em VTT, que o player do navegador exige.
#[tauri::command]
pub async fn subtitle(state: State<'_, AppState>, url: String) -> Result<String, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("URL de legenda inválida".into());
    }
    let text = state
        .http
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(srt_to_vtt(&text))
}

fn srt_to_vtt(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with("WEBVTT") {
        return text.to_string();
    }
    let body = text
        .lines()
        .map(|l| if l.contains("-->") { l.replace(',', ".") } else { l.to_string() })
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
        assert_eq!(normalize_url("https://a.example/").unwrap(), "https://a.example/manifest.json");
        assert!(normalize_url("ftp://x").is_err());
    }

    #[test]
    fn converts_srt_to_vtt() {
        let vtt = srt_to_vtt("1\n00:00:01,000 --> 00:00:02,500\nOlá\n");
        assert!(vtt.starts_with("WEBVTT"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:02.500"));
    }
}
