// CineHub — interface web. Fala com o backend Tauri via invoke.

const $ = (s) => document.querySelector(s);
const view = $('#view');
const player = $('#player');
const video = $('#video');
const toastEl = $('#toast');
const confirmDialog = $('#confirm');
const topQ = $('#q');

const PAGE_HINT = 20; // a partir deste tamanho, a fila mostra "Carregar mais"
const MAX_PROGRESS = 50; // quantidade de títulos guardados no progresso

const esc = (s) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

// Chamadas ao backend Tauri (funciona em desktop, Android e iOS).
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    throw new Error(String(e));
  }
}

// Armazenamento local protegido: em janelas privadas ou com dados bloqueados, o app segue sem salvar.
const store = {
  get(key, fallback) {
    try {
      const raw = localStorage.getItem(key);
      return raw == null ? fallback : JSON.parse(raw);
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* sem armazenamento: nada a fazer */
    }
  },
};

const setView = (html) => (view.innerHTML = html);
const showError = (err) => setView(`<p class="muted">Erro: ${esc(err.message || err)}</p>`);
const skeleton = (n = 6) => `<div class="strip" aria-hidden="true">${'<div class="card"><div class="poster skel"></div></div>'.repeat(n)}</div>`;
const sectionOf = () => location.hash.replace(/^#\/?/, '').split('?')[0].split('/')[0] || 'home';

// Avisos e confirmações dentro do app (substituem alert/confirm do navegador).
let toastTimer;
function toast(msg) {
  toastEl.textContent = msg;
  toastEl.classList.remove('hidden');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.add('hidden'), 4000);
}

function askConfirm(msg, okLabel) {
  return new Promise((resolve) => {
    $('#confirm-msg').textContent = msg;
    $('#confirm-ok').textContent = okLabel;
    confirmDialog.addEventListener('close', () => resolve(confirmDialog.returnValue === 'ok'), { once: true });
    confirmDialog.showModal();
  });
}

// Só http(s) sai do app: outros esquemas são ignorados.
function isWebUrl(u) {
  return typeof u === 'string' && /^https?:\/\//i.test(u);
}

function isTorrentStream(stream) {
  const url = String(stream?.url ?? '').trim();
  return url.startsWith('magnet:?') || Boolean(stream?.infoHash) || Boolean(stream?.hash);
}

function magnetFromInfoHash(hash, name = 'CineHub') {
  const clean = String(hash ?? '').trim();
  if (!clean) return '';
  return `magnet:?xt=urn:btih:${clean}&dn=${encodeURIComponent(name)}`;
}

async function openTorrentSource(stream, label, id, meta, subs) {
  const currentUrl = String(stream?.url ?? '').trim();
  const magnet = currentUrl.startsWith('magnet:?')
    ? currentUrl
    : magnetFromInfoHash(stream?.infoHash ?? stream?.hash ?? '', label || meta?.name || 'CineHub');
  if (!magnet) {
    toast('Fonte torrent inválida.');
    return;
  }

  try {
    toast('Preparando torrent...');
    const result = await call('torrent_open', {
      magnet,
      torrent_bytes: null,
      file_index: stream?.fileIndex ?? null,
    });
    const fixed = { ...stream, url: result.streamUrl, mimeType: result.mimeType, name: result.name || stream?.name || label };
    await play(fixed, label, id, meta, subs);
  } catch (err) {
    toast(err.message || String(err));
  }
}

// ---------- Páginas ----------

async function homePage(alive) {
  setView(`<h1>Início</h1><div id="rows">${skeleton()}${skeleton()}</div>`);
  const [m, s] = await Promise.all([call('home', { kind: 'movie' }), call('home', { kind: 'series' })]);
  if (!alive()) return;
  const cont = continueWatchingRow();
  renderRows([...(cont ? [cont] : []), ...m.rows, ...s.rows], 'Nenhum catálogo disponível. Instale addons na aba Addons.');
}

async function catalogPage(kind, title, alive) {
  setView(`<h1>${title}</h1><div id="rows">${skeleton()}${skeleton()}</div>`);
  const { rows } = await call('home', { kind });
  if (!alive()) return;
  renderRows(rows, 'Nenhum catálogo disponível para este tipo.');
}

// Pesquisa: a busca roda enquanto o usuário digita (com espera de 450 ms) e ignora respostas antigas.
let debounceTimer = null;
let searchSeq = 0;

function searchPage(q) {
  setView(`
    <h1>Pesquisar</h1>
    <form id="sf" role="search"><input id="sq" type="search" value="${esc(q)}" placeholder="Digite um título…" autocomplete="off" aria-label="Pesquisar título"></form>
    <div id="rows"></div>`);
  const input = $('#sq');
  $('#sf').onsubmit = (e) => {
    e.preventDefault();
    clearTimeout(debounceTimer);
    setSearch(input.value);
  };
  input.addEventListener('input', () => {
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => setSearch(input.value), 450);
  });
  runSearch(q.trim());
}

// Atualiza a busca: na página de pesquisa troca só a URL e refaz a busca; em outras páginas, navega.
function setSearch(raw) {
  const q = raw.trim();
  if (sectionOf() !== 'search') {
    location.hash = '#/search?q=' + encodeURIComponent(q);
    return;
  }
  history.replaceState(null, '', '#/search?q=' + encodeURIComponent(q));
  const sq = $('#sq');
  if (sq && document.activeElement !== sq) sq.value = q;
  if (document.activeElement !== topQ) topQ.value = q;
  runSearch(q);
}

async function runSearch(q) {
  const seq = ++searchSeq;
  const stillHere = () => seq === searchSeq && sectionOf() === 'search';
  const box = $('#rows');
  if (!box) return;
  if (!q) {
    box.innerHTML = '<p class="muted">Digite um título para pesquisar.</p>';
    return;
  }
  box.innerHTML = skeleton(4);
  try {
    const { rows } = await call('search', { q });
    if (stillHere()) renderRows(rows, 'Nenhum resultado. Verifique se há addons de catálogo com busca instalados.', { pageable: false });
  } catch (e) {
    if (stillHere()) box.innerHTML = `<p class="muted">Erro: ${esc(e.message)}</p>`;
  }
}

function libraryPage() {
  const favs = getFavs();
  setView(`<h1>Biblioteca</h1>
    <div class="dataactions">
      <button id="export-data" class="chip">Exportar dados</button>
      <button id="import-data" class="chip">Importar dados</button>
      <input id="import-file" type="file" accept="application/json,.json" hidden>
    </div>
    <p class="muted small">Biblioteca e progresso ficam neste aparelho. Exporte um arquivo para levá-los a outro.</p>${
    favs.length
      ? `<div class="grid">${favs.map(cardHtml).join('')}</div>`
      : '<p class="muted">Sua biblioteca está vazia. Abra um título e clique em “Adicionar à biblioteca”.</p>'
  }`);
  $('#export-data').onclick = exportData;
  $('#import-data').onclick = () => $('#import-file').click();
  $('#import-file').onchange = (e) => {
    const file = e.target.files[0];
    e.target.value = '';
    importData(file);
  };
}

// Exporta biblioteca e progresso em um arquivo JSON.
function exportData() {
  const payload = {
    app: 'CineHub',
    version: 1,
    exportedAt: new Date().toISOString(),
    favs: getFavs(),
    progress: store.get('progress', {}),
  };
  const url = URL.createObjectURL(new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = 'cinehub-dados.json';
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

const MAX_IMPORT_BYTES = 2 * 1024 * 1024;
const MAX_FAVS = 500;
const str = (v) => (typeof v === 'string' ? v : '');
const num = (v) => (Number.isFinite(Number(v)) ? Number(v) : 0);

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
  };
}

// Importa um backup: junta favoritos sem repetir e, no progresso, fica o registro mais recente.
async function importData(file) {
  if (!file) return;
  if (file.size > MAX_IMPORT_BYTES) {
    toast('Arquivo grande demais para importar.');
    return;
  }
  try {
    const data = JSON.parse(await file.text());
    if (data?.app !== 'CineHub' || typeof data.version !== 'number') throw new Error('o arquivo não é um backup do CineHub');

    const favs = getFavs();
    const ids = new Set(favs.map((f) => f.id));
    for (const f of Array.isArray(data.favs) ? data.favs : []) {
      if (f && typeof f.id === 'string' && f.id && !ids.has(f.id)) {
        favs.push(cleanFav(f));
        ids.add(f.id);
      }
    }
    store.set('favs', favs.slice(0, MAX_FAVS));

    const progress = store.get('progress', {});
    for (const [key, p] of Object.entries(data.progress && typeof data.progress === 'object' ? data.progress : {})) {
      if (!p || typeof p !== 'object') continue;
      const incoming = cleanProgress(p);
      if (!progress[key] || incoming.updated > num(progress[key].updated)) progress[key] = incoming;
    }
    const kept = Object.entries(progress)
      .sort((a, b) => num(b[1].updated) - num(a[1].updated))
      .slice(0, MAX_PROGRESS);
    store.set('progress', Object.fromEntries(kept));

    toast('Dados importados.');
    libraryPage();
  } catch (err) {
    toast(`Não consegui importar: ${err.message}`);
  }
}

async function addonsPage() {
  setView(`
    <h1>Addons</h1>
    <form id="af" class="addform">
      <input id="au" placeholder="Cole o link do addon (manifest.json ou stremio://…)" aria-label="Link do addon">
      <button>Instalar</button>
    </form>
    <form id="tf" class="torrentform">
      <input id="tm" placeholder="Cole um magnet: ou use um arquivo .torrent" aria-label="Magnet do torrent">
      <label class="torrentfile">
        <span>Selecionar .torrent</span>
        <input id="torrent-file" type="file" accept=".torrent" hidden>
      </label>
      <button type="submit" class="chip">Abrir torrent</button>
    </form>
    <p class="muted small">Addons compatíveis com o protocolo do Stremio. Instale apenas fontes que você tem autorização para acessar.</p>
    <div id="al" class="loading">Carregando…</div>`);

  $('#af').onsubmit = async (e) => {
    e.preventDefault();
    const url = $('#au').value;
    try {
      const res = await call('install_addon', { url });
      toast(`Addon instalado: ${res.name}`);
      addonsPage();
    } catch (err) {
      toast(err.message);
    }
  };

  $('#tf').onsubmit = async (e) => {
    e.preventDefault();
    const input = $('#torrent-file');
    const magnet = $('#tm').value.trim();
    try {
      if (input.files && input.files.length) {
        const file = input.files[0];
        const bytes = new Uint8Array(await file.arrayBuffer());
        const result = await call('torrent_open', { magnet: null, torrent_bytes: Array.from(bytes), file_index: null });
        const streamUrl = result.streamUrl;
        const fake = { url: streamUrl, name: result.name, fileName: result.fileName, mimeType: result.mimeType };
        const playback = { id: `torrent:${Date.now()}`, label: result.name || file.name, name: result.name || file.name };
        await play(fake, playback.label, playback.id, { ...playback, type: 'movie', name: playback.label }, []);
        return;
      }
      if (!magnet) {
        toast('Cole um magnet ou selecione um .torrent.');
        return;
      }
      const result = await call('torrent_open', { magnet, torrent_bytes: null, file_index: null });
      const fake = { url: result.streamUrl, name: result.name, fileName: result.fileName, mimeType: result.mimeType };
      const playback = { id: `torrent:${Date.now()}`, label: result.name || 'Torrent', name: result.name || 'Torrent' };
      await play(fake, playback.label, playback.id, { ...playback, type: 'movie', name: playback.label }, []);
    } catch (err) {
      toast(err.message || String(err));
    }
  };

  const list = await call('list_addons');
  const al = $('#al');
  if (!al) return;
  al.className = '';
  al.innerHTML = list.length ? list.map(addonHtml).join('') : '<p class="muted">Nenhum addon instalado.</p>';
  al.querySelectorAll('button[data-url]').forEach((b) => {
    b.onclick = async () => {
      const ok = await askConfirm('Remover este addon? Os catálogos e fontes dele deixam de aparecer.', 'Remover');
      if (!ok) return;
      try {
        await call('remove_addon', { url: b.dataset.url });
        addonsPage();
      } catch (err) {
        toast(err.message);
      }
    };
  });
}

async function detailPage(kind, id, alive) {
  setView(`<div class="hero skel-hero" aria-hidden="true"></div>`);
  const { meta } = await call('meta', { kind, id });
  if (!alive()) return;
  if (!meta) {
    setView('<p class="muted">Não encontrei informações deste título. Verifique se há um addon de metadados instalado.</p>');
    return;
  }

  const bg = meta.background || meta.poster || '';
  const info = [meta.releaseInfo, (meta.genres || []).join(', ')].filter(Boolean).join(' · ');
  setView(`
    <div class="hero" style="background-image: linear-gradient(90deg, var(--bg) 20%, transparent), url('${esc(bg)}')">
      <div class="hero-info">
        <h1>${esc(meta.name)}</h1>
        <p class="muted">${esc(info)}</p>
        <p>${esc(meta.description || '')}</p>
        <div class="actions"><button id="fav"></button></div>
      </div>
    </div>
    <div id="body"></div>`);

  const favBtn = $('#fav');
  const paintFav = () => (favBtn.textContent = isFav(meta.id) ? '★ Na biblioteca' : '☆ Adicionar à biblioteca');
  paintFav();
  favBtn.onclick = () => {
    toggleFav({ ...meta, type: meta.type || kind });
    paintFav();
  };

  if ((meta.type || kind) === 'series') renderSeries(meta);
  else loadStreams(meta.type || kind, meta.id, meta.name, meta);
}

// ---------- Componentes ----------

function renderRows(rows, emptyMsg, { pageable = true } = {}) {
  const box = $('#rows');
  if (!box) return;
  box.className = '';
  box.innerHTML = '';
  if (!rows.length) {
    box.innerHTML = `<p class="muted">${esc(emptyMsg)}</p>`;
    return;
  }
  for (const row of rows) box.append(rowEl(row, pageable && Boolean(row.transportUrl)));
}

// Um trilho de catálogo. Quando o catálogo pode ser paginado, mostra "Carregar mais".
function rowEl(row, pageable) {
  const sec = document.createElement('section');
  sec.className = 'row';
  sec.innerHTML = `<h2>${esc(row.title)} <span class="muted small">· ${esc(row.addon)}</span></h2>`;
  const strip = document.createElement('div');
  strip.className = 'strip';
  sec.append(strip);

  const metas = row.metas || [];
  if (!metas.length) {
    strip.innerHTML = `<p class="muted small">${esc(row.error || 'Sem itens')}</p>`;
    return sec;
  }

  const seen = new Set();
  const append = (list) => {
    let added = 0;
    for (const m of list) {
      const key = `${m.type}:${m.id}`;
      if (seen.has(key)) continue;
      seen.add(key);
      strip.insertAdjacentHTML('beforeend', cardHtml(m));
      added++;
    }
    return added;
  };
  append(metas);

  if (pageable && metas.length >= PAGE_HINT) {
    const more = document.createElement('button');
    more.className = 'chip more';
    more.textContent = 'Carregar mais';
    let skip = metas.length;
    more.onclick = async () => {
      more.disabled = true;
      try {
        const { metas: next = [] } = await call('catalog_page', {
          transportUrl: row.transportUrl,
          kind: row.type,
          catalogId: row.catalogId,
          skip,
        });
        const added = append(next);
        skip += next.length;
        if (!next.length || !added) more.remove();
        else more.disabled = false;
      } catch (e) {
        toast(e.message);
        more.disabled = false;
      }
    };
    sec.append(more);
  }
  return sec;
}

function cardHtml(m) {
  const poster = m.poster ? `background-image: url('${esc(m.poster)}')` : '';
  const label = [m.name, m.releaseInfo].filter(Boolean).join(', ');
  return `
    <a class="card" href="#/detail/${esc(m.type)}/${encodeURIComponent(m.id)}" aria-label="${esc(label)}">
      <div class="poster" style="${poster}">${m.poster ? '' : esc(m.name)}</div>
      <div class="title">${esc(m.name)}</div>
      <div class="muted small">${esc(m.releaseInfo || '')}</div>
    </a>`;
}

function addonHtml(a) {
  const m = a.manifest;
  const resources = (m.resources || []).map((r) => (typeof r === 'string' ? r : r.name)).join(', ');
  return `
    <div class="addon">
      ${m.logo ? `<img src="${esc(m.logo)}" alt="">` : '<img alt="">'}
      <div>
        <strong>${esc(m.name)}</strong> <span class="muted small">v${esc(m.version)}</span>
        <p class="muted small">${esc(m.description || '')}</p>
        <p class="small">Recursos: ${esc(resources)} · Tipos: ${esc((m.types || []).join(', '))}</p>
      </div>
      <button class="danger" data-url="${esc(a.transportUrl)}">Remover</button>
    </div>`;
}

function renderSeries(meta) {
  const videos = (meta.videos || []).filter((v) => v.id);
  const seasons = [...new Set(videos.map((v) => v.season ?? 0))].sort((a, b) => a - b);
  let season = seasons.find((s) => s > 0) ?? seasons[0];
  const body = $('#body');

  const draw = () => {
    const eps = videos
      .filter((v) => (v.season ?? 0) === season)
      .sort((a, b) => (a.episode ?? 0) - (b.episode ?? 0));

    body.innerHTML = `
      <div class="seasons">${seasons
        .map((s) => `<button class="chip ${s === season ? 'on' : ''}" data-s="${s}">${s === 0 ? 'Especiais' : 'Temporada ' + s}</button>`)
        .join('')}</div>
      <div class="episodes">${eps
        .map((e) => `<button class="ep" data-id="${esc(e.id)}">${e.episode ?? ''}. ${esc(e.title || e.name || 'Episódio ' + (e.episode ?? ''))}</button>`)
        .join('')}</div>
      <div id="streams"></div>`;

    body.querySelectorAll('.chip').forEach((b) => {
      b.onclick = () => {
        season = Number(b.dataset.s);
        draw();
      };
    });
    body.querySelectorAll('.ep').forEach((b) => {
      b.onclick = () => {
        const ep = eps.find((e) => e.id === b.dataset.id);
        const label = `${meta.name} · T${season}E${ep?.episode ?? ''}`;
        loadStreams('series', b.dataset.id, label, { ...meta, type: 'series', seriesId: meta.id, poster: meta.poster, name: label });
      };
    });
  };
  draw();
}

// Fontes do título. Respostas de cliques antigos (outro episódio) são descartadas.
let streamsSeq = 0;
async function loadStreams(kind, id, label, meta) {
  const seq = ++streamsSeq;
  const first = $('#streams') || $('#body');
  if (!first) return;
  first.innerHTML = '<div class="loading">Buscando fontes nos addons…</div>';

  const { streams, subtitles } = await call('streams', { kind, id });
  if (seq !== streamsSeq) return;
  const box = $('#streams') || $('#body');
  if (!box) return;
  box.innerHTML = '';

  if (!streams.length) {
    box.innerHTML = '<p class="muted">Nenhuma fonte encontrada. Instale um addon de streams compatível na aba Addons.</p>';
    return;
  }

  const title = document.createElement('h3');
  title.textContent = 'Fontes';
  box.append(title);

  for (const s of streams) {
    const playable = Boolean(s.url || s.externalUrl || s.infoHash || s.hash || String(s.url || '').startsWith('magnet:?'));
    const b = document.createElement('button');
    b.className = 'stream' + (playable ? '' : ' disabled');
    b.disabled = !playable;
    const details = [s.addonName, s.note].filter(Boolean).join(' · ');
    b.innerHTML = `
      <strong>${esc(s.name || s.addonName)}</strong>
      <span>${esc((s.title || '').replace(/\n/g, ' · '))}</span>
      <em class="muted small">${esc(details)}</em>`;
    if (playable && (isTorrentStream(s) || s.url)) {
      b.onclick = async () => {
        if (isTorrentStream(s)) {
          await openTorrentSource(s, label, id, meta, subtitles || []);
          return;
        }
        await play(s, label, id, meta, subtitles || []);
      };
    } else if (isWebUrl(s.externalUrl)) {
      b.onclick = () => window.open(s.externalUrl, '_blank', 'noopener');
    }
    box.append(b);
  }
}

// ---------- Player (Video.js, com HLS embutido) ----------

const vjsPlayer = videojs(video, { controls: true, fluid: true, preload: 'auto', playsinline: true });
let current = null; // { id, label, meta } do que está tocando agora
let playSeq = 0; // muda a cada fonte; legendas de fontes antigas são descartadas
let blobUrls = [];
let lastSave = 0;

async function play(stream, label, id, meta, subs) {
  if (isTorrentStream(stream)) {
    await openTorrentSource(stream, label, id, meta, subs);
    return;
  }

  saveProgress(true);
  const seq = ++playSeq;
  current = { id, label, meta };

  const src = { src: stream.url };
  if (/\.m3u8(\?|$)/i.test(stream.url)) src.type = 'application/x-mpegURL';
  clearTextTracks();
  vjsPlayer.src(src);
  for (const sub of pickSubtitles(stream, subs)) addSubtitle(sub.url, seq);

  const saved = store.get('progress', {})[id];
  vjsPlayer.one('loadedmetadata', () => {
    if (seq !== playSeq) return;
    if (saved && saved.t > 5 && saved.t < vjsPlayer.duration() - 15) vjsPlayer.currentTime(saved.t);
  });

  document.title = `${label} · CineHub`;
  showPlayer();
  vjsPlayer.play().catch(() => {});
}

function showPlayer() {
  if (!history.state?.player) history.pushState({ player: true }, '', location.href);
  player.classList.remove('hidden');
}

// Legendas: as da própria fonte + as de addons de legenda, só em português.
function pickSubtitles(stream, subs) {
  const all = [...(stream.subtitles || []), ...(subs || [])];
  return all.filter((s) => /^(por|pt)/i.test(s.lang || '') && s.url).slice(0, 1);
}

function clearTextTracks() {
  const tracks = vjsPlayer.textTracks();
  for (let i = tracks.length - 1; i >= 0; i--) vjsPlayer.removeRemoteTextTrack(tracks[i]);
  blobUrls.forEach((u) => URL.revokeObjectURL(u));
  blobUrls = [];
}

async function addSubtitle(url, seq) {
  try {
    const vtt = await call('subtitle', { url });
    if (seq !== playSeq) return;
    const blobUrl = URL.createObjectURL(new Blob([vtt], { type: 'text/vtt' }));
    blobUrls.push(blobUrl);
    vjsPlayer.addRemoteTextTrack({ kind: 'subtitles', label: 'Português', srclang: 'pt', src: blobUrl, default: true }, false);
  } catch (e) {
    console.warn('legenda indisponível:', e);
  }
}

// Progresso: salva no máximo a cada 5 s (e sempre ao pausar ou fechar), guardando os 50 mais recentes.
function saveProgress(force) {
  if (!current) return;
  const now = Date.now();
  if (!force && now - lastSave < 5000) return;
  const t = vjsPlayer.currentTime() || 0;
  if (t < 5) return;
  lastSave = now;

  const duration = vjsPlayer.duration() || 0;
  const all = store.get('progress', {});
  all[current.id] = {
    t: Math.floor(t),
    duration: Math.floor(duration),
    label: current.label,
    seriesId: current.meta.seriesId || current.meta.id,
    type: current.meta.type,
    name: current.meta.name,
    poster: current.meta.poster || '',
    updated: now,
  };
  const kept = Object.entries(all)
    .sort((a, b) => (b[1].updated || 0) - (a[1].updated || 0))
    .slice(0, MAX_PROGRESS);
  store.set('progress', Object.fromEntries(kept));
}

vjsPlayer.on('timeupdate', () => saveProgress(false));
vjsPlayer.on('pause', () => saveProgress(true));
vjsPlayer.on('ended', () => {
  if (!current) return;
  const all = store.get('progress', {});
  delete all[current.id];
  store.set('progress', all);
});
vjsPlayer.on('error', () => {
  if (current) toast('Não foi possível tocar esta fonte. Tente outra.');
});

function closePlayer(fromHistory = false) {
  if (player.classList.contains('hidden')) return;
  saveProgress(true);
  vjsPlayer.pause();
  clearTextTracks();
  current = null;
  player.classList.add('hidden');
  document.title = 'CineHub';
  if (!fromHistory && history.state?.player) history.back();
}

// Fila de títulos com progresso parcial, do mais recente ao mais antigo.
function continueWatchingRow() {
  const items = Object.values(store.get('progress', {}))
    .filter((p) => p.duration > 0 && p.t > 5 && p.t < p.duration - 60)
    .sort((a, b) => b.updated - a.updated)
    .slice(0, 20);
  if (!items.length) return null;
  return {
    title: 'Continuar assistindo',
    addon: 'CineHub',
    metas: items.map((p) => ({
      id: p.seriesId,
      type: p.type,
      name: p.name,
      poster: p.poster,
      releaseInfo: p.label,
    })),
  };
}

$('#close-player').onclick = () => closePlayer(false);
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !player.classList.contains('hidden')) closePlayer(false);
});
window.addEventListener('popstate', () => {
  if (!player.classList.contains('hidden')) closePlayer(true);
});

// ---------- Biblioteca (favoritos no navegador) ----------

function getFavs() {
  const list = store.get('favs', []);
  return Array.isArray(list) ? list : [];
}

function isFav(id) {
  return getFavs().some((f) => f.id === id);
}

function toggleFav(meta) {
  const list = getFavs();
  const i = list.findIndex((f) => f.id === meta.id);
  if (i >= 0) list.splice(i, 1);
  else
    list.unshift({
      id: meta.id,
      type: meta.type,
      name: meta.name,
      poster: meta.poster || '',
      releaseInfo: meta.releaseInfo || '',
    });
  store.set('favs', list);
}

// ---------- Roteamento ----------

const NAV = {
  home: (alive) => homePage(alive),
  movies: (alive) => catalogPage('movie', 'Filmes', alive),
  series: (alive) => catalogPage('series', 'Séries', alive),
  search: () => searchPage(new URLSearchParams(location.hash.split('?')[1] || '').get('q') || ''),
  library: () => libraryPage(),
  addons: () => addonsPage(),
};

let routeSeq = 0;
async function route() {
  const seq = ++routeSeq;
  const alive = () => seq === routeSeq;
  clearTimeout(debounceTimer);

  const path = location.hash.replace(/^#\/?/, '').split('?')[0];
  const [section, kind, id] = path.split('/');
  const active = section === 'detail' ? '' : section || 'home';
  document.querySelectorAll('[data-nav]').forEach((a) => {
    const on = a.dataset.nav === active;
    a.classList.toggle('active', on);
    if (on) a.setAttribute('aria-current', 'page');
    else a.removeAttribute('aria-current');
  });

  try {
    if (section === 'detail') await detailPage(kind, decodeURIComponent(id || ''), alive);
    else await (NAV[section || 'home'] || NAV.home)(alive);
  } catch (err) {
    if (alive()) showError(err);
  }
  if (alive()) focusContent();
}

let usingKeys = false;
document.addEventListener('keydown', () => (usingKeys = true), true);
document.addEventListener('pointerdown', () => (usingKeys = false), true);

function focusContent() {
  if (!usingKeys) return;
  const a = document.activeElement;
  if (a && a !== document.body && !a.closest('.sidebar')) return;
  const first = view.querySelector('.strip a, #rows a, #view a[href], #view button:not([disabled])');
  first?.focus({ preventScroll: true });
}

topQ.addEventListener('input', () => {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => setSearch(topQ.value), 450);
});
topQ.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    clearTimeout(debounceTimer);
    setSearch(topQ.value);
  }
});

window.addEventListener('hashchange', route);
route();

const FOCUSABLE = 'a[href], button:not([disabled]), input, video';
const DIRS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };

function focusables() {
  return [...document.querySelectorAll(FOCUSABLE)].filter((el) => el.offsetParent !== null && !el.closest('.hidden'));
}

document.addEventListener('keydown', (e) => {
  const dir = DIRS[e.key];
  if (!dir || !player.classList.contains('hidden') || confirmDialog.open) return;

  const list = focusables();
  if (!list.length) return;
  const current = document.activeElement;
  e.preventDefault();

  if (!list.includes(current)) {
    list[0].focus();
    return;
  }

  const r = current.getBoundingClientRect();
  const cx = r.left + r.width / 2;
  const cy = r.top + r.height / 2;
  const horizontal = dir === 'left' || dir === 'right';

  let best = null;
  let bestScore = Infinity;
  for (const el of list) {
    if (el === current) continue;
    const q = el.getBoundingClientRect();
    const dx = q.left + q.width / 2 - cx;
    const dy = q.top + q.height / 2 - cy;
    const ahead = dir === 'right' ? dx > 4 : dir === 'left' ? dx < -4 : dir === 'down' ? dy > 4 : dy < -4;
    if (!ahead) continue;
    const main = horizontal ? Math.abs(dx) : Math.abs(dy);
    const cross = horizontal ? Math.abs(dy) : Math.abs(dx);
    const score = main + cross * 2;
    if (score < bestScore) {
      bestScore = score;
      best = el;
    }
  }

  if (best) {
    best.focus();
    best.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }
});
