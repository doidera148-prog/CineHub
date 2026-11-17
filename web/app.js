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
};

const setView = (html) => (view.innerHTML = html);
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
function withBusy(text, promise) {
  const started = Date.now();
  const render = () => {
    const secs = Math.floor((Date.now() - started) / 1000);
    busyEl.innerHTML = `<span class="spinner" aria-hidden="true"></span><span>${esc(text)} <span class="muted small">${secs} s</span></span>`;
  };
  render();
  busyEl.classList.remove('hidden');
  const timer = setInterval(render, 1000);
  return promise.finally(() => {
    clearInterval(timer);
    busyEl.classList.add('hidden');
  });
}
const TORRENT_BUSY = 'Preparando torrent (pode levar até 1 minuto)';

function askConfirm(msg, okLabel) {
  return new Promise((resolve) => {
    $('#confirm-msg').textContent = msg;
    $('#confirm-ok').textContent = okLabel;
    confirmDialog.addEventListener('close', () => resolve(confirmDialog.returnValue === 'ok'), { once: true });
    confirmDialog.showModal();
  });
}

// Funções puras (URLs seguras, ordenação de fontes, próximo episódio) vêm de lib.js, testadas com node.
const { isWebUrl, cssUrl, str, num, cleanFav, cleanProgress, progressPercent, sortStreams, nextEpisode, torrentLanguage, isSeriesLike, typeLabel } = window.CineHubLib;

// Abre um link externo no navegador do aparelho (window.open não funciona no WebView do Android).
async function openExternal(url) {
  try {
    await call('plugin:opener|open_url', { url });
  } catch (err) {
    toast(`Não consegui abrir o link: ${err.message}`);
  }
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
    const result = await withBusy(
      TORRENT_BUSY,
      call('torrent_open', {
        magnet,
        torrentBytes: null,
        fileIndex: Number.isInteger(stream?.fileIndex) ? stream.fileIndex : null,
      }),
    );
    const chosen = await pickTorrentVideo(result);
    const fixed = { ...stream, url: chosen.streamUrl, mimeType: chosen.mimeType, name: chosen.name || stream?.name || label };
    await play(fixed, label, id, meta, subs);
  } catch (err) {
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
    dlg.addEventListener('close', () => { dlg.remove(); resolve(chosen || result); }, { once: true });
    dlg.showModal();
  });
}

// Reproduz um torrent recém-aberto pelo backend (magnet ou arquivo .torrent).
async function playOpenedTorrent(input, fallbackName) {
  const result = await pickTorrentVideo(input);
  const name = result.name || fallbackName;
  const id = `torrent:${Date.now()}`;
  const fake = { url: result.streamUrl, name, fileName: result.fileName, mimeType: result.mimeType };
  await play(fake, name, id, { id, label: name, name, type: 'movie' }, []);
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

// Destaque no topo da tela inicial: o primeiro título com imagem disponível nos catálogos.
function renderBillboard(rows) {
  const box = $('#billboard');
  if (!box) return;
  const m = rows.flatMap((r) => r.metas || []).find((x) => isWebUrl(x.background) || isWebUrl(x.poster));
  if (!m) {
    box.remove();
    return;
  }
  const bg = isWebUrl(m.background) ? m.background : m.poster;
  const kindLabel = typeLabel(m.type) || 'Filme';
  const info = [m.releaseInfo, kindLabel].filter(Boolean).join(' · ');
  const href = `#/detail/${esc(m.type)}/${encodeURIComponent(m.id)}`;
  box.className = 'billboard';
  box.style.backgroundImage = cssUrl(bg);
  box.innerHTML = `
    <div class="billboard-info">
      <h2>${esc(m.name)}</h2>
      <p class="meta-line">${esc(info)}</p>
      ${m.description ? `<p>${esc(m.description)}</p>` : ''}
      <div class="actions">
        <a class="btn-link btn-light" href="${href}">▶ Assistir</a>
        <a class="btn-link btn-grey" href="${href}">ⓘ Mais informações</a>
      </div>
    </div>`;
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
  const payload = {
    app: 'CineHub',
    version: 1,
    exportedAt: new Date().toISOString(),
    favs: getFavs(),
    progress: store.get('progress', {}),
  };
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
const SUB_LANGS = { pt: /^(por|pt)/i, en: /^(eng|en)/i, off: null };
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
      <input id="tm" placeholder="Cole um magnet: ou use um arquivo .torrent" aria-label="Link magnet do torrent">
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
      logEvent('WARN', `instalar addon falhou: ${err.message}`);
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
        // Mesmo limite do backend (8 MiB), checado antes de enviar pelo IPC.
        if (file.size > 8 * 1024 * 1024) {
          toast('Arquivo .torrent maior que 8 MiB.');
          return;
        }
        const bytes = new Uint8Array(await file.arrayBuffer());
        const result = await withBusy(TORRENT_BUSY, call('torrent_open', { magnet: null, torrentBytes: Array.from(bytes), fileIndex: null }));
        await playOpenedTorrent(result, file.name);
        return;
      }
      if (!magnet) {
        toast('Cole um magnet ou selecione um .torrent.');
        return;
      }
      const result = await withBusy(TORRENT_BUSY, call('torrent_open', { magnet, torrentBytes: null, fileIndex: null }));
      await playOpenedTorrent(result, 'Torrent');
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

  document.title = `${meta.name || 'Título'} · CineHub`;
  const bg = meta.background || meta.poster || '';
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

  if (isSeriesLike(meta.type || kind)) renderSeries(meta);
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
      if (box) box.innerHTML = `<p class="muted">Não consegui buscar as fontes: ${esc(err.message)}</p>`;
    }
    return;
  }
  if (seq !== streamsSeq) return;
  const { subtitles = [], errors = [] } = result;
  // Fontes que tocam vêm primeiro, depois as de maior resolução (e dublagem em português).
  const streams = sortStreams(Array.isArray(result.streams) ? result.streams : []);
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
    box.innerHTML = '<p class="muted">Nenhuma fonte encontrada. Instale um addon de streams compatível na aba Addons.</p>';
    showErrors();
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
      b.onclick = () => openExternal(s.externalUrl);
    }
    box.append(b);
  }
  // Botão de destaque: assiste à melhor fonte (a primeira reproduzível, já ordenada).
  const best = box.querySelector('.stream:not(.disabled)');
  if (best) {
    const primary = document.createElement('button');
    primary.className = 'primary-play';
    primary.textContent = '▶ Assistir';
    primary.onclick = () => best.click();
    box.prepend(primary);
  }
  showErrors();
}

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
let playSeq = 0; // muda a cada fonte; legendas de fontes antigas são descartadas
let blobUrls = [];
let lastSave = 0;
let resumeHandler = null; // retomada da posição salva no próximo loadedmetadata

// Player nativo: ExoPlayer no Android, libmpv no desktop. Se o nativo falhar, usa o player web.
// Se o nativo não estiver disponível (ex.: build sem a feature `mpv`), cai no player do navegador.
const IS_ANDROID = /Android/i.test(navigator.userAgent);

// Legenda em português como data: URI (VTT), para o ExoPlayer carregar sem arquivo local.
async function subtitleDataUri(stream, subs) {
  const sub = pickSubtitles(stream, subs)[0];
  if (!sub) return null;
  try {
    const vtt = await call('subtitle', { url: sub.url });
    const bytes = new TextEncoder().encode(vtt);
    let binary = '';
    for (const b of bytes) binary += String.fromCharCode(b);
    return 'data:text/vtt;base64,' + btoa(binary);
  } catch (e) {
    console.warn('legenda indisponível para o player nativo:', e);
    return null;
  }
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

async function playNative(stream, label, id, meta, subs) {
  const saved = store.get('progress', {})[id];
  const startMs = saved && saved.t > 5 && !saved.watched ? saved.t * 1000 : 0;
  current = { id, label, meta, src: stream.url, mime: '' };
  document.title = `${label} · CineHub`;
  try {
    const command = IS_ANDROID ? 'plugin:exoplayer|play' : 'player_play';
    const args = { url: stream.url, startMs };
    // Android: legenda como data: URI (o ExoPlayer carrega direto). Desktop: URL, que o
    // backend baixa e entrega ao mpv como arquivo.
    if (IS_ANDROID) args.subtitle = await subtitleDataUri(stream, subs);
    else args.subtitleUrl = pickSubtitles(stream, subs)[0]?.url ?? null;
    const res = await invoke(command, args);
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
  }
}

async function play(stream, label, id, meta, subs) {
  hideNext();
  if (isTorrentStream(stream)) {
    await openTorrentSource(stream, label, id, meta, subs);
    return;
  }
  nativeFailure = null;
  if (await playNative(stream, label, id, meta, subs)) return;

  saveProgress(true);
  const seq = ++playSeq;
  current = { id, label, meta, src: stream.url, mime: stream.mimeType || '' };

  const src = { src: stream.url };
  if (/\.m3u8(\?|$)/i.test(stream.url)) src.type = 'application/x-mpegURL';
  clearTextTracks();
  vjsPlayer.src(src);
  for (const sub of pickSubtitles(stream, subs)) addSubtitle(sub.url, seq);

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
  const re = SUB_LANGS[subPref()];
  if (!re) return [];
  const all = [...(stream.subtitles || []), ...(subs || [])];
  return all.filter((s) => re.test(s.lang || '') && s.url).slice(0, 1);
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

vjsPlayer.on('timeupdate', () => saveProgress(false));
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
};

const NAV = {
  home: (alive) => homePage(alive),
  movies: (alive) => catalogPage('movie', 'Filmes', alive),
  series: (alive) => catalogPage('series', 'Séries', alive),
  animes: (alive) => catalogPage('anime', 'Animes', alive),
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
  topQ.focus();
  topQ.select();
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
