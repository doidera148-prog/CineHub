// Teste de fumaça da interface: rotas, ações básicas e navegação de teclado/controle remoto.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const WEB = resolve(fileURLToPath(new URL('../../web/', import.meta.url)));
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png', '.woff': 'font/woff', '.woff2': 'font/woff2', '.json': 'application/json' };
const server = createServer(async (req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname));
  const file = join(WEB, path === '/' ? 'index.html' : path);
  if (!file.startsWith(WEB)) { console.error(`Recurso bloqueado pelo servidor de teste: ${req.url} -> ${file}`); return res.writeHead(403).end(); }
  let body;
  try { body = await readFile(file); } catch { return res.writeHead(404).end(); }
  res.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream' });
  res.end(body);
});
await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen));
const base = `http://127.0.0.1:${server.address().port}`;
const fixtures = `window.__TAURI__={core:{invoke:async(cmd,args)=>{const metas=(n,p)=>Array.from({length:n},(_,i)=>({id:'tt'+p+i,type:'movie',name:p+' filme '+i,releaseInfo:'2026'}));switch(cmd){case'home':return{rows:[{title:'Populares',addon:'Teste',transportUrl:'https://test.invalid/manifest.json',type:args.kind,catalogId:'top',metas:metas(20,'home')}]};case'search':return{rows:[]};case'catalog_page':return{metas:[]};case'list_addons':return[];case'meta':return{meta:{id:args.id,type:'movie',name:'Título de teste',description:'Descrição'}};case'streams':if(args.id==='tt2')throw Error('offline');return{streams:[],subtitles:[],errors:[]};case'subtitle':return'WEBVTT';case'torrent_stats':return{active:true,state:'Live',downloadSpeedBytes:1250000,uploadSpeedBytes:64000,peers:8,peersSeen:14,downloadedBytes:500000000,totalBytes:1000000000,error:null};default:return{};}}}};`;
const routes = ['#/', '#/movies', '#/series', '#/animes', '#/search?q=filme', '#/library', '#/addons', '#/detail/movie/tt1'];
const failures = [];
const browser = await chromium.launch();
async function check(viewport, label) {
  const page = await browser.newPage({ viewport });
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('response', (response) => { if (response.status() === 403) errors.push(`HTTP 403: ${response.url()}`); });
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
  await page.addInitScript(fixtures);
  for (const hash of routes) {
    await page.goto(`${base}/${hash}`);
    await page.waitForTimeout(300);
    const text = await page.locator('#view').innerText();
    if (/Erro:/.test(text)) failures.push(`[${label}] ${hash}: erro visível`);
    if (await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth)) failures.push(`[${label}] ${hash}: overflow horizontal`);
  }
  await page.goto(`${base}/#/settings`);
  await page.waitForTimeout(150);
  if (!(await page.locator('#view h1').innerText()).includes('Configurações')) failures.push(`[${label}] rota de configurações não abriu`);
  const statsToggle = page.locator('#torrent-stats-pref');
  if (!(await statsToggle.count())) failures.push(`[${label}] opção de estatísticas torrent ausente`);
  else {
    await statsToggle.check();
    await page.waitForTimeout(150);
    if (!(await page.locator('#torrent-stats-content').innerText()).includes('1.2 MB/s')) failures.push(`[${label}] estatísticas torrent não foram exibidas`);
    await statsToggle.uncheck();
    if (!(await page.locator('#torrent-stats-panel').isHidden())) failures.push(`[${label}] painel não foi ocultado ao desligar a opção`);
  }
  await page.goto(`${base}/#/movies`);
  await page.waitForTimeout(300);
  const beforeSkip = await page.evaluate(() => location.hash);
  await page.locator('.skip-link').focus();
  await page.keyboard.press('Enter');
  if (await page.evaluate(() => location.hash) !== beforeSkip) failures.push(`[${label}] atalho acessível alterou a rota`);
  if (await page.evaluate(() => document.activeElement?.id) !== 'view') failures.push(`[${label}] atalho acessível não focou o conteúdo`);
  const cards = page.locator('#view .strip a');
  await cards.first().focus();
  await page.keyboard.press('ArrowRight');
  if (await page.evaluate(() => document.activeElement?.getAttribute('aria-label')) !== await cards.nth(1).getAttribute('aria-label')) failures.push(`[${label}] setas não navegam entre cards`);
  await cards.first().focus();
  await page.keyboard.press('Enter');
  await page.waitForTimeout(350);
  if (!(await page.evaluate(() => location.hash.startsWith('#/detail/')))) failures.push(`[${label}] Enter não ativou o card focado`);
  if (label === 'desktop') {
    await page.goto(`${base}/#/movies`); await page.waitForTimeout(250);
    await page.locator('body').click({ position: { x: 2, y: 2 } }); await page.keyboard.press('/');
    if (await page.evaluate(() => document.activeElement?.id) !== 'q') failures.push('[desktop] atalho / não focou busca');
  }
  if (label === 'celular') {
    // Página nova: o teste acima usou teclado, e isso liga o modo controle remoto (foco vai para a lista).
    const fresh = await browser.newPage({ viewport });
    await fresh.addInitScript(fixtures);
    await fresh.goto(`${base}/#/movies`); await fresh.waitForTimeout(250);
    if (await fresh.evaluate(() => document.activeElement?.tagName) !== 'BODY') failures.push('[celular] foco inicial não esperado');
    await fresh.close();
  }
  failures.push(...errors.map((e) => `[${label}] ${e}`));
  await page.close();
}

async function checkBackupImport() {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.addInitScript(fixtures);
  await page.goto(`${base}/#/library`);
  await page.waitForTimeout(200);
  const valid = JSON.stringify({
    app: 'CineHub', version: 1, exportedAt: '2026-11-18T00:00:00.000Z',
    favs: [{ id: 'backup:movie', type: 'movie', name: 'Backup de teste', poster: '', releaseInfo: '' }],
    progress: { 'backup:movie': { t: 30, duration: 100, updated: 10, name: 'Backup de teste' } },
  });
  await page.locator('#import-file').setInputFiles({ name: 'backup.json', mimeType: 'application/json', buffer: Buffer.from(valid) });
  await page.waitForFunction(() => document.querySelector('#toast')?.textContent.includes('Dados importados.'));
  const saved = await page.evaluate(() => ({ favs: JSON.parse(localStorage.getItem('favs') || '[]'), progress: JSON.parse(localStorage.getItem('progress') || '{}') }));
  if (!saved.favs.some((item) => item.id === 'backup:movie')) errors.push('[backup] favorito não foi persistido após importação');
  if (saved.progress['backup:movie']?.t !== 30) errors.push('[backup] progresso não foi persistido após importação');

  const invalid = JSON.stringify({ app: 'CineHub', version: 999, favs: [], progress: {} });
  await page.locator('#import-file').setInputFiles({ name: 'backup-invalido.json', mimeType: 'application/json', buffer: Buffer.from(invalid) });
  await page.waitForFunction(() => document.querySelector('#toast')?.textContent.includes('Não consegui importar:'));
  const afterInvalid = await page.evaluate(() => ({ favs: JSON.parse(localStorage.getItem('favs') || '[]'), progress: JSON.parse(localStorage.getItem('progress') || '{}') }));
  if (JSON.stringify(afterInvalid) !== JSON.stringify(saved)) errors.push('[backup] backup inválido alterou os dados existentes');
  await page.close();
  if (errors.length) failures.push(...errors);
}

async function checkTv() {
  const page = await browser.newPage({
    viewport: { width: 1280, height: 720 },
    userAgent: 'Mozilla/5.0 (Linux; Android 11; Android TV) AppleWebKit/537.36 Chrome/120.0 Safari/537.36',
  });
  await page.addInitScript(fixtures);
  await page.addInitScript(() => {
    const original = window.matchMedia.bind(window);
    window.matchMedia = (query) => {
      if (query.includes('(pointer: none)') || query.includes('(hover: none)')) {
        return { matches: true, media: query, onchange: null, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}, dispatchEvent() { return false; } };
      }
      return original(query);
    };
  });
  await page.goto(`${base}/#/movies`);
  await page.waitForTimeout(350);
  if (!(await page.locator('body').evaluate((el) => el.classList.contains('tv-mode')))) failures.push('[tv] interface dedicada não foi ativada');
  if (await page.evaluate(() => document.querySelector('#nav-main a:last-child')?.getBoundingClientRect().bottom > innerHeight - 8)) failures.push('[tv] menu lateral ultrapassa a tela 720p');
  const first = page.locator('#view .strip a').first();
  await first.focus();
  await page.keyboard.press('ArrowLeft');
  if (await page.evaluate(() => document.activeElement?.closest('.strip') === document.querySelector('.strip'))) failures.push('[tv] controle ficou preso na borda do trilho');
  await page.close();
}
try { await check({ width: 360, height: 800 }, 'celular'); await check({ width: 1280, height: 800 }, 'desktop'); await checkTv(); await checkBackupImport(); }
finally { await browser.close(); server.close(); }
if (failures.length) { console.error('Falhas:\n- ' + failures.join('\n- ')); process.exit(1); }
console.log('Smoke test ok: rotas, acessibilidade e navegação em celular, desktop e Android TV.');
