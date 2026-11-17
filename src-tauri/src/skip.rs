//! Tempos de abertura, recapitulação e créditos para o botão "Pular" do player do navegador.
//!
//! As fontes são bases comunitárias, as mesmas que o dx.bi usa: TheIntroDB (filmes e séries,
//! pelo IMDb ou TMDB) e AniSkip (anime, pelo MAL, achado a partir do Kitsu). A consulta sai do
//! backend, e não da página, para funcionar igual no computador e no Android.

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::addon;

/// Tempo máximo para as fontes responderem. Sem resposta, o botão simplesmente não aparece.
const TIMEOUT: Duration = Duration::from_secs(12);
/// Tamanho máximo de uma resposta (as bases devolvem JSON pequeno).
const MAX_BODY: usize = 256 * 1024;

/// Um trecho a pular, em segundos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    #[serde(rename = "type")]
    pub kind: String,
    pub start: f64,
    pub end: f64,
}

/// O que consultar. Só entra número validado na URL.
#[derive(Debug, PartialEq)]
pub enum Lookup {
    /// TheIntroDB, com a query pronta (ex.: `imdb_id=tt…&season=1&episode=2`)
    IntroDb(String),
    /// AniSkip: número do Kitsu e número do episódio
    Kitsu { id: String, episode: u32 },
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Escolhe a consulta pelo id do título e pelo id do vídeo (o episódio).
/// Ids fora do formato esperado dão None: nada é consultado.
pub fn parse_target(kind: &str, id: &str, video_id: &str) -> Option<Lookup> {
    let parts: Vec<&str> = video_id.split(':').collect();
    if let Some(kitsu) = id.strip_prefix("kitsu:") {
        if !digits(kitsu) {
            return None;
        }
        let episode = parts
            .get(2)
            .and_then(|e| e.parse::<u32>().ok())
            .unwrap_or(1);
        return Some(Lookup::Kitsu {
            id: kitsu.to_string(),
            episode,
        });
    }
    let mut query = if id.strip_prefix("tt").is_some_and(digits) {
        format!("imdb_id={id}")
    } else {
        format!(
            "tmdb_id={}",
            id.strip_prefix("tmdb:").filter(|n| digits(n))?
        )
    };
    if kind != "movie" {
        // Episódios terminam em temporada:episódio ("tt0903747:1:2", "tmdb:1396:1:2")
        if parts.len() < 3 {
            return None;
        }
        let season = parts[parts.len() - 2].parse::<u32>().ok()?;
        let episode = parts[parts.len() - 1].parse::<u32>().ok()?;
        query.push_str(&format!("&season={season}&episode={episode}"));
    }
    Some(Lookup::IntroDb(query))
}

/// Trecho que está valendo no instante `t` (segundos). Ele some um pouco antes do fim.
pub fn active_segment(segments: &[Segment], t: f64) -> Option<&Segment> {
    segments.iter().find(|s| t >= s.start && t < s.end - 0.5)
}

/// Texto do botão para cada tipo de trecho.
pub fn label(kind: &str) -> &'static str {
    match kind {
        "intro" => "Pular abertura",
        "recap" => "Pular recapitulação",
        "preview" => "Pular prévia",
        "credits" => "Pular créditos",
        _ => "Pular",
    }
}

/// Guarda o trecho se o fim for conhecido e depois do início (com pelo menos 1 s).
fn keep(out: &mut Vec<Segment>, kind: &str, start: f64, end: f64) {
    if start.is_finite() && end.is_finite() && start >= 0.0 && end > start + 1.0 {
        out.push(Segment {
            kind: kind.to_string(),
            start,
            end,
        });
    }
}

/// Resposta do TheIntroDB (tempos em milissegundos). Trecho sem fim conhecido é descartado.
pub fn introdb_segments(v: &Value) -> Vec<Segment> {
    let mut out = Vec::new();
    for kind in ["intro", "recap", "credits", "preview"] {
        for seg in v.get(kind).and_then(Value::as_array).into_iter().flatten() {
            let start = seg.get("start_ms").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0;
            if let Some(end) = seg.get("end_ms").and_then(Value::as_f64) {
                keep(&mut out, kind, start, end / 1000.0);
            }
        }
    }
    out
}

/// Resposta do AniSkip (tempos em segundos).
pub fn aniskip_segments(v: &Value) -> Vec<Segment> {
    let mut out = Vec::new();
    for r in v
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = match r.get("skipType").and_then(Value::as_str) {
            Some("op" | "mixed-op") => "intro",
            Some("ed" | "mixed-ed") => "credits",
            Some("recap") => "recap",
            _ => continue,
        };
        let start = r.pointer("/interval/startTime").and_then(Value::as_f64);
        let end = r.pointer("/interval/endTime").and_then(Value::as_f64);
        if let (Some(s), Some(e)) = (start, end) {
            keep(&mut out, kind, s, e);
        }
    }
    out
}

/// Número do MAL na resposta de mapeamento do Kitsu.
pub fn mal_from_mapping(v: &Value) -> Option<u32> {
    let ext = v.pointer("/data/0/attributes/externalId")?;
    ext.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| ext.as_u64().and_then(|n| u32::try_from(n).ok()))
}

async fn get_json(client: &Client, url: &str) -> Option<Value> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_BODY {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

async fn fetch(lookup: &Lookup) -> Vec<Segment> {
    let client = addon::client_no_redirect();
    match lookup {
        Lookup::IntroDb(query) => {
            let url = format!("https://api.theintrodb.org/v3/media?{query}");
            get_json(&client, &url)
                .await
                .map(|v| introdb_segments(&v))
                .unwrap_or_default()
        }
        Lookup::Kitsu { id, episode } => {
            let map_url = format!(
                "https://kitsu.app/api/edge/anime/{id}/mappings?filter[externalSite]=myanimelist/anime"
            );
            let Some(mal) = get_json(&client, &map_url)
                .await
                .and_then(|v| mal_from_mapping(&v))
            else {
                return Vec::new();
            };
            let url = format!(
                "https://api.aniskip.com/v2/skip-times/{mal}/{episode}?types[]=op&types[]=ed&types[]=recap&types[]=mixed-op&types[]=mixed-ed&episodeLength=0"
            );
            get_json(&client, &url)
                .await
                .map(|v| aniskip_segments(&v))
                .unwrap_or_default()
        }
    }
}

/// Tempos para o episódio que está tocando. Falha ou id desconhecido dá lista vazia:
/// pular é um extra, nunca impede o vídeo de tocar.
#[tauri::command]
pub async fn skip_segments(kind: String, id: String, video_id: String) -> Result<Value, String> {
    let Some(lookup) = parse_target(&kind, &id, &video_id) else {
        return Ok(json!({ "segments": [] }));
    };
    let segments = tokio::time::timeout(TIMEOUT, fetch(&lookup))
        .await
        .unwrap_or_default();
    Ok(json!({ "segments": segments }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_uses_imdb_id_only() {
        assert_eq!(
            parse_target("movie", "tt0111161", "tt0111161"),
            Some(Lookup::IntroDb("imdb_id=tt0111161".into()))
        );
    }

    #[test]
    fn series_adds_season_and_episode_from_video_id() {
        assert_eq!(
            parse_target("series", "tt0903747", "tt0903747:1:2"),
            Some(Lookup::IntroDb(
                "imdb_id=tt0903747&season=1&episode=2".into()
            ))
        );
    }

    #[test]
    fn tmdb_series_uses_the_last_two_parts() {
        assert_eq!(
            parse_target("series", "tmdb:1396", "tmdb:1396:1:2"),
            Some(Lookup::IntroDb("tmdb_id=1396&season=1&episode=2".into()))
        );
    }

    #[test]
    fn rejects_ids_that_are_not_plain_numbers() {
        assert_eq!(parse_target("movie", "tt12x", "tt12x"), None);
        assert_eq!(parse_target("movie", "tt1&x=1", "tt1&x=1"), None);
        assert_eq!(parse_target("movie", "tmdb:", "tmdb:"), None);
        assert_eq!(parse_target("movie", "kitsu:4a", "kitsu:4a"), None);
    }

    #[test]
    fn episode_without_season_is_not_looked_up() {
        assert_eq!(parse_target("series", "tt0903747", "tt0903747"), None);
        assert_eq!(parse_target("series", "tt0903747", "tt0903747:x:2"), None);
    }

    #[test]
    fn anime_maps_to_kitsu_with_episode() {
        assert_eq!(
            parse_target("anime", "kitsu:42", "kitsu:42:3"),
            Some(Lookup::Kitsu {
                id: "42".into(),
                episode: 3
            })
        );
        assert_eq!(
            parse_target("anime", "kitsu:42", "kitsu:42"),
            Some(Lookup::Kitsu {
                id: "42".into(),
                episode: 1
            })
        );
    }

    #[test]
    fn introdb_converts_ms_and_drops_open_ended_segments() {
        let v = json!({
            "intro": [{ "start_ms": 5000, "end_ms": 90000 }],
            "credits": [{ "start_ms": 1200000, "end_ms": null }],
            "recap": [{ "start_ms": 0, "end_ms": 500 }]
        });
        assert_eq!(
            introdb_segments(&v),
            vec![Segment {
                kind: "intro".into(),
                start: 5.0,
                end: 90.0
            }]
        );
    }

    #[test]
    fn aniskip_maps_skip_types() {
        let v = json!({ "results": [
            { "skipType": "op", "interval": { "startTime": 30.0, "endTime": 120.0 } },
            { "skipType": "mixed-ed", "interval": { "startTime": 1300.0, "endTime": 1400.0 } },
            { "skipType": "recap", "interval": { "startTime": 0.0, "endTime": 40.0 } },
            { "skipType": "unknown", "interval": { "startTime": 0.0, "endTime": 40.0 } }
        ]});
        let segments = aniskip_segments(&v);
        let kinds: Vec<&str> = segments.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, ["intro", "credits", "recap"]);
    }

    #[test]
    fn active_segment_ends_half_a_second_early() {
        let segs = vec![Segment {
            kind: "intro".into(),
            start: 5.0,
            end: 90.0,
        }];
        assert!(active_segment(&segs, 4.9).is_none());
        assert_eq!(active_segment(&segs, 5.0).map(|s| s.end), Some(90.0));
        assert!(active_segment(&segs, 89.7).is_none());
    }

    #[test]
    fn mal_id_accepts_string_or_number() {
        assert_eq!(
            mal_from_mapping(&json!({ "data": [{ "attributes": { "externalId": "5114" } }] })),
            Some(5114)
        );
        assert_eq!(
            mal_from_mapping(&json!({ "data": [{ "attributes": { "externalId": 5114 } }] })),
            Some(5114)
        );
        assert_eq!(mal_from_mapping(&json!({ "data": [] })), None);
    }
}
