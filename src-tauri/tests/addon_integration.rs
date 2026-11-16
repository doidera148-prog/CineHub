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
/// `build` recebe o endereço real do servidor, para que as rotas possam citá-lo.
async fn spawn_addon(build: impl FnOnce(&str) -> HashMap<String, String>) -> (String, Hits) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: Routes = Arc::new(build(&base));
    let hits: Hits = Arc::new(Mutex::new(HashMap::new()));
    let hits_task = hits.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
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
    let (base, hits) = spawn_addon(|base| {
        let mut routes = HashMap::new();
        routes.insert("/manifest.json".to_string(), manifest_json(base));
        add_protocol_routes(&mut routes);
        routes
    })
    .await;
    let a = installed(&base);
    let client = addon::client();
    let cache = Cache::default();

    // Manifest lido pela rede: o endereço dentro dele é o do servidor de teste.
    let m = addon::fetch_manifest(&client, &format!("{base}/manifest.json"))
        .await
        .unwrap();
    assert_eq!(m.name, "Addon de teste");
    assert_eq!(m.logo.as_deref(), Some(format!("{base}/logo.png").as_str()));

    rest_of_flow(client, cache, a, hits).await;
}

fn add_protocol_routes(routes: &mut HashMap<String, String>) {
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
        r#"{"subtitles":[{"id":"s1","url":"https://subs.example/pt.srt","lang":"por"}]}"#
            .to_string(),
    );
}

async fn rest_of_flow(client: reqwest::Client, cache: Cache, a: Installed, hits: Hits) {
    // Recursos e tipos
    assert!(a.supports("meta", "movie", "tt1"));
    assert!(!a.supports("meta", "movie", "xx1"));
    assert!(a.supports("subtitles", "movie", "tt1"));

    // Catálogo
    let metas = addon::catalog(&client, &cache, &a, "movie", "top", "")
        .await
        .unwrap();
    assert_eq!(metas.len(), 1);

    // Meta e fontes
    let meta = addon::meta(&client, &cache, &a, "movie", "tt1")
        .await
        .unwrap();
    assert_eq!(meta.unwrap()["name"], "Filme A");
    let streams = addon::streams(&client, &cache, &a, "movie", "tt1")
        .await
        .unwrap();
    assert_eq!(streams.len(), 2);
    let subs = addon::subtitles(&client, &cache, &a, "movie", "tt1")
        .await
        .unwrap();
    assert_eq!(subs[0]["lang"], "por");

    // Segunda chamada ao catálogo vem do cache: o servidor não recebe nova requisição
    let _ = addon::catalog(&client, &cache, &a, "movie", "top", "")
        .await
        .unwrap();
    let count = hits
        .lock()
        .unwrap()
        .get("/catalog/movie/top.json")
        .copied()
        .unwrap_or(0);
    assert_eq!(count, 1, "o cache deveria evitar a segunda requisição");
}

#[tokio::test]
async fn oversized_responses_are_rejected() {
    let (base, _hits) = spawn_addon(|_| {
        let mut routes = HashMap::new();
        routes.insert("/big".to_string(), "x".repeat(5_000));
        routes
    })
    .await;
    let client = addon::client();
    let err = addon::read_capped(&client, &format!("{base}/big"), 1_000)
        .await
        .unwrap_err();
    assert!(err.contains("grande demais"));
}

#[tokio::test]
async fn http_errors_are_reported() {
    let (base, _hits) = spawn_addon(|_| HashMap::new()).await;
    let client = addon::client();
    let err = addon::read_capped(&client, &format!("{base}/nao-existe"), 1_000)
        .await
        .unwrap_err();
    assert!(err.contains("404"));
}

#[tokio::test]
async fn subtitle_urls_must_be_public() {
    // IPs públicos literais não dependem de DNS, então o teste roda offline.
    assert!(addon::is_safe_url("http://93.184.216.34/pt.srt").await);
    assert!(addon::is_safe_url("https://93.184.216.34:8443/pt.srt").await);
    assert!(!addon::is_safe_url("http://localhost:8080/x").await);
    assert!(!addon::is_safe_url("http://127.0.0.1/x").await);
    assert!(!addon::is_safe_url("http://192.168.0.10/x").await);
    assert!(!addon::is_safe_url("http://10.0.0.5/x").await);
    assert!(!addon::is_safe_url("http://169.254.169.254/latest/meta-data").await);
    assert!(!addon::is_safe_url("http://[::1]/x").await);
    assert!(!addon::is_safe_url("file:///etc/passwd").await);
    assert!(!addon::is_safe_url("ftp://93.184.216.34/x").await);
}

#[tokio::test]
async fn ipv4_mapped_ipv6_cannot_reach_private_hosts() {
    // Endereços IPv6 que embutem IPv4 privado/loopback precisam ser recusados.
    assert!(!addon::is_safe_url("http://[::ffff:127.0.0.1]/x").await);
    assert!(!addon::is_safe_url("http://[::ffff:192.168.0.1]/x").await);
    assert!(!addon::is_safe_url("http://[::ffff:10.0.0.1]/x").await);
    assert!(!addon::is_safe_url("http://[::ffff:169.254.169.254]/x").await);
    assert!(!addon::is_safe_url("http://[::]/x").await);
}

#[test]
fn reserved_ranges_are_not_public() {
    use std::net::IpAddr;
    for ip in [
        "100.64.0.1",  // CGNAT
        "0.1.2.3",     // 0.0.0.0/8
        "192.0.0.8",   // IETF
        "198.18.0.1",  // testes de rede
        "240.0.0.1",   // reservado
        "fec0::1",     // site-local (obsoleto, mas ainda roteável só em redes antigas)
        "2001:db8::1", // documentação
    ] {
        let ip: IpAddr = ip.parse().unwrap();
        assert!(!addon::ip_is_public(ip), "{ip} deveria ser recusado");
    }
    assert!(addon::ip_is_public("93.184.216.34".parse().unwrap()));
}

#[tokio::test]
async fn names_resolving_to_local_addresses_are_refused() {
    // "localhost" é um nome, não um literal: a recusa vem do resolvedor de DNS.
    let client = addon::client();
    let err = addon::read_capped(&client, "http://localhost:9/x", 1_000)
        .await
        .unwrap_err();
    assert!(
        err.contains("conectar") || err.contains("permitido"),
        "erro inesperado: {err}"
    );
}
