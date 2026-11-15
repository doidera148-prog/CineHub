//! Teste de integração: sobe um addon falso em localhost e valida o protocolo do Stremio
//! de ponta a ponta (manifest, catálogo, meta, streams, legendas, cache e limites).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cinehub_lib::addon::{self, Cache, Installed, Manifest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type Routes = Arc<HashMap<String, String>>;
type Hits = Arc<Mutex<HashMap<String, usize>>>;

/// Servidor HTTP mínimo: responde ao caminho exato com o corpo configurado e conta as chamadas.
async fn spawn_addon(routes: HashMap<String, String>) -> (String, Hits) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: Routes = Arc::new(routes);
    let hits: Hits = Arc::new(Mutex::new(HashMap::new()));
    let hits_task = hits.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            let routes = routes.clone();
            let hits = hits_task.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                *hits.lock().unwrap().entry(path.clone()).or_insert(0) += 1;

                let (status, body) = match routes.get(&path) {
                    Some(b) => ("200 OK", b.clone()),
                    None => ("404 Not Found", "{}".to_string()),
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base, hits)
}

fn manifest_json(base: &str) -> String {
    format!(
        r#"{{"id":"test.addon","version":"1.0.0","name":"Addon de teste","types":["movie","series"],
            "idPrefixes":["tt"],
            "resources":["catalog","meta",{{"name":"stream","types":["movie","series"],"idPrefixes":["tt"]}},"subtitles"],
            "catalogs":[{{"type":"movie","id":"top","name":"Populares"}}],
            "logo":"{base}/logo.png"}}"#
    )
}

fn installed(base: &str) -> Installed {
    let manifest: Manifest = serde_json::from_str(&manifest_json(base)).unwrap();
    Installed {
        transport_url: format!("{base}/manifest.json"),
        manifest,
    }
}

#[tokio::test]
async fn full_protocol_flow_and_cache() {
    let base_placeholder = "PLACEHOLDER";
    let mut routes = HashMap::new();
    routes.insert("/manifest.json".to_string(), manifest_json(base_placeholder));
    routes.insert(
        "/catalog/movie/top.json".to_string(),
        r#"{"metas":[{"id":"tt1","type":"movie","name":"Filme A"}]}"#.to_string(),
    );
    routes.insert(
        "/meta/movie/tt1.json".to_string(),
        r#"{"meta":{"id":"tt1","type":"movie","name":"Filme A"}}"#.to_string(),
    );
    routes.insert(
        "/stream/movie/tt1.json".to_string(),
        r#"{"streams":[{"url":"https://cdn.example/a.mp4","title":"1080p"},{"infoHash":"abc","title":"x"}]}"#.to_string(),
    );
    routes.insert(
        "/subtitles/movie/tt1.json".to_string(),
        r#"{"subtitles":[{"id":"s1","url":"https://subs.example/pt.srt","lang":"por"}]}"#.to_string(),
    );
    let (base, hits) = spawn_addon(routes).await;
    // Reescreve o manifest com o endereço real do servidor.
    let manifest_body = manifest_json(&base);
    let a = installed(&base);
    let client = addon::client();
    let cache = Cache::default();

    // Manifest lido pela rede
    let m = addon::fetch_manifest(&client, &format!("{base}/manifest.json")).await.unwrap();
    assert_eq!(m.name, "Addon de teste");
    assert!(manifest_body.contains(&base));

    // Recursos e tipos
    assert!(a.supports("meta", "movie", "tt1"));
    assert!(!a.supports("meta", "movie", "xx1"));
    assert!(a.supports("subtitles", "movie", "tt1"));

    // Catálogo
    let metas = addon::catalog(&client, &cache, &a, "movie", "top", "").await.unwrap();
    assert_eq!(metas.len(), 1);

    // Meta e fontes
    let meta = addon::meta(&client, &cache, &a, "movie", "tt1").await.unwrap();
    assert_eq!(meta.unwrap()["name"], "Filme A");
    let streams = addon::streams(&client, &cache, &a, "movie", "tt1").await.unwrap();
    assert_eq!(streams.len(), 2);
    let subs = addon::subtitles(&client, &cache, &a, "movie", "tt1").await.unwrap();
    assert_eq!(subs[0]["lang"], "por");

    // Segunda chamada ao catálogo vem do cache: o servidor não recebe nova requisição
    let _ = addon::catalog(&client, &cache, &a, "movie", "top", "").await.unwrap();
    let count = hits.lock().unwrap().get("/catalog/movie/top.json").copied().unwrap_or(0);
    assert_eq!(count, 1, "o cache deveria evitar a segunda requisição");
}

#[tokio::test]
async fn oversized_responses_are_rejected() {
    let mut routes = HashMap::new();
    routes.insert("/big".to_string(), "x".repeat(5_000));
    let (base, _hits) = spawn_addon(routes).await;
    let client = addon::client();
    let err = addon::read_capped(&client, &format!("{base}/big"), 1_000).await.unwrap_err();
    assert!(err.contains("grande demais"));
}

#[tokio::test]
async fn http_errors_are_reported() {
    let (base, _hits) = spawn_addon(HashMap::new()).await;
    let client = addon::client();
    let err = addon::read_capped(&client, &format!("{base}/nao-existe"), 1_000).await.unwrap_err();
    assert!(err.contains("404"));
}

#[test]
fn subtitle_urls_must_be_public() {
    assert!(addon::is_safe_url("https://subs.example.com/pt.srt"));
    assert!(addon::is_safe_url("http://93.184.216.34/pt.srt"));
    assert!(!addon::is_safe_url("http://localhost:8080/x"));
    assert!(!addon::is_safe_url("http://127.0.0.1/x"));
    assert!(!addon::is_safe_url("http://192.168.0.10/x"));
    assert!(!addon::is_safe_url("http://10.0.0.5/x"));
    assert!(!addon::is_safe_url("http://169.254.169.254/latest/meta-data"));
    assert!(!addon::is_safe_url("http://[::1]/x"));
    assert!(!addon::is_safe_url("file:///etc/passwd"));
    assert!(!addon::is_safe_url("ftp://example.com/x"));
}
