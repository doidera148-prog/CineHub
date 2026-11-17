// Integração manual com a API JSON-RPC de um Kodi na rede local.
// O usuário informa o IP; o CineHub não procura nem varre dispositivos na rede.
(() => {
  const view = document.querySelector('#view');
  if (!view || !window.__TAURI__?.core?.invoke) return;

  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);
  let credentials = { endpoint: '', username: null, password: null };
  let currentDirectory = '';

  function makePanel() {
    if (view.querySelector('#kodi-panel')) return;
    const panel = document.createElement('section');
    panel.id = 'kodi-panel';
    panel.className = 'kodi-panel';
    panel.innerHTML = `
      <h2>Conectar ao Kodi</h2>
      <p class="muted small">Acesse os addons de vídeo instalados em uma instância Kodi sua ou que você esteja autorizado a controlar. A reprodução é feita pelo próprio Kodi.</p>
      <form id="kodi-connect" class="kodi-connect">
        <input id="kodi-endpoint" type="url" placeholder="http://192.168.1.20:8080" aria-label="Endereço IP local e porta do Kodi" required>
        <input id="kodi-user" autocomplete="username" placeholder="Usuário (opcional)" aria-label="Usuário do Kodi">
        <input id="kodi-pass" type="password" autocomplete="current-password" placeholder="Senha (opcional)" aria-label="Senha do Kodi">
        <button type="submit">Conectar</button>
      </form>
      <p class="muted small">Ative o controle remoto HTTP nas configurações do Kodi. Por segurança, o CineHub aceita somente IPs locais/privados e não salva a senha.</p>
      <div id="kodi-status" class="muted small" role="status" aria-live="polite"></div>
      <div id="kodi-results"></div>`;
    const heading = view.querySelector('h1');
    if (heading) heading.insertAdjacentElement('afterend', panel);
    else view.prepend(panel);
    panel.querySelector('#kodi-connect').addEventListener('submit', connect);
  }

  async function connect(event) {
    event.preventDefault();
    const panel = view.querySelector('#kodi-panel');
    if (!panel) return;
    credentials = {
      endpoint: panel.querySelector('#kodi-endpoint').value.trim(),
      username: panel.querySelector('#kodi-user').value.trim() || null,
      password: panel.querySelector('#kodi-pass').value || null,
    };
    const status = panel.querySelector('#kodi-status');
    status.textContent = 'Buscando addons de vídeo…';
    try {
      const result = await invoke('kodi_addons', credentials);
      const addons = Array.isArray(result?.addons) ? result.addons : [];
      status.textContent = addons.length ? `${addons.length} addon(s) de vídeo encontrado(s).` : 'Conectado; nenhum addon de vídeo habilitado foi encontrado.';
      renderAddons(addons);
    } catch (error) {
      status.textContent = String(error);
      panel.querySelector('#kodi-results').replaceChildren();
    }
  }

  function renderAddons(addons) {
    const out = view.querySelector('#kodi-results');
    if (!out) return;
    out.replaceChildren();
    addons.forEach((addon) => {
      const card = document.createElement('div');
      card.className = 'kodi-addon';
      const info = document.createElement('div');
      const title = document.createElement('strong');
      title.textContent = addon.name || addon.addonid || 'Addon Kodi';
      const summary = document.createElement('p');
      summary.className = 'muted small';
      summary.textContent = addon.summary || addon.addonid || '';
      info.append(title, summary);
      const browse = document.createElement('button');
      browse.className = 'chip';
      browse.textContent = 'Abrir';
      browse.addEventListener('click', () => browseDirectory(`plugin://${addon.addonid}/`));
      card.append(info, browse);
      out.append(card);
    });
    const listing = document.createElement('div');
    listing.id = 'kodi-listing';
    out.append(listing);
  }

  async function browseDirectory(directory) {
    currentDirectory = directory;
    const listing = view.querySelector('#kodi-listing');
    const status = view.querySelector('#kodi-status');
    if (!listing) return;
    listing.textContent = 'Carregando…';
    try {
      const result = await invoke('kodi_browse', { ...credentials, directory });
      const files = Array.isArray(result?.files) ? result.files : [];
      listing.replaceChildren();
      if (directory !== `plugin://${directory.split('/')[2]}/`) {
        const back = document.createElement('button');
        back.className = 'chip';
        back.textContent = '← Voltar';
        back.addEventListener('click', () => browseDirectory(directory.slice(0, directory.lastIndexOf('/', directory.length - 2) + 1)));
        listing.append(back);
      }
      if (!files.length) listing.append('Nenhum item encontrado.');
      files.forEach((item) => {
        const row = document.createElement('div');
        row.className = 'kodi-item';
        const label = document.createElement('span');
        label.textContent = item.label || item.title || item.file || 'Item';
        row.append(label);
        const action = document.createElement('button');
        action.className = 'chip';
        const isFolder = item.filetype === 'directory' || item.file?.endsWith('/');
        action.textContent = isFolder ? 'Abrir' : 'Reproduzir no Kodi';
        action.addEventListener('click', async () => {
          try {
            if (isFolder) await browseDirectory(item.file);
            else {
              await invoke('kodi_play', { ...credentials, file: item.file });
              status.textContent = 'Reprodução solicitada ao Kodi.';
            }
          } catch (error) {
            status.textContent = String(error);
          }
        });
        row.append(action);
        listing.append(row);
      });
    } catch (error) {
      listing.textContent = String(error);
      status.textContent = 'Falha ao abrir o catálogo do addon Kodi.';
    }
  }

  // A tela de addons é construída por app.js; observar suas alterações permite manter
  // a integração isolada e sem interferir na navegação ou nos addons Stremio existentes.
  const observer = new MutationObserver(() => {
    const route = location.hash.replace(/^#\/?/, '').split('?')[0].split('/')[0];
    if (route === 'addons') makePanel();
  });
  observer.observe(view, { childList: true, subtree: true });
  window.addEventListener('hashchange', () => {
    if (location.hash.replace(/^#\/?/, '').split('?')[0].split('/')[0] === 'addons') setTimeout(makePanel, 0);
  });
  if (location.hash.replace(/^#\/?/, '').split('?')[0].split('/')[0] === 'addons') setTimeout(makePanel, 0);
})();
