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
  if (!file.startsWith(WEB)) return res.writeHead(403).end();
  let body;
  try { body = await readFile(file); } catch { return res.writeHead(404).end(); }
  res.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream' });
  res.end(body);
});
await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen));
const base = `http://127.0.0.1:${server.address().port}`;
const fixtures = `window.__TAURI__={core:{invoke:async(cmd,args)=>{const metas=(n,p)=>Array.from({length:n},(_,i)=>({id:'tt'+p+i,type:'movie',name:p+' filme '+i,releaseInfo:'2026'}));switch(cmd){case'home':return{rows:[{title:'Populares',addon:'Teste',transportUrl:'https://test.invalid/manifest.json',type:args.kind,catalogId:'top',metas:metas(20,'home')}]};case'search':return{rows:[]};case'catalog_page':return{metas:[]};case'list_addons':return[];case'meta':return{meta:{id:args.id,type:'movie',name:'Título de teste',description:'Descrição'}};case'streams':if(args.id==='tt2')throw Error('offline');return{streams:[],subtitles:[],errors:[]};case'subtitle':return'WEBVTT';default:return{};}}}};`;
const routes = ['#/', '#/movies', '#/series', '#/animes', '#/search?q=filme', '#/library', '#/addons', '#/detail/movie/tt1'];
const failures = [];
const browser = await chromium.launch();
async function check(viewport, label) {
  const page = await browser.newPage({ viewport });
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
  await page.addInitScript(fixtures);
  for (const hash of routes) {
    await page.goto(`${base}/${hash}`);
    await page.waitForTimeout(300);
    const text = await page.locator('#view').innerText();
    if (/Erro:/.test(text)) failures.push(`[${label}] ${hash}: erro visível`);
    if (await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth)) failures.push(`[${label}] ${hash}: overflow horizontal`);
  }
  await page.goto(`${base}/#/movies`);
  await page.waitForTimeout(300);
  const cards = page.locator('#view .strip a');
  await cards.first().focus();
  await page.keyboard.press('ArrowRight');
  if (await page.evaluate(() => document.activeElement?.getAttribute('aria-label')) !== await cards.nth(1).getAttribute('aria-label')) failures.push(`[${label}] setas não navegam entre cards`);
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
try { await check({ width: 360, height: 800 }, 'celular'); await check({ width: 1280, height: 800 }, 'desktop'); }
finally { await browser.close(); server.close(); }
if (failures.length) { console.error('Falhas:\n- ' + failures.join('\n- ')); process.exit(1); }
console.log('Smoke test ok: rotas e navegação em celular e desktop.');
