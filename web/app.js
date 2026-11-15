// CineHub — interface web. Fala com o backend Tauri via invoke.

const $ = (s) => document.querySelector(s);
const view = $('#view');
const player = $('#player');
const video = $('#video');

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

const setView = (html) => (view.innerHTML = html);
const showError = (err) => setView(`<p class="muted">Erro: ${esc(err.message || err)}</p>`);

// ---------- Páginas ----------

async function homePage() {
  setView('<h1>Início</h1><div id="rows" class="loading">Carregando catálogos…</div>');
  const [m, s] = await Promise.all([call('home', { kind: 'movie' }), call('home', { kind: 'series' })]);
  renderRows([...m.rows, ...s.rows], 'Nenhum catálogo disponível. Instale addons na aba Addons.');
}

async function catalogPage(kind, title) {
  setView(`<h1>${title}</h1><div id="rows" class="loading">Carregando…</div>`);
  const { rows } = await call('home', { kind });
  renderRows(rows, 'Nenhum catálogo disponível para este tipo.');
}

async function searchPage(q) {
  setView(`
    <h1>Pesquisar</h1>
    <form id="sf"><input id="sq" value="${esc(q)}" placeholder="Digite um título…" autofocus></form>
    <div id="rows"></div>`);
  $('#sf').onsubmit = (e) => {
    e.preventDefault();
    location.hash = '#/search?q=' + encodeURIComponent($('#sq').value.trim());
  };
  if (!q) return;
  $('#rows').innerHTML = '<div class="loading">Pesquisando…</div>';
  const { rows } = await call('search', { q });
  renderRows(rows, 'Nenhum resultado. Verifique se há addons de catálogo com busca instalados.');
}

function libraryPage() {
  const favs = getFavs();
  setView(`<h1>Biblioteca</h1>${
    favs.length
      ? `<div class="grid">${favs.map(cardHtml).join('')}</div>`
      : '<p class="muted">Sua biblioteca está vazia. Abra um título e clique em “Adicionar à biblioteca”.</p>'
  }`);
}

async function addonsPage() {
  setView(`
    <h1>Addons</h1>
    <form id="af" class="addform">
      <input id="au" placeholder="Cole o link do addon (manifest.json ou stremio://…)">
      <button>Instalar</button>
    </form>
    <p class="muted small">Addons compatíveis com o protocolo do Stremio. Instale apenas fontes que você tem autorização para acessar.</p>
    <div id="al" class="loading">Carregando…</div>`);

  $('#af').onsubmit = async (e) => {
    e.preventDefault();
    try {
      await call('install_addon', { url: $('#au').value });
      addonsPage();
    } catch (err) {
      alert(err.message);
    }
  };

  const list = await call('list_addons');
  $('#al').innerHTML = list.length
    ? list.map(addonHtml).join('')
    : '<p class="muted">Nenhum addon instalado.</p>';
  $('#al').querySelectorAll('button[data-url]').forEach((b) => {
    b.onclick = async () => {
      await call('remove_addon', { url: b.dataset.url });
      addonsPage();
    };
  });
}

async function detailPage(kind, id) {
  setView('<div class="loading">Carregando…</div>');
  const { meta } = await call('meta', { kind, id });
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
  else loadStreams('movie', meta.id, meta.name);
}

// ---------- Componentes ----------

function renderRows(rows, emptyMsg) {
  const box = $('#rows');
  box.className = '';
  box.innerHTML = '';
  if (!rows.length) {
    box.innerHTML = `<p class="muted">${esc(emptyMsg)}</p>`;
    return;
  }
  for (const row of rows) {
    const sec = document.createElement('section');
    sec.className = 'row';
    sec.innerHTML = `<h2>${esc(row.title)} <span class="muted small">· ${esc(row.addon)}</span></h2>`;
    const strip = document.createElement('div');
    strip.className = 'strip';
    if (row.metas && row.metas.length) strip.innerHTML = row.metas.map(cardHtml).join('');
    else strip.innerHTML = `<p class="muted small">${esc(row.error || 'Sem itens')}</p>`;
    sec.append(strip);
    box.append(sec);
  }
}

function cardHtml(m) {
  const poster = m.poster ? `background-image: url('${esc(m.poster)}')` : '';
  return `
    <a class="card" href="#/detail/${esc(m.type)}/${encodeURIComponent(m.id)}">
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
        loadStreams('series', b.dataset.id, `${meta.name} · T${season}E${ep?.episode ?? ''}`);
      };
    });
  };
  draw();
}

async function loadStreams(kind, id, label) {
  const box = $('#streams') || $('#body');
  box.innerHTML = '<div class="loading">Buscando fontes nos addons…</div>';
  const { streams } = await call('streams', { kind, id });
  box.innerHTML = '';

  if (!streams.length) {
    box.innerHTML = '<p class="muted">Nenhuma fonte encontrada. Instale um addon de streams compatível na aba Addons.</p>';
    return;
  }

  const title = document.createElement('h3');
  title.textContent = 'Fontes';
  box.append(title);

  for (const s of streams) {
    const playable = Boolean(s.url || s.externalUrl);
    const b = document.createElement('button');
    b.className = 'stream' + (playable ? '' : ' disabled');
    b.innerHTML = `
      <strong>${esc(s.name || s.addonName)}</strong>
      <span>${esc((s.title || '').replace(/\n/g, ' · '))}</span>
      <em class="muted small">${esc(s.addonName || '')}${playable ? '' : ' · formato não suportado neste app'}</em>`;
    if (s.url) b.onclick = () => play(s, label, id);
    else if (s.externalUrl) b.onclick = () => window.open(s.externalUrl, '_blank', 'noopener');
    box.append(b);
  }
}

// ---------- Player ----------

function play(stream, label, id) {
  video.pause();
  video.innerHTML = '';
  video.src = stream.url;

  const sub = (stream.subtitles || []).find((s) => /^(por|pt)/i.test(s.lang || ''));
  if (sub) addSubtitle(sub.url);

  // Retoma de onde parou, guardado no navegador.
  const key = 'pos:' + id;
  const saved = Number(localStorage.getItem(key) || 0);
  video.onloadedmetadata = () => {
    if (saved > 5 && saved < video.duration - 15) video.currentTime = saved;
  };
  video.ontimeupdate = () => {
    if (video.currentTime > 5) localStorage.setItem(key, String(Math.floor(video.currentTime)));
  };

  document.title = `${label} · CineHub`;
  player.classList.remove('hidden');
  video.play().catch(() => {});
}

async function addSubtitle(url) {
  try {
    const vtt = await call('subtitle', { url });
    const blobUrl = URL.createObjectURL(new Blob([vtt], { type: 'text/vtt' }));
    const t = document.createElement('track');
    t.kind = 'subtitles';
    t.label = 'Português';
    t.srclang = 'pt';
    t.default = true;
    t.src = blobUrl;
    video.append(t);
  } catch (e) {
    console.warn('legenda indisponível:', e);
  }
}

function closePlayer() {
  video.pause();
  video.removeAttribute('src');
  video.innerHTML = '';
  video.load();
  player.classList.add('hidden');
  document.title = 'CineHub';
}

$('#close-player').onclick = closePlayer;
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !player.classList.contains('hidden')) closePlayer();
});

// ---------- Biblioteca (favoritos no navegador) ----------

function getFavs() {
  try {
    return JSON.parse(localStorage.getItem('favs') || '[]');
  } catch {
    return [];
  }
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
  localStorage.setItem('favs', JSON.stringify(list));
}

// ---------- Roteamento ----------

const NAV = {
  home: homePage,
  movies: () => catalogPage('movie', 'Filmes'),
  series: () => catalogPage('series', 'Séries'),
  search: () => searchPage(new URLSearchParams(location.hash.split('?')[1] || '').get('q') || ''),
  library: libraryPage,
  addons: addonsPage,
};

async function route() {
  const path = location.hash.replace(/^#\/?/, '').split('?')[0];
  const [section, kind, id] = path.split('/');
  const active = section === 'detail' ? '' : section || 'home';
  document.querySelectorAll('[data-nav]').forEach((a) => a.classList.toggle('active', a.dataset.nav === active));

  try {
    if (section === 'detail') return await detailPage(kind, decodeURIComponent(id || ''));
    const page = NAV[section || 'home'] || homePage;
    await page();
  } catch (err) {
    showError(err);
  }
}

$('#q').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') location.hash = '#/search?q=' + encodeURIComponent(e.target.value.trim());
});

window.addEventListener('hashchange', route);
route();

// ---------- Controle remoto (Android TV) e teclado ----------
// Setas movem o foco para o elemento mais próximo na direção pressionada.
// OK/Enter ativa o elemento focado (o próprio navegador já faz isso em <a> e <button>).

const FOCUSABLE = 'a[href], button:not([disabled]), input, video';
const DIRS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };

function focusables() {
  return [...document.querySelectorAll(FOCUSABLE)].filter((el) => el.offsetParent !== null && !el.closest('.hidden'));
}

document.addEventListener('keydown', (e) => {
  const dir = DIRS[e.key];
  if (!dir || !player.classList.contains('hidden')) return; // com o player aberto, as setas controlam o vídeo

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
    const score = main + cross * 2; // prefere o que está alinhado na mesma faixa
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
