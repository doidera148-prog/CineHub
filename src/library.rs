//! Indexa a pasta de mídia local e reconhece filmes e episódios pelo nome do arquivo.
//!
//! Exemplos aceitos:
//!   Filmes:   "A Origem (2010) 1080p.mkv", "Interestelar.2014.720p.mp4"
//!   Séries:   "Dark.S01E02.1080p.mkv", "Breaking Bad - s05e16.mp4"
//!   Legendas: "Dark.S01E02.pt-BR.srt" (ao lado do vídeo, mesmo nome)

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

const VIDEO_EXT: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "m4v"];
const SUB_EXT: &[&str] = &["srt", "vtt"];
/// Marcadores no nome da legenda que indicam português.
const SUB_LANG_MARKERS: &[&str] = &["pt", "por", "pob", "portug"];
/// Termos de qualidade/origem que encerram o título do filme.
const TAGS: &[&str] = &[
    "bluray", "brrip", "bdrip", "webrip", "webdl", "web", "hdrip", "dvdrip", "x264", "x265",
    "h264", "h265", "hevc", "dual", "dublado", "legendado",
];
const QUALITIES: &[&str] = &["2160p", "1080p", "720p", "480p"];

pub struct Movie {
    pub id: String,
    pub title: String,
    pub year: Option<u32>,
}

pub struct Series {
    pub id: String,
    pub title: String,
    /// (temporada, episódio) -> id do vídeo do episódio
    pub episodes: BTreeMap<(u32, u32), String>,
}

#[derive(Default)]
pub struct Library {
    pub movies: Vec<Movie>,
    /// chave: slug da série
    pub series: BTreeMap<String, Series>,
    /// id do vídeo -> caminho do arquivo
    pub media: HashMap<String, PathBuf>,
    /// id do vídeo -> caminho da legenda em português
    pub subs: HashMap<String, PathBuf>,
    /// id do vídeo -> qualidade (ex.: "1080p")
    pub quality: HashMap<String, String>,
}

pub fn scan(dir: &Path) -> Library {
    let mut lib = Library::default();

    let mut files: Vec<PathBuf> = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| has_ext(p, VIDEO_EXT))
        .collect();
    files.sort();

    for path in files {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        let quality = detect_quality(&stem);

        if let Some((raw_title, season, episode)) = find_episode(&stem) {
            let mut title = clean(&raw_title);
            if title.is_empty() {
                title = parent_name(&path);
            }
            let slug = slugify(&title);
            let vid = format!("local:s:{slug}:{season}:{episode}");

            let series = lib.series.entry(slug.clone()).or_insert_with(|| Series {
                id: format!("local:s:{slug}"),
                title: title.clone(),
                episodes: BTreeMap::new(),
            });
            series.episodes.insert((season, episode), vid.clone());
            register(&mut lib, &vid, &path, &stem, quality);
        } else {
            let (raw_title, year) = parse_movie(&stem);
            let title = if raw_title.is_empty() { clean(&stem) } else { raw_title };
            let mut slug = slugify(&title);
            if let Some(y) = year {
                slug = format!("{slug}-{y}");
            }
            let vid = format!("local:m:{slug}");
            if lib.media.contains_key(&vid) {
                eprintln!("ignorado (duplicado): {}", path.display());
                continue;
            }
            lib.movies.push(Movie { id: vid.clone(), title, year });
            register(&mut lib, &vid, &path, &stem, quality);
        }
    }

    lib.movies.sort_by(|a, b| a.title.cmp(&b.title));
    lib
}

fn register(lib: &mut Library, vid: &str, path: &Path, stem: &str, quality: Option<String>) {
    lib.media.insert(vid.to_string(), path.to_path_buf());
    if let Some(q) = quality {
        lib.quality.insert(vid.to_string(), q);
    }
    if let Some(sub) = find_subtitle(path, stem) {
        lib.subs.insert(vid.to_string(), sub);
    }
}

/// Procura legenda em português na mesma pasta cujo nome comece com o título do vídeo.
/// Ex.: "Dark.S01E02.720p.mkv" casa com "Dark.S01E02.pt-BR.srt".
fn find_subtitle(video: &Path, stem: &str) -> Option<PathBuf> {
    let dir = video.parent()?;
    let base = key(&base_name(stem));
    if base.is_empty() {
        return None;
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if !has_ext(&p, SUB_EXT) {
            continue;
        }
        let k = key(p.file_name()?.to_str()?);
        if let Some(rest) = k.strip_prefix(&base) {
            if SUB_LANG_MARKERS.iter().any(|m| rest.contains(m)) {
                return Some(p);
            }
        }
    }
    None
}

/// Parte do nome do vídeo antes da qualidade/tags: "Dark.S01E02.720p" -> "Dark.S01E02".
fn base_name(stem: &str) -> String {
    stem.replace(['_', ' '], ".")
        .split('.')
        .take_while(|w| !is_tag(w))
        .collect::<Vec<_>>()
        .join(".")
}

/// Chave comparável: só letras e dígitos, minúsculos.
fn key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Procura SxxExx no nome. Retorna (título antes do padrão, temporada, episódio).
fn find_episode(stem: &str) -> Option<(String, u32, u32)> {
    let c: Vec<char> = stem.chars().collect();
    for i in 0..c.len() {
        if c[i] != 's' && c[i] != 'S' {
            continue;
        }
        if i > 0 && c[i - 1].is_alphanumeric() {
            continue;
        }
        let s0 = i + 1;
        let mut j = s0;
        while j < c.len() && c[j].is_ascii_digit() {
            j += 1;
        }
        if j == s0 || j >= c.len() || (c[j] != 'e' && c[j] != 'E') {
            continue;
        }
        let e0 = j + 1;
        let mut k = e0;
        while k < c.len() && c[k].is_ascii_digit() {
            k += 1;
        }
        if k == e0 {
            continue;
        }
        let season: u32 = c[s0..j].iter().collect::<String>().parse().ok()?;
        let episode: u32 = c[e0..k].iter().collect::<String>().parse().ok()?;
        let prefix: String = c[..i].iter().collect();
        return Some((prefix, season, episode));
    }
    None
}

/// Separa título e ano de um nome de filme. Para antes de tags como 1080p ou BluRay.
fn parse_movie(stem: &str) -> (String, Option<u32>) {
    let norm = stem.replace(['.', '_'], " ");
    let mut words: Vec<&str> = Vec::new();
    let mut year = None;

    for w in norm.split_whitespace() {
        let bare = w.trim_matches(|c: char| "()[]{}".contains(c));
        if is_tag(bare) {
            break;
        }
        if bare.len() == 4 {
            if let Ok(y) = bare.parse::<u32>() {
                if (1900..=2100).contains(&y) && !words.is_empty() {
                    year = Some(y);
                    break;
                }
            }
        }
        words.push(bare);
    }
    (words.join(" "), year)
}

fn is_tag(word: &str) -> bool {
    let w = word.to_lowercase();
    if TAGS.contains(&w.as_str()) || QUALITIES.contains(&w.as_str()) || w == "4k" {
        return true;
    }
    w.len() > 1 && w.ends_with('p') && w[..w.len() - 1].chars().all(|c| c.is_ascii_digit())
}

fn detect_quality(stem: &str) -> Option<String> {
    stem.replace(['.', '_', '-', ' ', '[', ']', '(', ')'], " ")
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .find_map(|w| match w.as_str() {
            "4k" | "2160p" => Some("2160p".to_string()),
            q if QUALITIES.contains(&q) => Some(q.to_string()),
            _ => None,
        })
}

fn clean(s: &str) -> String {
    s.replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| c == '-' || c.is_whitespace())
        .to_string()
}

fn parent_name(p: &Path) -> String {
    p.parent()
        .and_then(|d| d.file_name())
        .and_then(|n| n.to_str())
        .map(clean)
        .unwrap_or_else(|| "Sem título".to_string())
}

pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

fn has_ext(p: &Path, exts: &[&str]) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| exts.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_with_year_and_quality() {
        assert_eq!(parse_movie("A.Origem.2010.1080p.BluRay"), ("A Origem".to_string(), Some(2010)));
        assert_eq!(parse_movie("1917 (2019)"), ("1917".to_string(), Some(2019)));
    }

    #[test]
    fn episode_patterns() {
        assert_eq!(find_episode("Dark.S01E02.1080p"), Some(("Dark.".to_string(), 1, 2)));
        assert_eq!(find_episode("Breaking Bad - s05e16"), Some(("Breaking Bad - ".to_string(), 5, 16)));
        assert_eq!(find_episode("Interestelar 2014"), None);
    }

    #[test]
    fn quality_and_slug() {
        assert_eq!(detect_quality("Filme.4K.HDR").as_deref(), Some("2160p"));
        assert_eq!(slugify("Breaking Bad!"), "breaking-bad");
    }
}
