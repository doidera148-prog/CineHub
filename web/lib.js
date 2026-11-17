// CineHub — funções puras (sem DOM). Carregado antes de app.js e testado com `node --test`.
(function (root) {
  const str = (v) => (typeof v === 'string' ? v : '');
  const num = (v) => (Number.isFinite(Number(v)) ? Number(v) : 0);

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

  // Ordena as fontes: as reproduzíveis primeiro, depois as de maior resolução, com dublagem
  // em português na frente entre empates. A ordem original é mantida quando tudo empata.
  function sortStreams(list) {
    const score = (s) => {
      const text = `${s.name || ''} ${s.title || ''}`;
      const playable = Boolean(s.url || s.externalUrl || s.infoHash || s.hash) ? 100 : 0;
      return playable + qualityScore(text) * 2 + (isPortuguese(text) ? 1 : 0);
    };
    return list
      .map((s, i) => ({ s, i, score: score(s) }))
      .sort((a, b) => b.score - a.score || a.i - b.i)
      .map((x) => x.s);
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

  root.CineHubLib = {
    str,
    num,
    seasonOf,
    episodeOf,
    isWebUrl,
    cssUrl,
    cleanFav,
    cleanProgress,
    progressPercent,
    sortStreams,
    nextEpisode,
    isPortuguese,
  };
})(typeof globalThis !== 'undefined' ? globalThis : this);
