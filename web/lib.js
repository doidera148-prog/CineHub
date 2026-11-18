// CineHub — funções puras (sem DOM). Carregado antes de app.js e testado com `node --test`.
(function (root) {
  const str = (v) => (typeof v === 'string' ? v : '');
  const num = (v) => (Number.isFinite(Number(v)) ? Number(v) : 0);

  // Grava várias chaves como uma operação: se alguma falhar, tenta restaurar o estado anterior.
  function writeStorageBatch(storage, entries) {
    if (!storage || !entries || typeof entries !== 'object') return false;
    const keys = Object.keys(entries);
    let previous;
    let serialized;
    try {
      previous = keys.map((key) => storage.getItem(key));
      serialized = keys.map((key) => JSON.stringify(entries[key]));
      if (serialized.some((value) => value === undefined)) return false;
    } catch {
      return false;
    }
    try {
      keys.forEach((key, index) => storage.setItem(key, serialized[index]));
      return true;
    } catch {
      keys.forEach((key, index) => {
        try {
          if (previous[index] === null) storage.removeItem(key);
          else storage.setItem(key, previous[index]);
        } catch {
          // A restauração é best-effort; não mascara a falha original.
        }
      });
      return false;
    }
  }

  // Streams locais do motor torrent já estão preparados pelo backend e não podem passar pelo proxy externo.
  function isLocalTorrentStream(stream) {
    const url = String(stream?.url ?? '').trim();
    try {
      const parsed = new URL(url);
      return ['127.0.0.1', 'localhost', '[::1]'].includes(parsed.hostname)
        && parsed.pathname.startsWith('/stream/');
    } catch {
      return false;
    }
  }

  function shouldUseMediaProxy(stream) {
    return !isLocalTorrentStream(stream);
  }

  function isTorrentSource(stream) {
    const url = String(stream?.url ?? '').trim();
    if (isLocalTorrentStream(stream)) return false;
    return url.startsWith('magnet:?') || Boolean(stream?.infoHash) || Boolean(stream?.hash);
  }

  // Só http(s) é aceito como link de imagem ou de página.
  function isWebUrl(u) {
    return typeof u === 'string' && /^https?:\/\//i.test(u);
  }

  // Imagem de fundo em CSS: só http(s), com aspas e parênteses codificados para não quebrar o estilo.
  function cssUrl(u) {
    if (!isWebUrl(u)) return '';
    const safe = encodeURI(u).replace(/['()]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase());
    return `url('${safe}')`;
  }

  // Mescla um backup sem confiar nas chaves ou nos registros recebidos.
  function mergeBackupData(currentFavs, currentProgress, data, maxFavs = 500, maxProgress = 50) {
    if (!data || data.app !== 'CineHub' || data.version !== 1) throw new Error('backup incompatível ou inválido');

    const favs = Array.isArray(currentFavs) ? currentFavs.slice() : [];
    const ids = new Set(favs.map((f) => f && f.id).filter((id) => typeof id === 'string'));
    if (Array.isArray(data.favs)) {
      for (const item of data.favs) {
        if (!item || typeof item.id !== 'string' || !item.id.trim() || item.id.length > 512 || ids.has(item.id)) continue;
        favs.push(cleanFav(item));
        ids.add(item.id);
        if (favs.length >= maxFavs) break;
      }
    }

    // Objetos comuns podem herdar chaves como __proto__; use um dicionário sem protótipo.
    const progress = Object.create(null);
    const forbidden = new Set(['__proto__', 'prototype', 'constructor']);
    const mergeProgress = (source) => {
      if (!source || typeof source !== 'object' || Array.isArray(source)) return;
      for (const [key, value] of Object.entries(source)) {
        if (!key || key.length > 512 || forbidden.has(key) || !value || typeof value !== 'object' || Array.isArray(value)) continue;
        const incoming = cleanProgress(value);
        incoming.t = Math.max(0, incoming.t);
        incoming.duration = Math.max(0, incoming.duration);
        incoming.updated = Math.max(0, incoming.updated);
        if (!Object.prototype.hasOwnProperty.call(progress, key) || incoming.updated > progress[key].updated) progress[key] = incoming;
      }
    };
    mergeProgress(currentProgress);
    mergeProgress(data.progress);

    const kept = Object.entries(progress)
      .sort((a, b) => b[1].updated - a[1].updated)
      .slice(0, maxProgress);
    return { favs: favs.slice(0, maxFavs), progress: Object.fromEntries(kept) };
  }

  // Cria um backup canônico: aplica os mesmos limites e saneamento usados na importação.
  function createBackupData(favs, progress, exportedAt = new Date().toISOString(), maxFavs = 500, maxProgress = 50) {
    const cleaned = mergeBackupData([], {}, {
      app: 'CineHub',
      version: 1,
      favs,
      progress,
    }, maxFavs, maxProgress);
    return {
      app: 'CineHub',
      version: 1,
      exportedAt: typeof exportedAt === 'string' ? exportedAt : new Date().toISOString(),
      favs: cleaned.favs,
      progress: cleaned.progress,
    };
  }

  function cleanFav(f) {
    return { id: str(f.id), type: str(f.type), name: str(f.name), poster: str(f.poster), releaseInfo: str(f.releaseInfo) };
  }

  function cleanProgress(p) {
    return {
      t: num(p.t),
      duration: num(p.duration),
      label: str(p.label),
      seriesId: str(p.seriesId),
      type: str(p.type),
      name: str(p.name),
      poster: str(p.poster),
      updated: num(p.updated),
      watched: p.watched === true,
    };
  }

  // Percentual assistido (0 a 100) para a barra sobre o cartaz.
  function progressPercent(p) {
    if (!p || !(p.duration > 0)) return 0;
    return Math.max(0, Math.min(100, (num(p.t) / p.duration) * 100));
  }

  // Qualidade estimada pelo texto da fonte (resolução).
  function qualityScore(text) {
    const t = String(text || '').toLowerCase();
    if (/2160p|4k|uhd/.test(t)) return 4;
    if (/1080p/.test(t)) return 3;
    if (/720p/.test(t)) return 2;
    if (/480p|360p/.test(t)) return 1;
    return 0;
  }

  // Dublagem ou áudio em português, segundo o texto da fonte.
  function isPortuguese(text) {
    return /dublad|dual|pt-?br|portugu/i.test(String(text || ''));
  }

  // Identifica o contêiner declarado pela fonte, sem confundir extensão com codec.
  // URLs sem extensão continuam desconhecidas; MKV é bloqueado porque o player-alvo não o suporta.
  function streamContainer(s) {
    const mime = String(s?.mimeType || s?.mime || '').toLowerCase();
    if (/matroska|x-matroska/.test(mime)) return 'mkv';
    if (/video\\/mp4/.test(mime)) return 'mp4';
    if (/video\\/webm/.test(mime)) return 'webm';
    const values = [s?.url, s?.fileName, s?.name, s?.title].filter(Boolean);
    for (const value of values) {
      let path = String(value);
      try { path = new URL(path).pathname; } catch { /* nome de arquivo ou rótulo do addon */ }
      const match = path.match(/\\.(mkv|mp4|m4v|webm)(?:$|[?#])/i) || String(value).match(/\\b(matroska|mkv)\\b/i);
      if (!match) continue;
      const ext = String(match[1]).toLowerCase();
      return ext === 'matroska' ? 'mkv' : ext === 'm4v' ? 'mp4' : ext;
    }
    return 'unknown';
  }
  function isMkvStream(s) {
    return streamContainer(s) === 'mkv';
  }

  // MP4/WebM têm prioridade; MKV é descartado no carregamento das fontes.
  // Mantém preferência por português e torrents, sem permitir que ela supere o formato compatível.
  function sortStreams(list) {
    const score = (s) => {
      const text = `${s.name || ''} ${s.title || ''}`;
      const container = streamContainer(s);
      const format = container === 'mp4' || container === 'webm' ? 2000 : 0;
      const hasLink = Boolean(s.url || s.externalUrl);
      const torrent = !hasLink && Boolean(s.infoHash || s.hash) ? 1000 : 0;
      const playable = hasLink || s.infoHash || s.hash ? 100 : 0;
      const portuguese = isPortuguese(text) ? 10000 : 0;
      return portuguese + format + torrent + playable + qualityScore(text) * 2;
    };
    return list
      .map((s, i) => ({ s, i, score: score(s) }))
      .sort((a, b) => b.score - a.score || a.i - b.i)
      .map((x) => x.s);
  }

  // Idioma do áudio pelo nome do arquivo de um torrent: 'pt', 'en' ou 'other'.
  function torrentLanguage(name) {
    const t = String(name || '');
    if (isPortuguese(t) || /\bpt\b/i.test(t)) return 'pt';
    if (/english|\beng\b|ingl[eê]s/i.test(t)) return 'en';
    return 'other';
  }

  // Próximo episódio na ordem (temporada, episódio). Null quando é o último.
  function nextEpisode(videos, currentId) {
    const eps = (Array.isArray(videos) ? videos : [])
      .filter((v) => v && v.id)
      .slice()
      .sort((a, b) => num(a.season) - num(b.season) || num(a.episode) - num(b.episode));
    const i = eps.findIndex((v) => v.id === currentId);
    return i < 0 || i + 1 >= eps.length ? null : eps[i + 1];
  }

  // Temporada e episódio vindos do addon: sempre número. Texto malicioso vira 0 ou null,
  // então nunca chega ao HTML como conteúdo.
  function seasonOf(v) {
    return num(v && v.season);
  }

  function episodeOf(v) {
    const n = Number(v && v.episode);
    return Number.isFinite(n) ? n : null;
  }

  // Tipos de conteúdo do Stremio. "anime" tem episódios como as séries: mesma tela de temporadas,
  // mesmo caminho para fontes e para o próximo episódio. A chave é o tipo que o addon declara.
  // Pular abertura, recapitulação e créditos. `segments` vem do backend, em segundos.
  const SKIP_LABELS = { intro: 'Pular abertura', recap: 'Pular recapitulação', preview: 'Pular prévia', credits: 'Pular créditos' };
  // Trecho que vale no instante t: some meio segundo antes do fim, para o botão não piscar
  function activeSegment(segments, t) {
    if (!Array.isArray(segments) || !Number.isFinite(t)) return null;
    return segments.find((s) => s && t >= s.start && t < s.end - 0.5) || null;
  }
  // Teclas 0–9 pulam para 0–90% do vídeo; null se a tecla não for um dígito
  function seekFraction(key, duration) {
    if (!/^[0-9]$/.test(key) || !Number.isFinite(duration) || duration <= 0) return null;
    return (Number(key) / 10) * duration;
  }
  const SERIES_LIKE = ['series', 'anime'];
  function isSeriesLike(type) {
    return SERIES_LIKE.includes(str(type));
  }
  const TYPE_LABELS = { movie: 'Filme', series: 'Série', anime: 'Anime' };
  function typeLabel(type) {
    return TYPE_LABELS[str(type)] || '';
  }

  // Barra de fontes (filtro e ordenação), adaptada da barra do dxbi.plugin.js (dx.bi, GPL-3.0-or-later).
  function qualityLabel(text) {
    const t = String(text || '').toLowerCase();
    if (/2160p|4k|uhd/.test(t)) return '4K';
    if (/1080p/.test(t)) return '1080p';
    if (/720p/.test(t)) return '720p';
    return 'Outras';
  }
  function streamText(s) {
    return `${s.name || ''} ${s.title || ''}`;
  }
  // Filtros: cada grupo é uma lista de valores escolhidos; lista vazia não filtra.
  function filterStreams(list, f) {
    return list.filter((s) => {
      const text = streamText(s);
      if (f.quality && f.quality.length && !f.quality.includes(qualityLabel(text))) return false;
      if (f.lang && f.lang.length && !f.lang.includes(torrentLanguage(text))) return false;
      if (f.addon && f.addon.length && !f.addon.includes(str(s.addonName))) return false;
      return true;
    });
  }
  // Ordenação: 'default' mantém a ordem de sortStreams; as outras são estáveis.
  function sortStreamsBy(list, mode) {
    if (mode === 'quality') return [...list].sort((a, b) => qualityScore(streamText(b)) - qualityScore(streamText(a)));
    if (mode === 'pt') return [...list].sort((a, b) => Number(isPortuguese(streamText(b))) - Number(isPortuguese(streamText(a))));
    return list;
  }

  root.CineHubLib = {
    str,
    isLocalTorrentStream,
    shouldUseMediaProxy,
    isTorrentSource,
    qualityLabel,
    filterStreams,
    sortStreamsBy,
    isSeriesLike,
    typeLabel,
    num,
    seasonOf,
    episodeOf,
    isWebUrl,
    cssUrl,
    cleanFav,
    writeStorageBatch,
    mergeBackupData,
    createBackupData,
    cleanProgress,
    progressPercent,
    sortStreams,
    streamContainer,
    isMkvStream,
    torrentLanguage,
    nextEpisode,
    isPortuguese,
    SKIP_LABELS,
    activeSegment,
    seekFraction,
  };
})(typeof globalThis !== 'undefined' ? globalThis : this);
