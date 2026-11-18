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

// Chamadas ao backend Tauri (funciona em desktop, Android e Android TV).
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    throw new Error(String(e));
  }
}

// Registro local de diagnóstico: fica no aparelho e só sai se o usuário exportar.
// Passe só a descrição do erro, nunca títulos ou dados pessoais (o backend também redige endereços).
function logEvent(level, message) {
  try {
    invoke('log_event', { level, message: String(message ?? '').slice(0, 500) }).catch(() => {});
  } catch {
    /* sem backend (ex.: pré-visualização): nada a registrar */
  }
}
window.addEventListener('error', (e) => logEvent('ERROR', `erro de script: ${e.message || 'desconhecido'}`));
window.addEventListener('unhandledrejection', (e) => logEvent('ERROR', `promessa rejeitada: ${describeError(e.reason)}`));

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
  setMany(entries) {
    return writeStorageBatch(localStorage, entries);
  },
};

const setView = (html) => {
  view.classList.remove('detail-with-panel');
  view.innerHTML = html;
};
const showError = (err) => setView(`<p class="muted">Erro: ${esc(err.message || err)}</p>`);
const skeleton = (n = 6) => `<div class="strip" aria-hidden="true">${'<div class="card"><div class="poster skel"></div></div>'.repeat(n)}</div>`;
const sectionOf = () => location.hash.replace(/^#\/?/, '').split('?')[0].split('/')[0] || 'home';

// Avisos e confirmações dentro do app (substituem alert/confirm do navegador).
let toastTimer;
function toast(msg, ms = 4000) {
  toastEl.textContent = msg;
  toastEl.classList.remove('hidden');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.add('hidden'), ms);
}

// Aviso de conexão: aparece quando o aparelho fica sem rede.
function syncOnline() {
  $('#offline')?.classList.toggle('hidden', navigator.onLine !== false);
}
window.addEventListener('online', syncOnline);
window.addEventListener('offline', syncOnline);
syncOnline();

// Menu lateral recolhível no desktop (a preferência fica guardada neste aparelho).
function applySidebar(collapsed) {
  document.body.classList.toggle('nav-collapsed', collapsed);
  const btn = $('#toggle-sidebar');
  if (!btn) return;
  btn.setAttribute('aria-expanded', String(!collapsed));
  btn.title = collapsed ? 'Expandir menu' : 'Recolher menu';
}
$('#toggle-sidebar').onclick = () => {
  const collapsed = !document.body.classList.contains('nav-collapsed');
  store.set('sidebarCollapsed', collapsed);
  applySidebar(collapsed);
};
applySidebar(store.get('sidebarCollapsed', false) === true);

// Barra superior: transparente com gradiente sobre o destaque; sólida depois de rolar a página.
const topbarEl = document.querySelector('.sidebar');
const syncTopbar = () => topbarEl && topbarEl.classList.toggle('solid', window.scrollY > 40);
window.addEventListener('scroll', syncTopbar, { passive: true });
syncTopbar();

// Indicador persistente de trabalho em andamento (ex.: abrir um torrent leva até 1 minuto).
// Fica visível até a operação terminar, com o tempo decorrido; não depende de toque, então
// funciona também com controle remoto de TV.
const busyEl = $('#busy');
const busyTasks = [];
let busyTimer = null;
function renderBusy() {
  if (!busyEl || !busyTasks.length) return;
  // Mostra a operação mais recente; a anterior continua contando até terminar.
  const task = busyTasks[busyTasks.length - 1];
  const secs = Math.floor((Date.now() - task.started) / 1000);
  busyEl.innerHTML = `<span class="spinner" aria-hidden="true"></span><span>${esc(task.text)} <span class="muted small">${secs} s</span></span>`;
}
function withBusy(text, promise) {
  const task = { text: String(text), started: Date.now() };
  busyTasks.push(task);
  if (busyEl) {
    busyEl.classList.remove('hidden');
    busyEl.setAttribute('aria-busy', 'true');
  }
  renderBusy();
  if (busyTimer === null) busyTimer = setInterval(renderBusy, 1000);
  return Promise.resolve(promise).finally(() => {
    const index = busyTasks.indexOf(task);
    if (index !== -1) busyTasks.splice(index, 1);
    if (!busyTasks.length) {
      if (busyTimer !== null) clearInterval(busyTimer);
      busyTimer = null;
      busyEl?.classList.add('hidden');
      busyEl?.setAttribute('aria-busy', 'false');
    } else {
      renderBusy();
    }
  });
}
const TORRENT_BUSY = 'Preparando torrent (pode levar até 1 minuto)';

// Quem abre um diálogo recebe o foco de volta quando ele fecha (o controle remoto não se perde).
function focusReturner() {
  const el = document.activeElement;
  return () => {
    if (el && el !== document.body && el.isConnected) el.focus({ preventScroll: true });
  };
}

function askConfirm(msg, okLabel) {
  return new Promise((resolve) => {
    $('#confirm-msg').textContent = msg;
    $('#confirm-ok').textContent = okLabel;
    const back = focusReturner();
    confirmDialog.addEventListener(
      'close',
      () => {
        back();
        resolve(confirmDialog.returnValue === 'ok');
      },
      { once: true },
    );
    confirmDialog.showModal();
  });
}

// Funções puras (URLs seguras, ordenação de fontes, próximo episódio) vêm de lib.js, testadas com node.
const { isWebUrl, cssUrl, str, num, cleanFav, cleanProgress, mergeBackupData, createBackupData, writeStorageBatch, progressPercent, sortStreams, nextEpisode, torrentLanguage, isSeriesLike, typeLabel, qualityLabel, filterStreams, sortStreamsBy, isMkvStream } = window.CineHubLib;

const STREAM_LANG = { pt: 'Português', en: 'Inglês', other: 'Outros' };
const STREAM_SORTS = [
  ['default', 'Ordem padrão'],
  ['quality', 'Maior qualidade'],
  ['pt', 'Português primeiro'],
];
const streamText = (s) => `${s.name || ''} ${s.title || ''}`;

// Abre um link externo no navegador do aparelho (window.open não funciona no WebView do Android).
async function openExternal(url) {
  try {
    await call('plugin:opener|open_url', { url });
  } catch (err) {
    toast(`Não consegui abrir o link: ${err.message}`);
  }
}

function isLocalTorrentStream(stream) {
  return window.CineHubLib.isLocalTorrentStream(stream);
}

function isTorrentStream(stream) {
  return window.CineHubLib.isTorrentSource(stream);
}

function magnetFromInfoHash(hash, name = 'CineHub') {
  const clean = String(hash ?? '').trim();
  if (!clean) return '';
  return `magnet:?xt=urn:btih:${clean}&dn=${encodeURIComponent(name)}`;
}

const closingTorrentTokens = new Map();

async function closeTorrentToken(token) {
  if (!token) return false;
  // Reutiliza a mesma operação se vários eventos tentarem fechar a sessão ao mesmo tempo.
  const existing = closingTorrentTokens.get(token);
  if (existing) return existing;

  const closing = (async () => {
    let lastError = null;
    // Falhas transitórias de IPC/engine não devem descartar o único token capaz de fechar a sessão.
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        await call('torrent_close', { token });
        if (activeTorrentToken === token) activeTorrentToken = null;
        return true;
      } catch (err) {
        lastError = err;
        if (attempt < 2) await new Promise((resolve) => setTimeout(resolve, 250 * (attempt + 1)));
      }
    }
    logEvent('WARN', `encerrar torrent falhou após 3 tentativas: ${lastError?.message || String(lastError)}`);
    return false;
  })();

  closingTorrentTokens.set(token, closing);
  try {
    return await closing;
  } finally {
    if (closingTorrentTokens.get(token) === closing) closingTorrentTokens.delete(token);
  }
}

async function closeActiveTorrent() {
  return closeTorrentToken(activeTorrentToken);
}

async function openTorrentSource(stream, label, id, meta, subs) {
  let openedToken = null;
  const currentUrl = String(stream?.url ?? '').trim();
  const magnet = currentUrl.startsWith('magnet:?')
    ? currentUrl
    : magnetFromInfoHash(stream?.infoHash ?? stream?.hash ?? '', label || meta?.name || 'CineHub');
  if (!magnet) {
    toast('Fonte torrent inválida.');
    return;
  }

  try {
    const result = await withBusy(
      TORRENT_BUSY,
      call('torrent_open', {
        magnet,
        torrentBytes: null,
        fileIndex: Number.isInteger(stream?.fileIndex) ? stream.fileIndex : null,
      }),
    );
    openedToken = result.token;
    activeTorrentToken = result.token;
    const chosen = await pickTorrentVideo(result);
    // A seleção de episódio pode devolver um token novo para isolar leituras HTTP antigas.
    openedToken = chosen.token || result.token;
    activeTorrentToken = openedToken;
    const fixed = {
      ...stream,
      url: chosen.streamUrl,
      mimeType: chosen.mimeType,
      name: chosen.name || stream?.name || label,
      torrentToken: openedToken,
    };
    await play(fixed, label, id, meta, subs);
  } catch (err) {
    if (openedToken && activeTorrentToken === openedToken) await closeTorrentToken(openedToken);
    logEvent('WARN', `abrir torrent falhou: ${err.message || String(err)}`);
    toast(err.message || String(err));
  }
}

// Torrent com vários vídeos (episódios ou versões em idiomas diferentes): deixa o usuário escolher.
// Devolve o resultado do arquivo escolhido, ou o padrão se ele cancelar.
function pickTorrentVideo(result) {
  const videos = Array.isArray(result?.videos) ? result.videos : [];
  if (videos.length < 2 || !result.token) return Promise.resolve(result);
  const LANG = { pt: 'Português', en: 'Inglês', other: 'Outros' };
  const present = new Set(videos.map((v) => torrentLanguage(v.name)));
  const langs = ['pt', 'en', 'other'].filter((l) => present.has(l));
  const size = (bytes) => (bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`);
  const dlg = document.createElement('dialog');
  dlg.className = 'torrent-pick';
  dlg.setAttribute('aria-labelledby', 'tp-title');
  dlg.innerHTML = `<form method="dialog" class="dialog-box">
    <h2 id="tp-title">Escolha o arquivo</h2>
    ${langs.length > 1 ? `<label class="small">Idioma <select id="tp-lang">
      <option value="all">Todos</option>${langs.map((l) => `<option value="${l}">${LANG[l]}</option>`).join('')}
    </select></label>` : ''}
    <ul id="tp-list" class="tp-list"></ul>
    <div class="dialog-actions"><button value="cancel" class="chip">Cancelar</button></div>
  </form>`;
  const back = focusReturner();
  document.body.append(dlg);
  return new Promise((resolve) => {
    let chosen = null;
    const list = dlg.querySelector('#tp-list');
    const select = dlg.querySelector('#tp-lang');
    // Começa no português, quando o torrent tem.
    if (select) select.value = present.has('pt') ? 'pt' : 'all';
    const fill = () => {
      const lang = select ? select.value : 'all';
      list.innerHTML = '';
      for (const v of videos.filter((x) => lang === 'all' || torrentLanguage(x.name) === lang)) {
        const li = document.createElement('li');
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'chip tp-item';
        btn.textContent = `${v.name.split(/[\\/]/).pop()} · ${size(v.sizeBytes)}`;
        btn.onclick = async () => {
          btn.disabled = true;
          try {
            chosen = await call('torrent_select_file', { token: result.token, fileIndex: v.index });
            dlg.close();
          } catch (err) {
            btn.disabled = false;
            logEvent('WARN', `escolher arquivo do torrent falhou: ${err.message || String(err)}`);
            toast(err.message || String(err));
          }
        };
        li.append(btn);
        list.append(li);
      }
    };
    if (select) select.onchange = fill;
    fill();
    dlg.addEventListener(
      'close',
      () => {
        dlg.remove();
        back();
        resolve(chosen || result);
      },
      { once: true },
    );
    dlg.showModal();
    // No Android TV o foco inicial do dialog pode cair no filtro de idioma.
    // Leva-o diretamente ao primeiro arquivo para que o DPAD percorra os episódios.
    requestAnimationFrame(() => {
      const firstItem = dlg.querySelector('.tp-item');
      if (firstItem && dlg.open) firstItem.focus({ preventScroll: true });
    });
  });
}

// Reproduz um torrent recém-aberto pelo backend (magnet ou arquivo .torrent).
async function playOpenedTorrent(input, fallbackName) {
  let openedToken = input.token || null;
  activeTorrentToken = openedToken;
  try {
    const result = await pickTorrentVideo(input);
    openedToken = result.token || openedToken;
    activeTorrentToken = openedToken;
    const name = result.name || fallbackName;
    const id = `torrent:${Date.now()}`;
    const fake = {
      url: result.streamUrl,
      name,
      fileName: result.fileName,
      mimeType: result.mimeType,
      torrentToken: openedToken,
    };
    await play(fake, name, id, { id, label: name, name, type: 'movie' }, []);
  } catch (err) {
    if (openedToken && activeTorrentToken === openedToken) await closeTorrentToken(openedToken);
    throw err;
  }
}

// ---------- Páginas ----------

async function homePage(alive) {
  setView(`<div id="billboard"></div><div id="rows">${skeleton()}${skeleton()}</div>`);
  // Um tipo que falha não derruba o outro: usa o que respondeu.
  const [m, s, a] = await Promise.allSettled([call('home', { kind: 'movie' }), call('home', { kind: 'series' }), call('home', { kind: 'anime' })]);
  if (!alive()) return;
  const rowsOf = (r) => (r.status === 'fulfilled' ? r.value.rows : []);
  const cont = continueWatchingRow();
  const rows = [...(cont ? [cont] : []), ...rowsOf(m), ...rowsOf(s), ...rowsOf(a)];
  renderRows(rows, 'Nenhum catálogo disponível. Instale addons na aba Addons.');
  renderBillboard(rows);
}

// Destaque no topo da tela inicial: até 6 títulos com imagem, em rodízio (como o destaque do dxbi).
// Troca a cada 9 s; pausa com o mouse ou o foco dentro dele, e quando a aba está oculta.
const BILLBOARD_INTERVAL_MS = 9000;
let billboardTimer = null;

function renderBillboard(rows) {
  const box = $('#billboard');
  clearInterval(billboardTimer);
  if (!box) return;
  const seen = new Set();
  const items = rows
    .flatMap((r) => r.metas || [])
    .filter((x) => (isWebUrl(x.background) || isWebUrl(x.poster)) && !seen.has(`${x.type}:${x.id}`) && seen.add(`${x.type}:${x.id}`))
    .slice(0, 6);
  if (!items.length) {
    box.remove();
    return;
  }
  const slideHtml = (m, i) => {
    const bg = isWebUrl(m.background) ? m.background : m.poster;
    const kindLabel = typeLabel(m.type) || 'Filme';
    const info = [m.releaseInfo, kindLabel].filter(Boolean).join(' · ');
    const href = `#/detail/${esc(m.type)}/${encodeURIComponent(m.id)}`;
    return `
      <div class="billboard-slide${i === 0 ? ' is-active' : ''}" data-i="${i}" style="background-image: ${cssUrl(bg)}">
        <div class="billboard-info">
          <h2>${esc(m.name)}</h2>
          <p class="meta-line">${esc(info)}</p>
          ${m.description ? `<p>${esc(m.description)}</p>` : ''}
          <div class="actions">
            <a class="btn-link btn-light" href="${href}">▶ Assistir</a>
            <a class="btn-link btn-grey" href="${href}">ⓘ Mais informações</a>
          </div>
        </div>
      </div>`;
  };
  box.className = 'billboard';
  box.setAttribute('aria-roledescription', 'destaques');
  box.setAttribute('aria-label', 'Destaques');
  box.innerHTML = `
    ${items.map(slideHtml).join('')}
    <div class="billboard-dots" role="group" aria-label="Escolher destaque">
      ${items.map((m, i) => `<button class="billboard-dot${i === 0 ? ' is-active' : ''}" type="button" data-i="${i}" aria-label="${esc(m.name)}"></button>`).join('')}
    </div>`;

  const slides = [...box.querySelectorAll('.billboard-slide')];
  const dots = [...box.querySelectorAll('.billboard-dot')];
  let index = 0;
  const show = (i) => {
    index = (i + slides.length) % slides.length;
    slides.forEach((el, k) => el.classList.toggle('is-active', k === index));
    dots.forEach((el, k) => {
      el.classList.toggle('is-active', k === index);
      el.setAttribute('aria-current', k === index ? 'true' : 'false');
    });
  };
  box.addEventListener('click', (e) => {
    const dot = e.target.closest('.billboard-dot');
    if (dot) show(Number(dot.dataset.i));
  });
  // Respeita a preferência do sistema: com movimento reduzido, mantém o primeiro destaque estático.
  if (slides.length < 2 || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
  billboardTimer = setInterval(() => {
    if (!box.isConnected) {
      clearInterval(billboardTimer);
      return;
    }
    const paused = document.hidden || box.matches(':hover') || box.contains(document.activeElement);
    const onHome = location.hash === '' || location.hash === '#' || location.hash === '#/';
    if (!paused && onHome) show(index + 1);
  }, BILLBOARD_INTERVAL_MS);
}

async function catalogPage(kind, title, alive) {
  if (kind === 'anime') {
    setView('<h1>Animes</h1><form id="anime-search-form" role="search"><input id="anime-search-input" type="search" placeholder="Pesquisar animes…" autocomplete="off" aria-label="Pesquisar animes"></form><p id="anime-search-status" class="muted small" role="status" aria-live="polite">Pesquise para encontrar títulos nos addons de anime.</p><div id="anime-search-results"></div><h2 class="section-heading">Catálogos</h2><div id="rows">' + skeleton() + skeleton() + '</div>');
    const input = $('#anime-search-input');
    const submit = (event) => { event?.preventDefault(); clearTimeout(debounceTimer); runAnimeSearch(input.value.trim()); };
    $('#anime-search-form').onsubmit = submit;
    input.addEventListener('input', () => { clearTimeout(debounceTimer); debounceTimer = setTimeout(() => runAnimeSearch(input.value.trim()), 450); });
    const { rows } = await call('home', { kind });
    if (!alive()) return;
    renderRows(rows, 'Nenhum catálogo disponível. Use a pesquisa acima para consultar addons que exigem um termo de busca.');
    return;
  }
  setView(`<h1>${title}</h1><div id="rows">${skeleton()}${skeleton()}</div>`);
  const { rows } = await call('home', { kind });
  if (!alive()) return;
  renderRows(rows, 'Nenhum catálogo disponível para este tipo.');
}

let animeSearchSeq = 0;
async function runAnimeSearch(q) {
  const seq = ++animeSearchSeq;
  const stillHere = () => seq === animeSearchSeq && sectionOf() === 'animes';
  const box = $('#anime-search-results');
  const status = $('#anime-search-status');
  if (!box) return;
  if (!q) { box.innerHTML = ''; if (status) status.textContent = 'Pesquise para encontrar títulos nos addons de anime.'; return; }
  box.innerHTML = skeleton(3);
  if (status) status.textContent = 'Pesquisando animes…';
  try {
    const { rows = [] } = await call('search', { q, kind: 'anime' });
    if (!stillHere()) return;
    renderRows(rows, 'Nenhum anime encontrado para essa pesquisa.', { pageable: false, targetId: 'anime-search-results' });
    const total = rows.reduce((count, row) => count + (row.metas || []).length, 0);
    if (status) status.textContent = total ? `${total} resultado${total === 1 ? '' : 's'}` : 'Nenhum resultado';
  } catch (error) {
    if (!stillHere()) return;
    box.innerHTML = `<p class="muted">Erro na pesquisa: ${esc(error.message)}</p>`;
    if (status) status.textContent = 'Erro na pesquisa de animes';
  }
}
// Pesquisa: a busca roda enquanto o usuário digita (com espera de 450 ms) e ignora respostas antigas.
let debounceTimer = null;
let searchSeq = 0;

function searchPage(q) {
  setView(`
    <h1>Pesquisar</h1>
    <form id="sf" role="search"><input id="sq" type="search" value="${esc(q)}" placeholder="Digite um título…" autocomplete="off" aria-label="Pesquisar título" aria-controls="rows"></form>
    <p id="search-status" class="muted small" role="status" aria-live="polite"></p>
    <div id="rows"></div>`);
  const input = $('#sq');
  // Esc limpa o campo (útil também com controle remoto, que não tem botão de limpar).
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && input.value) {
      e.preventDefault();
      input.value = '';
      clearTimeout(debounceTimer);
      setSearch('');
    }
  });
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
    const status = $('#search-status');
    if (status) status.textContent = '';
    return;
  }
  box.innerHTML = skeleton(4);
  box.setAttribute('aria-busy', 'true');
  const status = $('#search-status');
  if (status) status.textContent = 'Pesquisando…';
  try {
    const { rows } = await call('search', { q });
    if (stillHere()) {
      renderRows(rows, 'Nenhum resultado. Verifique se há addons de catálogo com busca instalados.', { pageable: false });
      // Quantidade anunciada para quem usa leitor de tela (o texto também fica visível).
      const total = rows.reduce((n, r) => n + (r.metas || []).length, 0);
      if (status) status.textContent = total ? `${total} resultado${total === 1 ? '' : 's'}` : 'Nenhum resultado';
    }
  } catch (e) {
    if (stillHere()) {
      box.innerHTML = `<p class="muted">Erro: ${esc(e.message)}</p>`;
      if (status) status.textContent = 'Erro na pesquisa';
    }
  } finally {
    if (stillHere()) box.removeAttribute('aria-busy');
  }
}

function libraryPage() {
  const favs = getFavs();
  setView(`<h1>Biblioteca</h1>
    <div class="dataactions">
      <button id="export-data" class="chip">Exportar dados</button>
      <button id="import-data" class="chip">Importar dados</button>
      <button id="export-diag" class="chip">Exportar diagnóstico</button>
      <input id="import-file" type="file" accept="application/json,.json" hidden>
    </div>
    <label class="prefrow small">Idioma das legendas
      <select id="sub-lang">
        <option value="pt"${subPref() === 'pt' ? ' selected' : ''}>Português</option>
        <option value="en"${subPref() === 'en' ? ' selected' : ''}>Inglês</option>
        <option value="off"${subPref() === 'off' ? ' selected' : ''}>Sem legenda</option>
      </select>
    </label>
    <label class="prefrow small">Pular abertura e créditos
      <select id="skip-pref">
        <option value="on"${skipPref() ? ' selected' : ''}>Ligado</option>
        <option value="off"${skipPref() ? '' : ' selected'}>Desligado</option>
      </select>
    </label>
    <p class="muted small">Os tempos vêm de bases comunitárias (TheIntroDB e AniSkip). Desligue se não quiser que o título seja consultado nelas.</p>
    <label class="prefrow small">Tema
      <select id="theme-pref">
        <option value="system"${themePref() === 'system' ? ' selected' : ''}>Automático (do aparelho)</option>
        <option value="dark"${themePref() === 'dark' ? ' selected' : ''}>Escuro</option>
        <option value="light"${themePref() === 'light' ? ' selected' : ''}>Claro</option>
      </select>
    </label>
    <p class="muted small">Biblioteca e progresso ficam neste aparelho. Exporte um arquivo para levá-los a outro.</p>${
    favs.length
      ? `<div class="grid">${favs
          .map((f) => `<div class="favcell">${cardHtml(f)}<button class="chip small" data-remove="${esc(f.id)}">Remover</button></div>`)
          .join('')}</div>`
      : '<p class="muted">Sua biblioteca está vazia. Abra um título e clique em “Adicionar à biblioteca”.</p>'
  }`);
  $('#export-data').onclick = exportData;
  $('#export-diag').onclick = exportDiagnostics;
  $('#sub-lang').onchange = (e) => store.set('subLang', e.target.value);
  $('#skip-pref').onchange = (e) => store.set('skipSegments', e.target.value);
  $('#theme-pref').onchange = (e) => {
    store.set('theme', e.target.value);
    applyTheme(e.target.value);
  };
  view.querySelectorAll('[data-remove]').forEach((b) => {
    b.onclick = async () => {
      const ok = await askConfirm('Remover este título da biblioteca?', 'Remover');
      if (!ok) return;
      store.set('favs', getFavs().filter((f) => f.id !== b.dataset.remove));
      libraryPage();
    };
  });
  $('#import-data').onclick = () => $('#import-file').click();
  $('#import-file').onchange = (e) => {
    const file = e.target.files[0];
    e.target.value = '';
    importData(file);
  };
}

// Salva um texto. No desktop abre o "Salvar como"; no Android (sem diálogo) usa a pasta do app.
async function saveText({ content, defaultName, filterName, ext, label, fallback }) {
  let path;
  if (!IS_ANDROID) {
    try {
      path = await invoke('plugin:dialog|save', {
        options: { defaultPath: defaultName, filters: [{ name: filterName, extensions: [ext] }] },
      });
    } catch {
      path = undefined; // diálogo indisponível: usa o salvamento interno abaixo
    }
    if (path === null) {
      toast('Exportação cancelada.');
      return;
    }
    if (path) {
      await call('write_export', { path, content });
      toast(`${label} salvo em: ${path}`);
      return;
    }
  }
  const saved = await fallback();
  toast(`${label} salvo em: ${saved}`);
}

// Exporta biblioteca e progresso (backup .json). O backend grava o arquivo e informa o caminho.
// (Download por blob não funciona no WebView do Android.)
async function exportData() {
  const payload = createBackupData(getFavs(), store.get('progress', {}));
  const content = JSON.stringify(payload, null, 2);
  try {
    await saveText({
      content,
      defaultName: `cinehub-dados-${payload.exportedAt.slice(0, 10)}.json`,
      filterName: 'Backup do CineHub',
      ext: 'json',
      label: 'Backup',
      fallback: () => call('save_export', { content }),
    });
  } catch (err) {
    toast(`Não consegui exportar: ${err.message}`);
  }
}

// Exporta o diagnóstico (registro local, já redigido) como .txt para enviar a quem for ajudar.
async function exportDiagnostics() {
  try {
    const content = await call('diagnostics_text');
    await saveText({
      content,
      defaultName: `cinehub-diagnostico-${new Date().toISOString().slice(0, 10)}.txt`,
      filterName: 'Diagnóstico do CineHub',
      ext: 'txt',
      label: 'Diagnóstico',
      fallback: () => call('save_diagnostics'),
    });
  } catch (err) {
    toast(`Não consegui exportar o diagnóstico: ${err.message}`);
  }
}

// Tema: automático segue o aparelho; claro e escuro são escolhas fixas (guardadas neste aparelho).
function themePref() {
  const v = store.get('theme', 'dark');
  return v === 'light' || v === 'dark' ? v : 'system';
}
function applyTheme(pref) {
  const root = document.documentElement;
  if (pref === 'light' || pref === 'dark') root.dataset.theme = pref;
  else delete root.dataset.theme;
}
applyTheme(themePref());

// Idioma das legendas (preferência guardada neste aparelho).
const SUB_LANGS = { pt: /^(por|pt|portugu[eê]s)/i, en: /^(eng|en|english)/i, off: null };
function subPref() {
  const v = store.get('subLang', 'pt');
  return Object.prototype.hasOwnProperty.call(SUB_LANGS, v) ? v : 'pt';
}

const MAX_IMPORT_BYTES = 2 * 1024 * 1024;
const MAX_FAVS = 500;

// Importa um backup: junta favoritos sem repetir e, no progresso, fica o registro mais recente.
async function importData(file) {
  if (!file) return;
  if (file.size > MAX_IMPORT_BYTES) {
    toast('Arquivo grande demais para importar.');
    return;
  }
  try {
    const data = JSON.parse(await file.text());
    const merged = mergeBackupData(getFavs(), store.get('progress', {}), data, MAX_FAVS, MAX_PROGRESS);
    if (!store.setMany({ favs: merged.favs, progress: merged.progress })) {
      throw new Error('não foi possível salvar os dados; verifique o espaço disponível no aparelho');
    }

    toast('Dados importados.');
    libraryPage();
  } catch (err) {
    toast(`Não consegui importar: ${err.message}`);
  }
}

// Catálogo comunitário próprio do CineHub. URLs de manifest verificadas antes de serem incluídas.
const COMMUNITY_ADDONS = [
  { id: 'community.anime.kitsu', name: 'Anime Kitsu', version: '0.0.10',
    description: 'Catálogos de animes populares, em exibição e mais bem avaliados, com metadados do Kitsu.',
    url: 'https://anime-kitsu.strem.fun/manifest.json', logo: '',
    categories: ['anime', 'catalog'], tags: ['Anime', 'Catálogos', 'Metadados'] },
  { id: 'com.stremio.torrentio.catalog.addon', name: 'Torrent Catalogs', version: '1.0.2',
    description: 'Catálogos de filmes, séries e animes organizados por torrents populares. Para catálogos de anime, requer o Anime Kitsu.',
    url: 'https://torrent-catalogs.strem.fun/manifest.json', logo: 'https://i.ibb.co/w4BnkC9/GwxAcDV.png',
    categories: ['catalog', 'anime'], tags: ['Filmes', 'Séries', 'Anime', 'Catálogos'] },
  { id: 'com.stremio.torrentio.addon', name: 'Torrentio', version: '0.0.15',
    description: 'Addon comunitário de fontes de vídeo. A disponibilidade depende dos provedores e das configurações do serviço.',
    url: 'https://torrentio.strem.fun/manifest.json', logo: 'https://torrentio.strem.fun/images/logo_v1.png',
    categories: ['streams'], tags: ['Filmes', 'Séries', 'Anime', 'Streams'] },
  { id: 'stremio.addons.mediafusion|elfhosted', name: 'MediaFusion | ElfHosted', version: '6.2.0',
    description: 'Addon versátil com fontes P2P, provedores debrid, catálogos e suporte a vários idiomas. Pode ser usado sem configurar uma conta premium em modos compatíveis.',
    url: 'https://mediafusion.elfhosted.com/manifest.json',
    logo: 'https://raw.githubusercontent.com/mhdzumair/MediaFusion/main/resources/images/mediafusion_logo.png',
    categories: ['streams'], tags: ['Filmes', 'Séries', 'Anime', 'Live TV', 'Debrid', 'P2P'] },
  { id: 'comet.elfhosted.com', name: 'Comet | ElfHosted', version: '2.0.0',
    description: 'Busca fontes de filmes, séries e anime. A instância pública do ElfHosted entrega streams via debrid; requer configurar um provedor e sua chave.',
    url: 'https://comet.elfhosted.com/manifest.json',
    logo: 'https://raw.githubusercontent.com/g0ldyy/comet/refs/heads/main/comet/assets/icon.png',
    categories: ['streams'], tags: ['Filmes', 'Séries', 'Anime', 'Debrid', 'Torrents'] },
  { id: 'jackettio.elfhosted.com', name: 'Jackettio | ElfHosted', version: '1.7.0',
    description: 'Pesquisa em indexadores configurados no Jackett e resolve resultados por serviços debrid. Requer configuração de Jackett e provedor compatível.',
    url: 'https://jackettio.elfhosted.com/manifest.json',
    logo: 'https://docs.elfhosted.com/images/logo.png',
    categories: ['streams'], tags: ['Filmes', 'Séries', 'Jackett', 'Debrid', 'Torrents'] },
  { id: 'pw.ers.netflix-catalog', name: 'Streaming Catalogs', version: '1.1.1',
    description: 'Catálogos de tendências de Netflix, HBO Max, Disney+, Prime Video e Apple TV+. A seleção de serviços pode ser configurada no addon.',
    url: 'https://7a82163c306e-stremio-netflix-catalog-addon.baby-beamup.club/manifest.json',
    logo: 'https://play-lh.googleusercontent.com/TBRwjS_qfJCSj1m7zZB93FnpJM5fSpMA_wUlFDLxWAb45T9RmwBvQd5cWR5viJJOhkI',
    categories: ['catalog'], tags: ['Filmes', 'Séries', 'Catálogos', 'Streaming'] },
  { id: 'brazilian-addon', name: 'Mico-Leão Dublado 🇧🇷', version: '1.0.0',
    description: 'Catálogo de filmes dublados em português, com gêneros e busca. Desenvolvido para conteúdo brasileiro.',
    url: 'https://27a5b2bfe3c0-stremio-brazilian-addon.baby-beamup.club/manifest.json',
    logo: 'https://i.ibb.co/YyPbHVT/icon.png',
    categories: ['catalog'], tags: ['Brasil', 'Português', 'Filmes', 'Dublado'] },
  { id: 'tmdb-addon', name: 'The Movie Database Addon', version: '3.1.7',
    description: 'Metadados e catálogos de filmes e séries com busca, tendências, lançamentos, filtros por gênero, ano e idioma. Integrações extras são opcionais.',
    url: 'https://94c8cb9f702d-tmdb-addon.baby-beamup.club/manifest.json',
    logo: 'https://94c8cb9f702d-tmdb-addon.baby-beamup.club/logo.png',
    categories: ['catalog'], tags: ['Filmes', 'Séries', 'Metadados', 'TMDB', 'Português'] },
];

async function addonsPage() {
  setView(`
    <div class="addons-heading"><div><h1>Addons</h1><p class="muted small">Amplie os catálogos e recursos do CineHub.</p></div></div>
    <div class="addon-tabs" role="tablist" aria-label="Seções de addons">
      <button type="button" class="chip on" data-addon-tab="community" role="tab" aria-selected="true">Comunidade</button>
      <button type="button" class="chip" data-addon-tab="installed" role="tab" aria-selected="false">Instalados</button>
    </div>
    <section id="community-panel" class="addon-panel" role="tabpanel">
      <div class="community-status-bar"><p class="muted small">Catálogo selecionado e mantido pelo CineHub. São addons independentes de terceiros; confira a descrição e instale apenas serviços em que confia.</p><button id="check-community" type="button" class="chip small">Verificar disponibilidade</button></div>
      <div class="community-tools">
        <input id="community-search" type="search" placeholder="Buscar addons…" aria-label="Buscar addons da comunidade" autocomplete="off">
        <div class="community-filters" role="group" aria-label="Filtrar addons">
          <button type="button" class="chip small on" data-addon-filter="all" aria-pressed="true">Todos</button>
          <button type="button" class="chip small" data-addon-filter="catalog" aria-pressed="false">Catálogos</button>
          <button type="button" class="chip small" data-addon-filter="anime" aria-pressed="false">Anime</button>
          <button type="button" class="chip small" data-addon-filter="streams" aria-pressed="false">Streams</button>
        </div>
      </div>
      <div id="community-list" class="community-grid"><p class="muted">Carregando catálogo…</p></div>
    </section>
    <section id="installed-panel" class="addon-panel" role="tabpanel" hidden>
      <div id="al" class="loading">Carregando addons instalados…</div>
    </section>
    <details class="addon-manual"><summary>Instalar addon por endereço</summary>
      <form id="af" class="addform">
        <input id="au" placeholder="Link do manifest.json ou stremio://…" aria-label="Link do addon">
        <button type="submit">Instalar</button>
      </form>
    </details>
    <form id="tf" class="torrentform">
      <input id="tm" placeholder="Cole um magnet: ou use um arquivo .torrent" aria-label="Link magnet do torrent">
      <label class="torrentfile"><span>Selecionar .torrent</span><input id="torrent-file" type="file" accept=".torrent" hidden></label>
      <button type="submit" class="chip">Abrir torrent</button>
    </form>
    <p class="muted small">Addons compatíveis com o protocolo do Stremio. Instale apenas fontes que você tem autorização para acessar.</p>`);

  const communityPanel = $('#community-panel'), installedPanel = $('#installed-panel');
  const communityList = $('#community-list'), installedList = $('#al');
  let activeFilter = 'all', installed = [];
  const addonHealth = new Map();
  const COMMUNITY_CHECK_TTL = 15 * 60 * 1000;
  let communityLastCheckedAt = 0;
  let communityCheckPromise = null;
  const installedFor = (addon) => installed.some((entry) => entry.manifest?.id === addon.id || entry.transportUrl === addon.url);

  const renderCommunity = () => {
    const query = ($('#community-search')?.value || '').trim().toLocaleLowerCase();
    const matches = COMMUNITY_ADDONS.filter((addon) => {
      const categoryMatch = activeFilter === 'all' || addon.categories.includes(activeFilter);
      const content = [addon.name, addon.description, ...addon.tags].join(' ').toLocaleLowerCase();
      return categoryMatch && (!query || content.includes(query));
    });
    communityList.innerHTML = matches.length ? matches.map((addon) => {
      const done = installedFor(addon), initial = esc(addon.name.slice(0, 1).toUpperCase());
      return `<article class="community-addon">
        <div class="community-addon-head">
          ${isWebUrl(addon.logo) ? `<img src="${esc(addon.logo)}" alt="" loading="lazy">` : `<span class="logo-fallback" aria-hidden="true">${initial}</span>`}
          <div><h3>${esc(addon.name)}</h3><span class="muted small">v${esc(addon.version)}</span></div>
        </div>
        <p>${esc(addon.description)}</p>
        <p class="addon-health muted small" data-community-status="${esc(addon.id)}" data-state="${esc(addonHealth.get(addon.id)?.state || 'unknown')}" role="status">${esc(addonHealth.get(addon.id)?.label || 'Não verificado')}</p>
        <div class="community-tags">${addon.tags.map((tag) => `<span>${esc(tag)}</span>`).join('')}</div>
        <button type="button" class="community-install${done ? ' installed' : ''}" data-community-install="${esc(addon.id)}" ${done ? 'disabled' : ''}>${done ? '✓ Instalado' : '＋ Instalar'}</button>
      </article>`;
    }).join('') : '<p class="muted">Nenhum addon encontrado com esses filtros.</p>';
  };

  const renderInstalled = () => {
    installedList.className = '';
    installedList.innerHTML = installed.length ? installed.map(addonHtml).join('') : '<p class="muted">Nenhum addon instalado.</p>';
    installedList.querySelectorAll('button[data-url]').forEach((button) => {
      button.onclick = async () => {
        const ok = await askConfirm('Remover este addon? Os catálogos e fontes dele deixam de aparecer.', 'Remover');
        if (!ok) return;
        try { await call('remove_addon', { url: button.dataset.url }); await refreshInstalled(); toast('Addon removido.'); }
        catch (err) { toast(err.message); }
      };
    });
  };

  const refreshInstalled = async () => {
    installed = await call('list_addons');
    renderInstalled();
    renderCommunity();
  };
  const selectTab = (tab) => {
    communityPanel.hidden = tab !== 'community';
    installedPanel.hidden = tab !== 'installed';
    document.querySelectorAll('[data-addon-tab]').forEach((button) => {
      const selected = button.dataset.addonTab === tab;
      button.classList.toggle('on', selected);
      button.setAttribute('aria-selected', String(selected));
    });
    if (tab === 'community') checkCommunityAddons(false);
  };
  document.querySelectorAll('[data-addon-tab]').forEach((button) => { button.onclick = () => selectTab(button.dataset.addonTab); });
  document.querySelectorAll('[data-addon-filter]').forEach((button) => {
    button.onclick = () => {
      activeFilter = button.dataset.addonFilter;
      document.querySelectorAll('[data-addon-filter]').forEach((filter) => {
        const selected = filter === button;
        filter.classList.toggle('on', selected);
        filter.setAttribute('aria-pressed', String(selected));
      });
      renderCommunity();
    };
  });
  async function checkCommunityAddons(force = false) {
    if (communityCheckPromise) return communityCheckPromise;
    if (!force && communityLastCheckedAt && Date.now() - communityLastCheckedAt < COMMUNITY_CHECK_TTL) return;
    const checkButton = $('#check-community');
    if (checkButton) { checkButton.disabled = true; checkButton.textContent = 'Verificando…'; }
    const queue = [...COMMUNITY_ADDONS];
    let cursor = 0;
    const worker = async () => {
      while (cursor < queue.length) {
        const addon = queue[cursor++];
        addonHealth.set(addon.id, { state: 'checking', label: 'Verificando disponibilidade…' });
        const status = communityList.querySelector(`[data-community-status="${CSS.escape(addon.id)}"]`);
        if (status) { status.textContent = 'Verificando disponibilidade…'; status.dataset.state = 'checking'; }
        try {
          const result = await call('check_addon_manifest', {
            url: addon.url, expectedId: addon.id, expectedVersion: addon.version,
          });
          const label = result.versionMatches
            ? `Disponível · v${result.version}`
            : `Disponível · versão publicada v${result.version} (catálogo v${addon.version})`;
          addonHealth.set(addon.id, { state: result.versionMatches ? 'ok' : 'version-differs', label });
        } catch (error) {
          const label = `Indisponível · ${String(error.message || error).slice(0, 140)}`;
          addonHealth.set(addon.id, { state: 'error', label });
        }
        const current = communityList.querySelector(`[data-community-status="${CSS.escape(addon.id)}"]`);
        if (current) {
          current.textContent = addonHealth.get(addon.id).label;
          current.dataset.state = addonHealth.get(addon.id).state;
        }
      }
    };
    communityCheckPromise = (async () => {
      try {
        await Promise.all(Array.from({ length: Math.min(3, queue.length) }, worker));
        communityLastCheckedAt = Date.now();
        toast('Verificação dos addons concluída.');
      } finally {
        communityCheckPromise = null;
        if (checkButton) { checkButton.disabled = false; checkButton.textContent = 'Verificar novamente'; }
      }
    })();
    return communityCheckPromise;
  }

  $('#community-search').addEventListener('input', renderCommunity);
  $('#check-community').onclick = () => checkCommunityAddons(true);
  communityList.addEventListener('click', async (event) => {
    const button = event.target.closest('[data-community-install]');
    if (!button || button.disabled) return;
    const addon = COMMUNITY_ADDONS.find((entry) => entry.id === button.dataset.communityInstall);
    if (!addon) return;
    button.disabled = true; button.textContent = 'Instalando…';
    try {
      const result = await call('install_addon', { url: addon.url });
      toast(`Addon instalado: ${result.name}`);
      await refreshInstalled();
    } catch (err) {
      logEvent('WARN', `instalar addon da comunidade falhou: ${err.message}`);
      toast(err.message || String(err));
      button.disabled = false; button.textContent = '＋ Instalar';
    }
  });

  $('#af').onsubmit = async (event) => {
    event.preventDefault();
    try {
      const result = await call('install_addon', { url: $('#au').value });
      toast(`Addon instalado: ${result.name}`);
      $('#au').value = '';
      await refreshInstalled();
      selectTab('installed');
    } catch (err) { logEvent('WARN', `instalar addon falhou: ${err.message}`); toast(err.message); }
  };

  $('#tf').onsubmit = async (event) => {
    event.preventDefault();
    const input = $('#torrent-file'), magnet = $('#tm').value.trim();
    try {
      if (input.files && input.files.length) {
        const file = input.files[0];
        if (file.size > 8 * 1024 * 1024) { toast('Arquivo .torrent maior que 8 MiB.'); return; }
        const bytes = new Uint8Array(await file.arrayBuffer());
        const result = await withBusy(TORRENT_BUSY, call('torrent_open', { magnet: null, torrentBytes: Array.from(bytes), fileIndex: null }));
        await playOpenedTorrent(result, file.name);
        return;
      }
      if (!magnet) { toast('Cole um magnet ou selecione um .torrent.'); return; }
      const result = await withBusy(TORRENT_BUSY, call('torrent_open', { magnet, torrentBytes: null, fileIndex: null }));
      await playOpenedTorrent(result, 'Torrent');
    } catch (err) { toast(err.message || String(err)); }
  };

  selectTab('community');
  await refreshInstalled();
}
async function detailPage(kind, id, alive) {
  setView(`<div class="hero skel-hero" aria-hidden="true"></div>`);
  const { meta } = await call('meta', { kind, id });
  if (!alive()) return;
  if (!meta) {
    setView('<p class="muted">Não encontrei informações deste título. Verifique se há um addon de metadados instalado.</p>');
    return;
  }

  document.title = `${meta.name || 'Título'} · CineHub`;
  const bg = isWebUrl(meta.background) ? meta.background : '';
  const info = [meta.releaseInfo, (meta.genres || []).join(', ')].filter(Boolean).join(' · ');
  const rating = meta.imdbRating ? `<span class="rating" title="Nota IMDb">IMDb ${esc(meta.imdbRating)}</span>` : '';
  setView(`
    <div class="hero" style="${esc(heroStyle(bg))}">
      <div class="hero-info">
        <h1>${esc(meta.name)}</h1>
        <p class="muted">${rating}${esc(info)}</p>
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

  // Filmes: as fontes ficam num painel ao lado da imagem, em telas largas (como no dxbi).
  if (isSeriesLike(meta.type || kind)) renderSeries(meta);
  else {
    view.classList.add('detail-with-panel');
    loadStreams(meta.type || kind, meta.id, meta.name, meta);
  }
}

// ---------- Componentes ----------

function renderRows(rows, emptyMsg, { pageable = true, targetId = 'rows' } = {}) {
  const box = $(`#${targetId}`);
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
  const wrap = document.createElement('div');
  wrap.className = 'strip-wrap';
  const strip = document.createElement('div');
  strip.className = 'strip';
  wrap.append(strip);
  sec.append(wrap);
  // Setas de paginação: só com o mouse (tabindex -1), o controle remoto anda pelos cartazes.
  const arrow = (dir) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = `row-arrow ${dir}`;
    b.tabIndex = -1;
    b.setAttribute('aria-label', dir === 'prev' ? 'Anterior' : 'Próximo');
    b.textContent = dir === 'prev' ? '‹' : '›';
    b.onclick = () => strip.scrollBy({ left: (dir === 'prev' ? -1 : 1) * strip.clientWidth * 0.85, behavior: 'smooth' });
    return b;
  };
  const prev = arrow('prev');
  const next = arrow('next');
  wrap.append(prev, next);
  const updateArrows = () => {
    const max = strip.scrollWidth - strip.clientWidth;
    prev.hidden = max <= 0 || strip.scrollLeft <= 4;
    next.hidden = max <= 0 || strip.scrollLeft >= max - 4;
  };
  strip.addEventListener('scroll', updateArrows, { passive: true });
  if (typeof ResizeObserver === 'function') new ResizeObserver(updateArrows).observe(strip);

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
    updateArrows();
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

function heroStyle(bg) {
  const img = cssUrl(bg);
  return img ? `background-image: ${img}` : '';
}

// Progresso guardado para um título (ou para um de seus episódios, em séries).
function progressFor(id) {
  const all = store.get('progress', {});
  if (all[id]) return all[id];
  return Object.values(all).find((p) => p && p.seriesId === id) || null;
}

// Cartaz remoto indisponível: substitui a imagem quebrada pelo título, sem deixar ícone de erro.
view.addEventListener('error', (event) => {
  const img = event.target;
  if (!(img instanceof HTMLImageElement) || !img.matches('.poster-img')) return;
  const card = img.closest('.card');
  const placeholder = document.createElement('span');
  placeholder.className = 'poster-placeholder';
  placeholder.textContent = card?.querySelector('.title')?.textContent || 'Imagem indisponível';
  img.replaceWith(placeholder);
}, true);

function cardHtml(m) {
  // Cartaz como <img> com carregamento preguiçoso (só baixa o que aparece na tela).
  const art = isWebUrl(m.poster) ? `<img class="poster-img" src="${esc(m.poster)}" alt="" loading="lazy" decoding="async">` : '';
  const label = [m.name, m.releaseInfo].filter(Boolean).join(', ');
  const p = progressFor(m.id);
  const pct = progressPercent(p);
  const bar = pct > 0 ? `<div class="progress" aria-hidden="true"><span style="width:${pct.toFixed(1)}%"></span></div>` : '';
  const seen = p?.watched ? '<span class="seen">✓ Assistido</span>' : '';
  return `
    <a class="card" href="#/detail/${esc(m.type)}/${encodeURIComponent(m.id)}" aria-label="${esc(label)}">
      <div class="poster">${art || esc(m.name)}${bar}${seen}</div>
      <div class="title">${esc(m.name)}</div>
      <div class="muted small">${esc(m.releaseInfo || '')}</div>
    </a>`;
}

function addonHtml(a) {
  const m = a.manifest;
  const resources = (m.resources || []).map((r) => (typeof r === 'string' ? r : r.name)).join(', ');
  // Sem logo, mostra a inicial do nome em vez de um quadrado vazio.
  const initial = esc(String(m.name || '?').trim().charAt(0).toUpperCase());
  return `
    <div class="addon">
      ${isWebUrl(m.logo) ? `<img src="${esc(m.logo)}" alt="" loading="lazy">` : `<span class="logo-fallback" aria-hidden="true">${initial}</span>`}
      <div>
        <strong>${esc(m.name)}</strong> <span class="muted small">v${esc(m.version)}</span>
        <p class="muted small">${esc(m.description || '')}</p>
        <p class="small">Recursos: ${esc(resources)} · Tipos: ${esc((m.types || []).join(', '))}</p>
      </div>
      <button class="danger" data-url="${esc(a.transportUrl)}">Remover</button>
    </div>`;
}

// Temporada e episódio vêm do addon e não são confiáveis: viram número antes de qualquer uso,
// e os textos passam por esc(). Sem número válido, o episódio aparece sem prefixo.
const { seasonOf: seasonNo, episodeOf: episodeNo } = window.CineHubLib;

function renderSeries(meta) {
  const videos = (meta.videos || []).filter((v) => v && v.id);
  const seasons = [...new Set(videos.map(seasonNo))].sort((a, b) => a - b);
  let season = seasons.find((s) => s > 0) ?? seasons[0];
  const body = $('#body');

  const draw = () => {
    const eps = videos
      .filter((v) => seasonNo(v) === season)
      .sort((a, b) => (episodeNo(a) ?? 0) - (episodeNo(b) ?? 0));
    const seasonName = (s) => (s === 0 ? 'Especiais' : `Temporada ${s}`);

    body.innerHTML = `
      <div class="seasons" role="group" aria-label="Temporadas">${seasons
        .map((s) => {
          const on = s === season;
          return `<button class="chip ${on ? 'on' : ''}" type="button" data-s="${s}" aria-pressed="${on}">${esc(seasonName(s))}</button>`;
        })
        .join('')}</div>
      <p class="muted small">${eps.length} ${eps.length === 1 ? 'episódio' : 'episódios'}</p>
      <div class="episodes">${eps
        .map((e) => {
          const no = episodeNo(e);
          const prefix = no == null ? '' : `${no}. `;
          const title = e.title || e.name || `Episódio ${no ?? ''}`.trim();
          // Episódio já assistido ganha um ✓ (texto, não só cor, para ser lido também por leitor de tela).
          const seen = progressFor(e.id)?.watched === true;
          const mark = seen ? '✓ ' : '';
          const toggle = seen ? 'Desmarcar' : 'Marcar';
          return `<div class="ep-row">
            <button class="ep${seen ? ' seen' : ''}" type="button" data-id="${esc(e.id)}">${mark}${esc(prefix + title)}</button>
            <button class="chip watch" type="button" data-watch="${esc(e.id)}" aria-pressed="${seen}" aria-label="${toggle} como assistido: ${esc(title)}" title="${toggle} como assistido">${seen ? '✓' : '○'}</button>
          </div>`;
        })
        .join('')}</div>
      <div id="streams"></div>`;

    body.querySelectorAll('.seasons .chip').forEach((b) => {
      b.onclick = () => {
        season = CineHubLib.num(b.dataset.s);
        draw();
      };
    });
    body.querySelectorAll('.watch').forEach((b) => {
      b.onclick = () => {
        const id = b.dataset.watch;
        setWatched(id, progressFor(id)?.watched !== true);
        draw();
      };
    });
    body.querySelectorAll('.ep').forEach((b) => {
      b.onclick = () => {
        const ep = eps.find((e) => e.id === b.dataset.id);
        const no = ep ? episodeNo(ep) : null;
        const label = `${meta.name} · T${season}E${no ?? ''}`;
        const epKind = meta.type === 'anime' ? 'anime' : 'series';
        loadStreams(epKind, b.dataset.id, label, { ...meta, type: epKind, seriesId: meta.id, poster: meta.poster, name: label });
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

  let result;
  try {
    result = await call('streams', { kind, id });
  } catch (err) {
    logEvent('WARN', `buscar fontes falhou: ${err.message}`);
    if (seq === streamsSeq) {
      const box = $('#streams') || $('#body');
      if (box) {
        box.innerHTML = '';
        const message = document.createElement('p');
        message.className = 'muted';
        message.textContent = `Não consegui buscar as fontes: ${err.message}`;
        const retry = document.createElement('button');
        retry.type = 'button';
        retry.className = 'chip';
        retry.textContent = 'Tentar novamente';
        retry.onclick = () => loadStreams(kind, id, label, meta);
        box.append(message, retry);
      }
    }
    return;
  }
  if (seq !== streamsSeq) return;
  const { subtitles = [], errors = [] } = result;
  // Fontes que tocam vêm primeiro, depois as de maior resolução (e dublagem em português).
  const allStreams = Array.isArray(result.streams) ? result.streams : [];
  const streams = sortStreams(allStreams);
  const box = $('#streams') || $('#body');
  if (!box) return;
  box.innerHTML = '';

  // Addons que falharam aparecem aqui, mesmo quando outros devolveram fontes.
  const showErrors = () => {
    for (const e of errors) {
      const p = document.createElement('p');
      p.className = 'muted small';
      p.textContent = `${e.addon}: ${e.error}`;
      box.append(p);
    }
  };

  if (!streams.length) {
    const message = document.createElement('p');
    message.className = 'muted';
    message.textContent = 'Nenhuma fonte reproduzível encontrada. Verifique os addons instalados ou tente novamente.';
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.className = 'chip';
    retry.textContent = 'Buscar fontes novamente';
    retry.onclick = () => loadStreams(kind, id, label, meta);
    box.append(message, retry);
    showErrors();
    return;
  }

  const title = document.createElement('h3');
  title.textContent = 'Fontes';
  const refresh = document.createElement('button');
  refresh.type = 'button';
  refresh.className = 'chip small';
  refresh.textContent = '↻ Atualizar fontes';
  refresh.setAttribute('aria-label', 'Buscar fontes novamente nos addons');
  refresh.onclick = () => loadStreams(kind, id, label, meta);
  box.append(title, refresh);

  // Barra de filtro e ordenação (só aparece com 3 ou mais fontes, como no dxbi).
  const ui = { sort: 'default', quality: [], lang: [], addon: [] };
  const bar = document.createElement('div');
  bar.className = 'stream-bar';
  const list = document.createElement('div');
  list.className = 'stream-list';
  box.append(bar, list);
  const primary = document.createElement('button');
  primary.className = 'primary-play';
  primary.textContent = '▶ Assistir';
  primary.onclick = () => list.querySelector('.stream:not(.disabled)')?.click();
  box.prepend(primary);

  const groups = {
    quality: { label: 'Qualidade', values: [...new Set(streams.map((s) => qualityLabel(streamText(s))))], name: (v) => v },
    lang: { label: 'Idioma', values: [...new Set(streams.map((s) => torrentLanguage(streamText(s))))], name: (v) => STREAM_LANG[v] || v },
    addon: { label: 'Addon', values: [...new Set(streams.map((s) => str(s.addonName)).filter(Boolean))], name: (v) => v },
  };
  const paintBar = () => {
    const total = streams.length;
    const shown = sortStreamsBy(filterStreams(streams, ui), ui.sort).length;
    const chips = Object.entries(groups)
      .filter(([, g]) => g.values.length > 1)
      .map(([key, g]) => `<div class="stream-group"><span class="small muted">${esc(g.label)}</span>${g.values
        .map((v) => `<button type="button" class="chip small${ui[key].includes(v) ? ' on' : ''}" data-group="${key}" data-value="${esc(v)}" aria-pressed="${ui[key].includes(v)}">${esc(g.name(v))}</button>`)
        .join('')}</div>`)
      .join('');
    const anyFilter = ui.quality.length || ui.lang.length || ui.addon.length || ui.sort !== 'default';
    bar.innerHTML = `
      <div class="stream-tools">
        <label class="small muted">Ordenar
          <select id="stream-sort">${STREAM_SORTS.map(([v, l]) => `<option value="${v}"${ui.sort === v ? ' selected' : ''}>${esc(l)}</option>`).join('')}</select>
        </label>
        <span class="small muted">${shown === total ? `${total} fontes` : `${shown} de ${total}`}</span>
        ${anyFilter ? '<button type="button" class="chip small" data-reset="1">Limpar</button>' : ''}
      </div>
      ${chips}`;
    bar.hidden = total < 3;
    paintList(sortStreamsBy(filterStreams(streams, ui), ui.sort));
  };
  bar.addEventListener('change', (e) => {
    if (e.target.id === 'stream-sort') {
      ui.sort = e.target.value;
      paintBar();
    }
  });
  bar.addEventListener('click', (e) => {
    const btn = e.target.closest('button');
    if (!btn) return;
    if (btn.dataset.reset) {
      Object.assign(ui, { sort: 'default', quality: [], lang: [], addon: [] });
    } else if (btn.dataset.group) {
      const set = new Set(ui[btn.dataset.group]);
      set.has(btn.dataset.value) ? set.delete(btn.dataset.value) : set.add(btn.dataset.value);
      ui[btn.dataset.group] = [...set];
    } else return;
    paintBar();
  });

  const paintList = (shown) => {
    list.innerHTML = '';
    if (!shown.length) {
      list.innerHTML = '';
      const message = document.createElement('p');
      message.className = 'muted small';
      message.textContent = 'Nenhuma fonte corresponde aos filtros selecionados.';
      const clear = document.createElement('button');
      clear.type = 'button';
      clear.className = 'chip';
      clear.textContent = 'Limpar filtros e mostrar todas';
      clear.onclick = () => {
        Object.assign(ui, { sort: 'default', quality: [], lang: [], addon: [] });
        paintBar();
      };
      list.append(message, clear);
      primary.hidden = true;
      return;
    }
    primary.hidden = !shown.some((s) => isPlayable(s));
    for (const s of shown) {
      const playable = isPlayable(s);
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
        b.onclick = () => openExternal(s.externalUrl);
      }
      list.append(b);
    }
  };
  const isPlayable = (s) => Boolean(s.url || s.externalUrl || s.infoHash || s.hash || String(s.url || '').startsWith('magnet:?'));

  paintBar();
  showErrors();
}

// ---------- Cartão de prévia ----------
// Ao passar o mouse (ou focar com o controle remoto) sobre um título, abre um cartão com fundo,
// nota, dados e botões. Adaptado do cartão de prévia do dxbi.plugin.js (dx.bi, GPL-3.0-or-later),
// sem o efeito de empurrar os vizinhos. Fecha ao sair do cartão e da prévia.
const preview = { el: null, card: null, openTimer: null, closeTimer: null, token: 0 };
const previewMeta = new Map();

function cardTarget(card) {
  const m = /^#\/detail\/([^/]+)\/(.+)$/.exec(card.getAttribute('href') || '');
  return m ? { kind: decodeURIComponent(m[1]), id: decodeURIComponent(m[2]) } : null;
}

function loadPreviewMeta(kind, id) {
  const key = `${kind}:${id}`;
  if (!previewMeta.has(key)) {
    previewMeta.set(key, call('meta', { kind, id }).then((r) => r.meta || null).catch(() => null));
  }
  return previewMeta.get(key);
}

function ensurePreviewEl() {
  if (preview.el) return preview.el;
  const el = document.createElement('div');
  el.className = 'preview-card';
  el.setAttribute('role', 'dialog');
  el.setAttribute('aria-label', 'Prévia do título');
  el.addEventListener('pointerenter', () => clearTimeout(preview.closeTimer));
  el.addEventListener('pointerleave', (e) => {
    if (e.pointerType !== 'touch') schedulePreviewClose();
  });
  el.addEventListener('focusout', (e) => {
    const next = e.relatedTarget;
    if (!el.contains(next) && !(preview.card && preview.card.contains(next))) schedulePreviewClose();
  });
  document.body.append(el);
  preview.el = el;
  return el;
}

function schedulePreviewClose(delay = 180) {
  clearTimeout(preview.openTimer);
  clearTimeout(preview.closeTimer);
  preview.closeTimer = setTimeout(closePreview, delay);
}

function closePreview() {
  clearTimeout(preview.openTimer);
  clearTimeout(preview.closeTimer);
  preview.token++;
  preview.card = null;
  if (preview.el) preview.el.classList.remove('is-open');
}

// Fica sobre o cartaz; se não couber em cima, vai para baixo, sem sair da tela.
function positionPreview(el, card) {
  const r = card.getBoundingClientRect();
  const width = Math.min(340, window.innerWidth - 24);
  const left = Math.max(12, Math.min(r.left + r.width / 2 - width / 2, window.innerWidth - width - 12));
  const height = el.offsetHeight || 260;
  let top = r.top - height - 10;
  if (top < 72) top = r.bottom + 10;
  top = Math.max(12, Math.min(top, window.innerHeight - height - 12));
  el.style.width = `${width}px`;
  el.style.left = `${Math.round(left)}px`;
  el.style.top = `${Math.round(top)}px`;
}

function paintPreview(el, card, target, meta) {
  const titleEl = card.querySelector('.title');
  const name = meta.name || (titleEl ? titleEl.textContent : '');
  const poster = card.querySelector('.poster-img')?.getAttribute('src') || '';
  const bg = isWebUrl(meta.background) ? meta.background : isWebUrl(meta.poster) ? meta.poster : poster;
  const year = meta.releaseInfo || card.querySelector('.card .small, .small')?.textContent || '';
  const facts = [
    meta.imdbRating ? `<span class="rating">IMDb ${esc(meta.imdbRating)}</span>` : '',
    year ? `<span>${esc(year)}</span>` : '',
    meta.runtime ? `<span>${esc(meta.runtime)}</span>` : '',
    typeLabel(target.kind) ? `<span>${esc(typeLabel(target.kind))}</span>` : '',
  ].filter(Boolean).join('');
  const genres = (meta.genres || []).slice(0, 3);
  const href = `#/detail/${esc(target.kind)}/${encodeURIComponent(target.id)}`;
  el.innerHTML = `
    <div class="preview-media" style="background-image: ${cssUrl(bg)}">
      <div class="preview-name">${esc(name)}</div>
    </div>
    <div class="preview-body">
      <div class="preview-facts">${facts}</div>
      ${meta.description ? `<p class="preview-desc">${esc(meta.description)}</p>` : ''}
      ${genres.length ? `<div class="preview-genres">${genres.map((g) => `<span>${esc(g)}</span>`).join('')}</div>` : ''}
      <div class="preview-actions">
        <a class="btn-link btn-light" href="${href}">▶ Assistir</a>
        <a class="btn-link btn-grey" href="${href}">ⓘ Mais informações</a>
      </div>
    </div>`;
}

async function openPreview(card) {
  const target = cardTarget(card);
  if (!target) return;
  const token = ++preview.token;
  preview.card = card;
  const el = ensurePreviewEl();
  paintPreview(el, card, target, {});
  positionPreview(el, card);
  el.classList.add('is-open');
  const meta = (await loadPreviewMeta(target.kind, target.id)) || {};
  if (token !== preview.token || !card.isConnected) return;
  paintPreview(el, card, target, meta);
  positionPreview(el, card);
}

view.addEventListener('pointerover', (e) => {
  if (e.pointerType === 'touch') return;
  const card = e.target.closest('.card');
  if (!card) return;
  clearTimeout(preview.closeTimer);
  if (card === preview.card) return;
  clearTimeout(preview.openTimer);
  preview.openTimer = setTimeout(() => openPreview(card), 450);
});
view.addEventListener('pointerout', (e) => {
  if (e.pointerType === 'touch') return;
  const card = e.target.closest('.card');
  if (!card) return;
  if (e.relatedTarget && (card.contains(e.relatedTarget) || preview.el?.contains(e.relatedTarget))) return;
  clearTimeout(preview.openTimer);
  if (preview.card === card) schedulePreviewClose();
});
// A prévia permanece exclusiva do ponteiro (desktop). Não abre ao focar com controle remoto,
// evitando a mini janela sobre os cartazes na navegação da Android TV.
// Rolagem (inclusive a dos trilhos, que o controle remoto usa): o cartão acompanha o título.
view.addEventListener('scroll', () => {
  if (preview.card && preview.el?.classList.contains('is-open')) positionPreview(preview.el, preview.card);
}, { capture: true, passive: true });
window.addEventListener('scroll', () => {
  if (preview.card && preview.el?.classList.contains('is-open')) positionPreview(preview.el, preview.card);
}, { passive: true });
window.addEventListener('hashchange', closePreview);

// ---------- Player (Video.js, com HLS embutido) ----------

// Codecs e contêineres que o WebView do aparelho aceita. canPlayType responde
// '', 'maybe' ou 'probably'; o resultado é guardado como booleano.
const CODEC_TESTS = {
  h264: 'video/mp4; codecs="avc1.640028"',
  hevc: 'video/mp4; codecs="hvc1.1.6.L120.B0"',
  vp9: 'video/webm; codecs="vp09.00.41.08"',
  av1: 'video/mp4; codecs="av01.0.08M.08"',
  mkv: 'video/x-matroska',
  ts: 'video/mp2t',
};
const playbackSupport = {};
(function probeContainers() {
  const probe = document.createElement('video');
  for (const [k, type] of Object.entries(CODEC_TESTS)) {
    playbackSupport[k] = { supported: probe.canPlayType(type) !== '', hardware: null };
  }
})();

// Pergunta ao sistema se a decodificação de cada codec é eficiente (geralmente, por hardware).
// É uma estimativa do WebView: o app não consegue forçar o hardware, só saber se ele existe.
async function probeHardwareDecode() {
  if (!navigator.mediaCapabilities?.decodingInfo) return;
  for (const k of ['h264', 'hevc', 'vp9', 'av1']) {
    try {
      const r = await navigator.mediaCapabilities.decodingInfo({
        type: 'file',
        video: { contentType: CODEC_TESTS[k], width: 1920, height: 1080, bitrate: 8000000, framerate: 30 },
      });
      playbackSupport[k] = { supported: r.supported, hardware: r.powerEfficient };
    } catch {
      playbackSupport[k] = { supported: playbackSupport[k].supported, hardware: null };
    }
  }
}
probeHardwareDecode();

// Tipo do arquivo a partir da URL ou do tipo informado pela fonte.
function formatOf(src, mime) {
  if (mime) return mime;
  const path = String(src || '').split('?')[0].toLowerCase();
  if (/\.mkv$/.test(path)) return 'video/x-matroska';
  if (/\.ts$/.test(path)) return 'video/mp2t';
  if (/\.(avi|mpe?g)$/.test(path)) return 'video/x-msvideo';
  if (/\.m3u8$/.test(path)) return 'application/x-mpegURL';
  return '';
}

// Mensagem que explica o erro de reprodução, com base no código do MediaError.
function describePlaybackError(err, src, mime) {
  const code = err?.code;
  const format = formatOf(src, mime);
  if (code === 2) return 'Falha de rede ao baixar o vídeo. Verifique a conexão e tente de novo.';
  if (code === 3) {
    const hw = playbackSupport.hevc?.hardware === false ? ' Este aparelho não decodifica H.265 por hardware.' : '';
    return `O aparelho não conseguiu decodificar este vídeo (codec sem suporte ou arquivo danificado).${hw} Tente outra fonte.`;
  }
  if (code === 4) {
    if (format === 'video/x-matroska') return 'O player do aparelho não toca MKV. Escolha uma fonte MP4 ou WebM.';
    if (format === 'video/mp2t') return 'O player do aparelho não toca MPEG-TS. Escolha uma fonte MP4 ou WebM.';
    if (format === 'video/x-msvideo' || format === 'video/mpeg') return 'O player do aparelho não toca AVI/MPEG. Escolha uma fonte MP4 ou WebM.';
    return 'Formato ou codec não suportado por este aparelho. Tente outra fonte.';
  }
  return 'Não foi possível tocar esta fonte. Tente outra.';
}

const vjsPlayer = videojs(video, { controls: true, fluid: true, preload: 'auto', playsinline: true, playbackRates: [0.5, 0.75, 1, 1.25, 1.5, 2] });
let current = null; // { id, label, meta } do que está tocando agora
let activeTorrentToken = null; // sessão torrent vinculada à reprodução atual
let playSeq = 0; // muda a cada fonte; legendas de fontes antigas são descartadas
let blobUrls = [];
let lastSave = 0;
let resumeHandler = null; // retomada da posição salva no próximo loadedmetadata

// Android usa ExoPlayer. No Windows, a reprodução é sempre feita pelo player HTML5
// dentro do WebView2; evita abrir uma janela externa do mpv e mantém os controles do CineHub.
const IS_ANDROID = /Android/i.test(navigator.userAgent);
const IS_WINDOWS = /Windows/i.test(navigator.userAgent);

// Baixa todas as legendas externas para que o usuário escolha no ExoPlayer.
async function subtitlePayload(stream, subs) {
  const selected = pickSubtitles(stream, subs).slice(0, 8);
  const tracks = await Promise.all(selected.map(async (sub) => {
    try {
      const vtt = await call('subtitle', { url: sub.url });
      const bytes = new TextEncoder().encode(vtt);
      let binary = '';
      for (const b of bytes) binary += String.fromCharCode(b);
      return { data: btoa(binary), language: sub.language, label: sub.label, isDefault: sub.isDefault };
    } catch (e) {
      console.warn('legenda indisponível para o player nativo:', sub.label, e);
      return null;
    }
  }));
  return JSON.stringify(tracks.filter(Boolean));
}

// Texto legível para erros vindos do Tauri/plugins (que podem ser strings ou objetos).
function describeError(e) {
  if (typeof e === 'string') return e;
  if (e && typeof e.message === 'string') return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

// Motivo da última falha do player nativo (mostrado se o player web também não tocar).
let nativeFailure = null;

async function playNative(stream, label, id, meta, subs, segments = []) {
  const torrentToken = stream?.torrentToken || null;
  let completed = false;
  const saved = store.get('progress', {})[id];
  const startMs = saved && saved.t > 5 && !saved.watched ? saved.t * 1000 : 0;
  current = { id, label, meta, src: stream.url, mime: '' };
  document.title = `${label} · CineHub`;
  try {
    const command = IS_ANDROID ? 'plugin:exoplayer|play' : 'player_play';
    const args = { url: stream.url, startMs };
    // Android recebe todas as faixas para seleção no ExoPlayer; desktop usa a preferida.
    if (IS_ANDROID) {
      args.subtitles = await subtitlePayload(stream, subs);
      args.subtitlesDisabled = subPref() === 'off';
    } else args.subtitleUrl = pickSubtitles(stream, subs)[0]?.url ?? null;
    // Botão de pular no player nativo: tecla F12 (mpv, desktop) ou botão na tela (Android)
    args.segments = IS_ANDROID ? JSON.stringify(segments) : segments;
    const res = await invoke(command, args);
    completed = true;
    saveProgress(true, { t: (res?.positionMs || 0) / 1000, duration: (res?.durationMs || 0) / 1000 });
    return true;
  } catch (e) {
    console.warn('player nativo indisponível, usando o do navegador:', e);
    // Guarda o motivo: o aviso do player web (ex.: "não toca MKV") o mostra junto.
    nativeFailure = describeError(e).slice(0, 300);
    logEvent('WARN', `player nativo indisponível: ${nativeFailure}`);
    return false;
  } finally {
    current = null;
    document.title = 'CineHub';
    // O comando nativo só resolve quando o diálogo do player é fechado.
    // Em sucesso, libera o torrent; em falha, mantém a sessão para o fallback web.
    if (completed && torrentToken) await closeTorrentToken(torrentToken);
  }
}

async function play(stream, label, id, meta, subs) {
  hideNext();
  if (isTorrentStream(stream)) {
    await openTorrentSource(stream, label, id, meta, subs);
    return;
  }
  // Ao mudar para uma fonte não-torrent, libera a sessão anterior antes de iniciar outra.
  if (!stream?.torrentToken) await closeActiveTorrent();
  nativeFailure = null;
  // Torrents já abertos são servidos pelo endpoint local do backend e nunca passam
  // pelo proxy HTTP público. Apenas fontes HTTP/HTTPS externas usam o proxy de mídia.
  if (window.CineHubLib.shouldUseMediaProxy(stream)) {
    try {
      const originalUrl = stream.url;
      stream = {
        ...stream,
        mimeType: stream.mimeType || (/\.m3u8(\?|$)/i.test(originalUrl) ? 'application/x-mpegURL' : ''),
        url: await call('media_proxy_url', {
          url: originalUrl,
          behaviorHints: stream.behaviorHints || null,
          mimeType: stream.mimeType || null,
        }),
      };
    } catch (e) {
      toast(`Fonte não pode ser reproduzida com segurança: ${describeError(e)}`);
      return;
    }
  }
  const segments = await skipSegmentsFor(id, meta);
  // MKV não é suportado de forma consistente pelo WebView2: no Windows usa libmpv como fallback.
  // MP4/WebM continuam no player integrado WebView2; Android continua no ExoPlayer.
  if (IS_WINDOWS && isMkvStream(stream) && await playNative(stream, label, id, meta, subs, segments)) return;
  if (!IS_WINDOWS && await playNative(stream, label, id, meta, subs, segments)) return;

  saveProgress(true);
  const seq = ++playSeq;
  skipSegs = segments;
  hideSkip();
  current = { id, label, meta, src: stream.url, mime: stream.mimeType || '' };

  const src = { src: stream.url };
  if (stream.mimeType === 'application/x-mpegURL' || /\.m3u8(\?|$)/i.test(stream.url)) src.type = 'application/x-mpegURL';
  clearTextTracks();
  vjsPlayer.src(src);
  const webSubtitle = pickSubtitles(stream, subs)[0];
  if (webSubtitle) addSubtitle(webSubtitle.url, seq);

  const saved = store.get('progress', {})[id];
  // Um único handler por vez: sem isso, cada vídeo novo empilha outro listener.
  if (resumeHandler) vjsPlayer.off('loadedmetadata', resumeHandler);
  resumeHandler = () => {
    if (seq !== playSeq) return;
    if (saved && !saved.watched && saved.t > 5 && saved.t < vjsPlayer.duration() - 15) vjsPlayer.currentTime(saved.t);
  };
  vjsPlayer.one('loadedmetadata', resumeHandler);

  document.title = `${label} · CineHub`;
  showPlayer();
  keepScreenOn(true);
  vjsPlayer.play().catch(() => {});
}

function showPlayer() {
  if (!history.state?.player) history.pushState({ player: true }, '', location.href);
  $('#player-title').textContent = current?.label || '';
  player.classList.remove('hidden');
}

// Mantém a tela ligada enquanto o vídeo está aberto (quando o aparelho permite).
let wakeLock = null;
async function keepScreenOn(on) {
  try {
    if (on && 'wakeLock' in navigator && !wakeLock) {
      wakeLock = await navigator.wakeLock.request('screen');
      wakeLock.addEventListener('release', () => (wakeLock = null));
    } else if (!on && wakeLock) {
      await wakeLock.release();
      wakeLock = null;
    }
  } catch {
    wakeLock = null; // sem suporte ou negado: o app segue normalmente
  }
}
// Ao voltar para o app, o sistema pode ter soltado a tela ligada: pede de novo.
document.addEventListener('visibilitychange', () => {
  if (!document.hidden && !player.classList.contains('hidden')) keepScreenOn(true);
});

// Legendas: as da própria fonte + as de addons de legenda, só em português.
function pickSubtitles(stream, subs) {
  return window.CineHubLib.selectSubtitles(stream, subs, subPref());
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
// `pos` (em segundos) é usado pelo player nativo; sem ele, lê o player web.
// ---- Pular abertura, recapitulação e créditos (tempos das bases comunitárias, consultados pelo backend)
const skipPref = () => store.get('skipSegments', 'on') !== 'off';
let skipSegs = [];
let skipKey = null;
let skipEnd = null;
const skipBtn = $('#skip-seg');

// Espera no máximo 3 s pelos tempos: sem resposta o vídeo toca normalmente, só sem o botão.
async function skipSegmentsFor(id, meta) {
  const mediaId = meta && (meta.seriesId || meta.id);
  if (!skipPref() || !mediaId) return [];
  const timeout = new Promise((resolve) => setTimeout(() => resolve({ segments: [] }), 3000));
  try {
    const res = await Promise.race([
      call('skip_segments', { kind: meta.type || 'movie', id: mediaId, videoId: id }),
      timeout,
    ]);
    return Array.isArray(res?.segments) ? res.segments : [];
  } catch (_) {
    return [];
  }
}
function hideSkip() {
  skipKey = null;
  skipEnd = null;
  skipBtn.classList.add('hidden');
}
function updateSkip() {
  const seg = activeSegment(skipSegs, vjsPlayer.currentTime());
  const key = seg ? `${seg.type}:${seg.end}` : null;
  if (key === skipKey) return;
  skipKey = key;
  skipEnd = seg ? seg.end : null;
  if (!seg) {
    skipBtn.classList.add('hidden');
    return;
  }
  skipBtn.textContent = `${SKIP_LABELS[seg.type] || 'Pular'} ▶`;
  skipBtn.classList.remove('hidden');
}
skipBtn.onclick = () => {
  if (skipEnd !== null) vjsPlayer.currentTime(skipEnd);
  hideSkip();
};

// Atalhos do player: 0–9 pulam para 0–90% do vídeo; ? mostra a lista.
document.addEventListener('keydown', (e) => {
  if (player.classList.contains('hidden') || e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
  if (e.key === '?') {
    toast('0–9: pular para 0–90% do vídeo · ?: esta lista', 6000);
    return;
  }
  const at = seekFraction(e.key, vjsPlayer.duration());
  if (at !== null) {
    e.preventDefault();
    vjsPlayer.currentTime(at);
  }
});

// Picture-in-picture: o vídeo segue numa janela flutuante enquanto o usuário usa outras telas.
const pipBtn = $('#pip');
const videoEl = $('#video');
if (document.pictureInPictureEnabled && typeof videoEl.requestPictureInPicture === 'function') pipBtn.hidden = false;
pipBtn.onclick = async () => {
  try {
    if (document.pictureInPictureElement) await document.exitPictureInPicture();
    else await videoEl.requestPictureInPicture();
  } catch (_) {
    toast('Não foi possível abrir a janela flutuante neste aparelho.');
  }
};

function saveProgress(force, pos) {
  if (!current) return;
  const now = Date.now();
  if (!force && now - lastSave < 5000) return;
  const t = pos ? pos.t : vjsPlayer.currentTime() || 0;
  if (t < 5) return;
  lastSave = now;

  const duration = pos ? pos.duration : vjsPlayer.duration() || 0;
  const all = store.get('progress', {});
  const meta = current.meta || {};
  all[current.id] = {
    t: Math.floor(t),
    duration: Math.floor(duration),
    label: current.label,
    seriesId: meta.seriesId || meta.id || current.id,
    type: meta.type,
    name: meta.name || current.label,
    poster: meta.poster || '',
    updated: now,
    watched: all[current.id]?.watched === true,
  };
  const kept = Object.entries(all)
    .sort((a, b) => (b[1].updated || 0) - (a[1].updated || 0))
    .slice(0, MAX_PROGRESS);
  store.set('progress', Object.fromEntries(kept));
}

vjsPlayer.on('timeupdate', () => {
  saveProgress(false);
  updateSkip();
});
vjsPlayer.on('pause', () => saveProgress(true));
// Terminou: marca como assistido (sai de "Continuar assistindo" e vira o selo nos cartazes)
// e oferece o próximo episódio, quando há.
vjsPlayer.on('ended', () => {
  markWatched(current?.id);
  offerNext();
});

// Próximo episódio: o botão aparece no player e busca/toca a melhor fonte dele.
let nextUp = null; // { ep, meta } do próximo episódio oferecido
function offerNext() {
  const btn = $('#next-ep');
  const next = current ? nextEpisode(current.meta?.videos, current.id) : null;
  nextUp = next ? { ep: next, meta: current.meta } : null;
  if (!nextUp) {
    btn.classList.add('hidden');
    return;
  }
  btn.textContent = `Próximo: ${next.episode ?? ''}. ${next.title || next.name || 'Episódio'} ▶`;
  btn.classList.remove('hidden');
}
function hideNext() {
  nextUp = null;
  $('#next-ep').classList.add('hidden');
}
$('#next-ep').onclick = () => playNextEpisode();

async function playNextEpisode() {
  if (!nextUp) return;
  const { ep, meta } = nextUp;
  const epKind = meta.type === 'anime' ? 'anime' : 'series';
  hideNext();
  const label = `${meta.name} · T${ep.season ?? 0}E${ep.episode ?? ''}`;
  try {
    const { streams = [], subtitles = [] } = await withBusy(
      'Buscando fontes do próximo episódio',
      call('streams', { kind: epKind, id: ep.id }),
    );
    const pick = sortStreams(Array.isArray(streams) ? streams : []).find((s) => s.url || s.externalUrl || s.infoHash || s.hash);
    if (!pick) {
      toast('Sem fontes para o próximo episódio.');
      return;
    }
    await play(pick, label, ep.id, { ...meta, type: epKind, seriesId: meta.id, name: label }, subtitles);
  } catch (err) {
    toast(`Não consegui abrir o próximo episódio: ${describeError(err)}`);
  }
}
function markWatched(id) {
  setWatched(id, true);
}

// Marca ou desmarca um título/episódio como assistido, sem apagar o tempo já salvo.
function setWatched(id, on) {
  if (!id) return;
  const all = store.get('progress', {});
  all[id] = { ...(all[id] || { t: 0, duration: 0, label: '', seriesId: id, type: '', name: '', poster: '' }), watched: on === true, updated: Date.now() };
  const kept = Object.entries(all)
    .sort((a, b) => num(b[1]?.updated) - num(a[1]?.updated))
    .slice(0, MAX_PROGRESS);
  store.set('progress', Object.fromEntries(kept));
}
vjsPlayer.on('error', () => {
  if (!current) return;
  const err = vjsPlayer.error();
  console.warn('[player] erro', err?.code, err?.message, current.src);
  const reason = describePlaybackError(err, current.src, current.mime);
  logEvent('ERROR', `reprodução falhou (código ${err?.code ?? '?'}): ${reason}`);
  toast(nativeFailure ? `${reason} (player nativo: ${nativeFailure})` : reason);
});

function closePlayer(fromHistory = false) {
  if (player.classList.contains('hidden')) return;
  // Esconde primeiro: se salvar o progresso falhar, o usuário ainda consegue sair do vídeo.
  player.classList.add('hidden');
  hideSkip();
  try {
    saveProgress(true);
  } catch (e) {
    console.warn('progresso não salvo:', e);
  }
  try {
    vjsPlayer.pause();
    clearTextTracks();
  } catch (e) {
    console.warn('falha ao limpar o player:', e);
  }
  void closeActiveTorrent();
  current = null;
  hideNext();
  $('#player-title').textContent = '';
  keepScreenOn(false);
  document.title = 'CineHub';
  if (!fromHistory && history.state?.player) history.back();
}

// Fila de títulos com progresso parcial, do mais recente ao mais antigo.
function continueWatchingRow() {
  const items = Object.values(store.get('progress', {}))
    .filter((p) => p && !p.watched && p.duration > 0 && p.t > 5 && p.t < p.duration - 60)
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

// ---------- Configurações do aplicativo ----------
let torrentStatsTimer = null;
function formatBytes(value) {
  const n = Math.max(0, Number(value) || 0);
  if (n < 1024) return `${n.toFixed(0)} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(1)} ${units[i]}`;
}
function formatRate(value) { return `${formatBytes(value)}/s`; }
async function settingsPage() {
  if (torrentStatsTimer !== null) clearInterval(torrentStatsTimer);
  torrentStatsTimer = null;
  const enabled = store.get('showTorrentStats', false) === true;
  setView(`
    <section class="settings-page">
      <header class="settings-heading">
        <div><p class="eyebrow">CINEHUB</p><h1>Configurações</h1><p class="muted">Ajuste a experiência do aplicativo neste aparelho.</p></div>
      </header>
      <section class="settings-section">
        <h2>Aparência</h2>
        <label class="prefrow">Tema
          <select id="theme-pref">
            <option value="system"${themePref() === 'system' ? ' selected' : ''}>Automático</option>
            <option value="dark"${themePref() === 'dark' ? ' selected' : ''}>Escuro</option>
            <option value="light"${themePref() === 'light' ? ' selected' : ''}>Claro</option>
          </select>
        </label>
      </section>
      <section class="settings-section">
        <h2>Reprodução</h2>
        <label class="prefrow">Idioma das legendas
          <select id="sub-lang">
            <option value="pt"${subPref() === 'pt' ? ' selected' : ''}>Português</option>
            <option value="en"${subPref() === 'en' ? ' selected' : ''}>Inglês</option>
            <option value="off"${subPref() === 'off' ? ' selected' : ''}>Sem legenda</option>
          </select>
        </label>
        <label class="prefrow">Pular abertura e créditos
          <select id="skip-pref">
            <option value="on"${skipPref() ? ' selected' : ''}>Ligado</option>
            <option value="off"${skipPref() ? '' : ' selected'}>Desligado</option>
          </select>
        </label>
      </section>
      <section class="settings-section">
        <h2>Rede e torrent</h2>
        <label class="setting-toggle" for="torrent-stats-pref">
          <span><strong>Estatísticas de rede do torrent</strong><small>Exibe velocidade de download e upload, peers conectados e progresso do torrent.</small></span>
          <input id="torrent-stats-pref" type="checkbox"${enabled ? ' checked' : ''}>
        </label>
        <div id="torrent-stats-panel" class="torrent-stats-panel"${enabled ? '' : ' hidden'}>
          <div class="stats-panel-head"><strong>Atividade do torrent</strong><button id="torrent-stats-refresh" class="chip small" type="button">Atualizar</button></div>
          <div id="torrent-stats-content" class="muted small" role="status" aria-live="polite">Consultando motor torrent…</div>
        </div>
        <p class="muted small">As estatísticas são consultadas somente enquanto esta opção está ligada e a tela de configurações está aberta.</p>
      </section>
      <section class="settings-section">
        <h2>Dados e privacidade</h2>
        <p class="muted small">Biblioteca, progresso e preferências ficam armazenados localmente neste aparelho.</p>
        <div class="dataactions">
          <button id="open-library" class="chip" type="button">Abrir biblioteca</button>
          <button id="export-diag" class="chip" type="button">Exportar diagnóstico</button>
        </div>
      </section>
    </section>`);
  $('#theme-pref').onchange = (e) => { store.set('theme', e.target.value); applyTheme(e.target.value); };
  $('#sub-lang').onchange = (e) => store.set('subLang', e.target.value);
  $('#skip-pref').onchange = (e) => store.set('skipSegments', e.target.value);
  $('#open-library').onclick = () => { location.hash = '#/library'; };
  $('#export-diag').onclick = exportDiagnostics;
  const toggle = $('#torrent-stats-pref');
  const panel = $('#torrent-stats-panel');
  const content = $('#torrent-stats-content');
  const refresh = async () => {
    if (!toggle.checked || !content.isConnected) return;
    try {
      const s = await call('torrent_stats');
      if (!content.isConnected || !toggle.checked) return;
      if (!s.active) {
        content.innerHTML = '<p>Nenhum torrent ativo no momento. Abra um torrent para acompanhar a rede.</p>';
        return;
      }
      const pct = s.totalBytes > 0 ? Math.min(100, s.downloadedBytes / s.totalBytes * 100) : 0;
      content.innerHTML = `
        <div class="torrent-stat-grid">
          <div><span>Download</span><strong>${esc(formatRate(s.downloadSpeedBytes))}</strong></div>
          <div><span>Upload</span><strong>${esc(formatRate(s.uploadSpeedBytes))}</strong></div>
          <div><span>Peers conectados</span><strong>${esc(s.peers)}</strong></div>
          <div><span>Peers descobertos</span><strong>${esc(s.peersSeen)}</strong></div>
          <div><span>Baixado</span><strong>${esc(formatBytes(s.downloadedBytes))} / ${esc(formatBytes(s.totalBytes))}</strong></div>
          <div><span>Estado</span><strong>${esc(s.state || '—')}</strong></div>
        </div>
        <div class="stats-progress" role="progressbar" aria-label="Progresso do torrent" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct.toFixed(1)}"><span style="width:${pct.toFixed(1)}%"></span></div>
        ${s.error ? `<p class="muted small">${esc(s.error)}</p>` : ''}`;
    } catch (err) {
      if (content.isConnected) content.textContent = `Não foi possível obter as estatísticas: ${err.message}`;
    }
  };
  toggle.onchange = () => {
    store.set('showTorrentStats', toggle.checked);
    panel.hidden = !toggle.checked;
    if (torrentStatsTimer !== null) clearInterval(torrentStatsTimer);
    torrentStatsTimer = null;
    if (toggle.checked) { refresh(); torrentStatsTimer = setInterval(refresh, 2000); }
  };
  $('#torrent-stats-refresh').onclick = refresh;
  if (enabled) { refresh(); torrentStatsTimer = setInterval(refresh, 2000); }
}

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
  if (i < 0 && list.length >= MAX_FAVS) {
    toast(`A biblioteca já tem ${MAX_FAVS} títulos. Remova algum para adicionar outro.`);
    return;
  }
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

// Título da aba por seção (o título de um detalhe ou de um player é definido na própria página).
const PAGE_TITLES = {
  home: 'Início',
  movies: 'Filmes',
  series: 'Séries',
  animes: 'Animes',
  search: 'Pesquisar',
  library: 'Biblioteca',
  addons: 'Addons',
  settings: 'Configurações',
};

const NAV = {
  home: (alive) => homePage(alive),
  movies: (alive) => catalogPage('movie', 'Filmes', alive),
  series: (alive) => catalogPage('series', 'Séries', alive),
  animes: (alive) => catalogPage('anime', 'Animes', alive),
  search: () => searchPage(new URLSearchParams(location.hash.split('?')[1] || '').get('q') || ''),
  library: () => libraryPage(),
  settings: () => settingsPage(),
  addons: () => addonsPage(),
};

let routeSeq = 0;
async function route() {
  const seq = ++routeSeq;
  const alive = () => seq === routeSeq;
  clearTimeout(debounceTimer);
  if (sectionOf() !== 'settings' && torrentStatsTimer !== null) {
    clearInterval(torrentStatsTimer);
    torrentStatsTimer = null;
  }

  const path = location.hash.replace(/^#\/?/, '').split('?')[0];
  const [section, kind, id] = path.split('/');
  const active = section === 'detail' ? '' : section || 'home';
  document.body.classList.toggle('search-active', active === 'search');
  document.body.classList.toggle('animes-active', active === 'animes');
  if (player.classList.contains('hidden') && section !== 'detail') {
    document.title = PAGE_TITLES[active] ? `${PAGE_TITLES[active]} · CineHub` : 'CineHub';
  }
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
// "/" foca a pesquisa (fora de campos de texto).
document.addEventListener('keydown', (e) => {
  if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return;
  const el = document.activeElement;
  const typing = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el?.isContentEditable;
  if (typing || !player.classList.contains('hidden') || confirmDialog.open) return;
  e.preventDefault();
  const target = sectionOf() === 'animes' ? $('#anime-search-input') : topQ;
  target?.focus();
  target?.select();
});

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

logEvent('INFO', `app iniciado (${IS_ANDROID ? 'android' : 'desktop'})`);
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

  // Em campos de texto, esquerda/direita movem o cursor; a navegação por setas fica para cima/baixo.
  const typing = document.activeElement;
  const inTextField =
    typing instanceof HTMLTextAreaElement ||
    (typing instanceof HTMLInputElement && !['button', 'checkbox', 'radio', 'file', 'range'].includes(typing.type));
  if (inTextField && (dir === 'left' || dir === 'right')) return;

  const list = focusables();
  if (!list.length) return;
  const focused = document.activeElement;
  e.preventDefault();

  if (!list.includes(focused)) {
    list[0].focus();
    return;
  }

  const r = focused.getBoundingClientRect();
  const cx = r.left + r.width / 2;
  const cy = r.top + r.height / 2;
  const horizontal = dir === 'left' || dir === 'right';

  let best = null;
  let bestScore = Infinity;
  for (const el of list) {
    if (el === focused) continue;
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
