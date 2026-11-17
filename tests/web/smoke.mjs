// Teste de fumaça da interface: abre cada rota num navegador real, com o backend Tauri simulado.
// Verifica erros de script, páginas de erro, rolagem horizontal no celular e o foco inicial.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const WEB = resolve(fileURLToPath(new URL('../../web/', import.meta.url)));
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png', '.woff2': 'font/woff2', '.json': 'application/json' };

const server = createServer(async (req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname));
  const file = join(WEB, path === '/' ? 'index.html' : path);
  if (!file.startsWith(WEB)) return res.writeHead(403).end();
  // Lê o arquivo antes de responder: se faltar, o 404 sai uma única vez.
  let body;
  try {
    body = await readFile(file);
  } catch {
    return res.writeHead(404).end();
  }
  res.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream' });
  res.end(body);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

// Dados falsos para cada comando do backend.
const fixtures = `
  window.__TAURI__ = { core: { invoke: async (cmd, args) => {
    const metas = (n, p) => Array.from({ length: n }, (_, i) => ({ id: 'tt' + p + i, type: 'movie', name: p + ' filme ' + i, releaseInfo: '2026' }));
    switch (cmd) {
      case 'home': return { rows: [{ title: 'Populares', addon: 'Addon de teste', transportUrl: 'http://x/manifest.json', type: args.kind, catalogId: 'top', metas: metas(20, 'home') }] };
      case 'search': return { rows: [{ title: 'Busca', addon: 'Addon de teste', metas: metas(3, 'busca') }] };
      case 'catalog_page': return { metas: [] };
      case 'list_addons': return [{ transportUrl: 'http://x/manifest.json', manifest: { name: 'Addon de teste', version: '1.0.0', resources: ['catalog'], types: ['movie'] } }];
      case 'meta': return { meta: { id: args.id, type: 'movie', name: 'Titulo ' + args.id, description: 'Descrição', genres: ['Ação'] } };
      case 'streams':
        // tt2 simula um addon que falha: a tela precisa mostrar o erro, não ficar carregando.
        if (args.id === 'tt2') throw new Error('addon fora do ar');
        return { streams: [{ name: 'Fonte', title: 'Qualidade', addonName: 'Addon de teste', note: null, externalUrl: 'javascript:alert(1)' }], subtitles: [], errors: [] };
      case 'subtitle': return 'WEBVTT';
      default: return {};
    }
  } } };
`;

const routes = ['#/', '#/movies', '#/series', '#/search?q=filme', '#/library', '#/addons', '#/detail/movie/tt1'];
const failures = [];
const browser = await chromium.launch();

async function check(viewport, label) {
  const page = await browser.newPage({ viewport });
  const errors = [];
  page.on('pageerror', (e) => errors.push(`erro de script: ${e.message}`));
  page.on('console', (m) => m.type() === 'error' && errors.push(`console: ${m.text()}`));
  await page.addInitScript(fixtures);

  for (const hash of routes) {
    await page.goto(`${base}/${hash}`);
    await page.waitForTimeout(400);
    const text = await page.locator('#view').innerText();
    if (/Erro:/.test(text)) failures.push(`[${label}] ${hash}: página mostrou erro: ${text.slice(0, 120)}`);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    if (overflow > 0) failures.push(`[${label}] ${hash}: rolagem horizontal de ${overflow}px`);
  }

  // Fontes com erro: a mensagem aparece e o carregamento não fica travado.
  await page.goto(`${base}/#/detail/movie/tt2`);
  await page.waitForTimeout(500);
  const fontes = await page.locator('#view').innerText();
  if (/Buscando fontes/.test(fontes) || !/Não consegui buscar as fontes/.test(fontes)) {
    failures.push(`[${label}] erro de fontes não foi mostrado ao usuário`);
  }

  // Importar backup: adiciona favoritos e guarda o progresso (regra de importação da biblioteca).
  await page.goto(`${base}/#/library`);
  await page.waitForTimeout(300);
  await page.setInputFiles('#import-file', {
    name: 'backup.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify({ app: 'CineHub', version: 1, favs: [{ id: 'tt9', type: 'movie', name: 'Importado' }], progress: { 'tt9:1': { t: 30, duration: 100, updated: 5 } } })),
  });
  await page.waitForTimeout(400);
  const favs = await page.evaluate(() => JSON.parse(localStorage.getItem('favs') || '[]'));
  if (!favs.some((f) => f.id === 'tt9')) failures.push(`[${label}] importação não adicionou favoritos`);

  // Preferência de legenda: a escolha fica guardada neste aparelho.
  await page.selectOption('#sub-lang', 'off');
  const sub = await page.evaluate(() => localStorage.getItem('subLang'));
  if (sub !== '"off"') failures.push(`[${label}] preferência de legenda não foi guardada (${sub})`);

  await page.goto(`${base}/#/movies`);
  await page.waitForTimeout(400);
  const focused = await page.evaluate(() => document.activeElement?.tagName);
  if (label === 'celular' && focused !== 'BODY') failures.push(`[celular] foco inicial indevido em ${focused}`);

  failures.push(...errors.map((e) => `[${label}] ${e}`));
  await page.close();
}

try {
  await check({ width: 360, height: 800 }, 'celular');
  await check({ width: 1280, height: 800 }, 'desktop');
} finally {
  await browser.close();
  server.close();
}

if (failures.length) {
  console.error('Falhas no teste de fumaça:\n- ' + failures.join('\n- '));
  process.exit(1);
}
console.log(`Teste de fumaça ok: ${routes.length} rotas em celular e desktop.`);
