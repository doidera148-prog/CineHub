//! Cliente do protocolo de addons do Stremio.
//!
//! Um addon é um servidor HTTP que publica `manifest.json` e responde às rotas:
//!   /catalog/{tipo}/{id}.json            (ou /catalog/{tipo}/{id}/search=texto.json)
//!   /meta/{tipo}/{id}.json
//!   /stream/{tipo}/{id}.json

use std::time::Duration;

use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub id_prefixes: Option<Vec<String>>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub catalogs: Vec<CatalogDef>,
}

/// Recurso pode ser apenas o nome ("meta") ou um objeto com tipos e prefixos próprios.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Resource {
    Name(String),
    Detailed(ResourceDetail),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDetail {
    pub name: String,
    #[serde(default)]
    pub types: Option<Vec<String>>,
    #[serde(default)]
    pub id_prefixes: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extra: Vec<ExtraDef>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtraDef {
    pub name: String,
    #[serde(default)]
    pub is_required: bool,
}

impl CatalogDef {
    /// Catálogos que exigem um parâmetro (ex.: busca) não entram na página inicial.
    pub fn requires_extra(&self) -> bool {
        self.extra.iter().any(|e| e.is_required)
    }

    pub fn supports_search(&self) -> bool {
        self.extra.iter().any(|e| e.name == "search")
    }
}

/// Addon instalado: URL do manifest e o manifest já lido.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Installed {
    pub transport_url: String,
    pub manifest: Manifest,
}

impl Installed {
    /// Endereço base das rotas do addon (o manifest sem "/manifest.json").
    pub fn base(&self) -> String {
        self.transport_url
            .strip_suffix("/manifest.json")
            .unwrap_or(&self.transport_url)
            .to_string()
    }

    /// Verifica se o addon responde ao recurso para este tipo e id, como o Stremio faz.
    pub fn supports(&self, resource: &str, kind: &str, id: &str) -> bool {
        let m = &self.manifest;
        m.resources.iter().any(|r| {
            let (name, types, prefixes) = match r {
                Resource::Name(n) => (n.as_str(), None, None),
                Resource::Detailed(d) => (d.name.as_str(), d.types.as_ref(), d.id_prefixes.as_ref()),
            };
            if name != resource {
                return false;
            }
            let types = types.unwrap_or(&m.types);
            let prefixes = prefixes.or(m.id_prefixes.as_ref());
            types.iter().any(|t| t == kind)
                && prefixes.map_or(true, |p| p.iter().any(|pre| id.starts_with(pre.as_str())))
        })
    }
}

pub fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("CineHub/0.2")
        .build()
        .expect("não foi possível criar o cliente HTTP")
}

pub async fn fetch_manifest(client: &Client, url: &str) -> Result<Manifest, String> {
    get_json(client, url).await
}

/// Lê um catálogo. `extra` vem já codificado, ex.: "search=dark" ou vazio.
pub async fn catalog(
    client: &Client,
    a: &Installed,
    kind: &str,
    id: &str,
    extra: &str,
) -> Result<Vec<Value>, String> {
    let url = if extra.is_empty() {
        format!("{}/catalog/{}/{}.json", a.base(), enc(kind), enc(id))
    } else {
        format!("{}/catalog/{}/{}/{}.json", a.base(), enc(kind), enc(id), extra)
    };
    let v: Value = get_json(client, &url).await?;
    Ok(v.get("metas").and_then(Value::as_array).cloned().unwrap_or_default())
}

pub async fn meta(client: &Client, a: &Installed, kind: &str, id: &str) -> Result<Option<Value>, String> {
    let url = format!("{}/meta/{}/{}.json", a.base(), enc(kind), enc(id));
    let v: Value = get_json(client, &url).await?;
    Ok(v.get("meta").filter(|m| !m.is_null()).cloned())
}

pub async fn streams(client: &Client, a: &Installed, kind: &str, id: &str) -> Result<Vec<Value>, String> {
    let url = format!("{}/stream/{}/{}.json", a.base(), enc(kind), enc(id));
    let v: Value = get_json(client, &url).await?;
    Ok(v.get("streams").and_then(Value::as_array).cloned().unwrap_or_default())
}

async fn get_json<T: DeserializeOwned>(client: &Client, url: &str) -> Result<T, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("falha ao conectar: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    resp.json::<T>().await.map_err(|e| format!("resposta inválida: {e}"))
}

/// Codifica um segmento de URL, mantendo ":" (usado em ids como "tt123:1:2").
pub fn enc(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(json: &str) -> Installed {
        Installed {
            transport_url: "https://addon.example/manifest.json".into(),
            manifest: serde_json::from_str(json).unwrap(),
        }
    }

    #[test]
    fn resource_matching_follows_stremio_rules() {
        let a = installed(
            r#"{"id":"x","version":"1","name":"X","types":["movie","series"],
                "idPrefixes":["tt"],
                "resources":["catalog","meta",{"name":"stream","types":["movie"],"idPrefixes":["tt"]}]}"#,
        );
        assert!(a.supports("meta", "series", "tt123"));
        assert!(!a.supports("meta", "series", "xx1"));
        assert!(a.supports("stream", "movie", "tt1"));
        assert!(!a.supports("stream", "series", "tt1"));
    }

    #[test]
    fn base_and_encoding() {
        let a = installed(r#"{"id":"x","version":"1","name":"X"}"#);
        assert_eq!(a.base(), "https://addon.example");
        assert_eq!(enc("tt1:1:2"), "tt1:1:2");
        assert_eq!(enc("ação"), "a%C3%A7%C3%A3o");
    }

    #[test]
    fn catalog_flags() {
        let c: CatalogDef = serde_json::from_str(
            r#"{"type":"movie","id":"s","extra":[{"name":"search","isRequired":true}]}"#,
        )
        .unwrap();
        assert!(c.requires_extra());
        assert!(c.supports_search());
    }
}
