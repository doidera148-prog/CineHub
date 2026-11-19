// Testes de regressão da pesquisa da interface (Chromium + backend Tauri simulado).
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const WEB = resolve(fileURLToPath(new URL('../../web/', import.meta.url)));
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png', '.woff2': 'font/woff2' };
const server = createServer(async (req, res) => {
  let pathname;
  try { pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname); }
  catch { res.writeHead(400).end(); return; }
  const path = normalize(pathname);
  const file = resolve(join(WEB, path === '/' ? 'index.html' : path));
  if (file !== WEB && !file.startsWith(WEB + '/')) { res.writeHead(403).end(); return; }
  try {
    const body = await readFile(file);
    res.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream' });
    res.end(body);
  } catch { res.writeHead(404).end(); }
});
await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen));
const base = `http://127.0.0.1:${server.address().port}`;
const fixture = `
window.__searchCalls = [];
window.__TAURI__ = { core: { invoke: async (cmd, args) => {
  if (cmd === 'search') {
    window.__searchCalls.push({ ...args, at: Date.now() });
    const delay = args.q === 'lento' ? 700 : 25;
    await new Promise(resolve => setTimeout(resolve, delay));
    if (args.q === 'erro') throw new Error('falha simulada');
    const label = args.kind === 'anime' ? 'Anime' : 'Resultado';
    return { rows: args.q === 'vazio' ? [] : [{
      title: 'Pesquisa', addon: 'Addon de teste', metas: [{
        id: 'search-' + args.q, type: args.kind || 'movie',
        name: label + ' ' + args.q, poster: '', releaseInfo: '2026'
      }]
    }] };
  }
  if (cmd === 'home') return { rows: [] };
  if (cmd === 'list_addons') return [];
  return {};
} } };
`;
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
await page.addInitScript(fixture);
const calls = () => page.evaluate(() => window.__searchCalls);
const submit = async (inputSelector, buttonSelector) => {
  if (buttonSelector) await page.locator(buttonSelector).click();
  else await page.locator(inputSelector).press('Enter');
};
const waitForQuery = (q, kind) => page.waitForFunction(
  ({ q, kind }) => window.__searchCalls.some(call => call.q === q && (call.kind || null) === kind),
  { q, kind }, { timeout: 3000 }
);
try {
  // Pesquisa pela rota: valor inicial, remoção de espaços e resultado.
  await page.goto(`${base}/#/search?q=inicial`);
  await waitForQuery('inicial', null);
  assert.equal(await page.locator('#sq').getAttribute('maxlength'), '200');
  assert.equal(await page.locator('#sf').getAttribute('role'), 'search');
  assert.equal(await page.locator('#sq').getAttribute('aria-label'), 'Pesquisar título');
  assert.equal(await page.locator('#search-status').getAttribute('role'), 'status');
  assert.equal(await page.locator('#search-status').getAttribute('aria-live'), 'polite');
  await page.locator('#sq').fill('  matrix  ');
  await page.waitForTimeout(1500);
  assert.equal((await calls()).length, 1, 'digitar não deve iniciar a pesquisa automaticamente');
  await submit('#sq', '#sf button[type="submit"]');
  await waitForQuery('matrix', null);
  await page.waitForFunction(() => document.querySelector('#search-status')?.textContent === '1 resultado');
  assert.equal((await calls()).at(-1).q, 'matrix', 'a busca deve remover espaços nas extremidades');
  assert.match(await page.locator('#rows').innerText(), /Resultado matrix/);

  // Nenhuma pausa entre letras dispara busca; o clique no botão confirma a consulta.
  const beforePartialInput = (await calls()).length;
  await page.locator('#sq').fill('a');
  await page.waitForTimeout(1800);
  assert.equal((await calls()).length, beforePartialInput,
    'uma letra isolada não deve iniciar a pesquisa automaticamente');
  await page.locator('#sq').fill('avatar');
  await page.waitForTimeout(1200);
  assert.equal((await calls()).length, beforePartialInput,
    'a pesquisa deve aguardar confirmação explícita mesmo após uma pausa longa');
  await submit('#sq', '#sf button[type="submit"]');
  await waitForQuery('avatar', null);

  // Enter também confirma a pesquisa e uma única ação gera uma consulta.
  const beforeEnter = (await calls()).length;
  await page.locator('#sq').fill('segundo');
  await page.waitForTimeout(1200);
  assert.equal((await calls()).length, beforeEnter);
  await submit('#sq');
  await waitForQuery('segundo', null);
  assert.equal((await calls()).filter(call => call.q === 'segundo').length, 1,
    'Enter deve disparar somente uma consulta');

  // Apagar o campo limpa resultados/estado imediatamente e invalida resposta pendente.
  await page.locator('#sq').fill('lento');
  await submit('#sq');
  await waitForQuery('lento', null);
  await page.locator('#sq').fill('');
  await submit('#sq');
  await page.waitForFunction(() => !document.querySelector('#rows')?.hasAttribute('aria-busy'));
  assert.match(await page.locator('#rows').innerText(), /Digite um título/);
  await page.waitForTimeout(800);
  assert.doesNotMatch(await page.locator('#rows').innerText(), /Resultado lento/,
    'resposta antiga não pode reaparecer após limpar');

  // Uma resposta lenta não pode sobrescrever uma busca mais recente.
  await page.locator('#sq').fill('lento');
  await waitForQuery('lento', null);
  await page.locator('#sq').fill('rapido');
  await submit('#sq');
  await waitForQuery('rapido', null);
  await page.waitForFunction(() => document.querySelector('#rows')?.textContent.includes('Resultado rapido'));
  await page.waitForTimeout(800);
  assert.match(await page.locator('#rows').innerText(), /Resultado rapido/);
  assert.doesNotMatch(await page.locator('#rows').innerText(), /Resultado lento/);

  // Estados vazio e erro não deixam o indicador de carregamento preso.
  await page.locator('#sq').fill('vazio');
  await submit('#sq');
  await waitForQuery('vazio', null);
  await page.waitForFunction(() => document.querySelector('#search-status')?.textContent === 'Nenhum resultado');
  assert.doesNotMatch(await page.locator('#rows').getAttribute('aria-busy') || '', /true/);
  await page.locator('#sq').fill('erro');
  await submit('#sq');
  await waitForQuery('erro', null);
  await page.waitForFunction(() => document.querySelector('#search-status')?.textContent === 'Erro na pesquisa');
  assert.match(await page.locator('#rows').innerText(), /falha simulada/);

  // Escape limpa a consulta atual.
  await page.locator('#sq').fill('limpar');
  await submit('#sq');
  await waitForQuery('limpar', null);
  await page.locator('#sq').press('Escape');
  await page.waitForFunction(() => document.querySelector('#sq')?.value === '' &&
    document.querySelector('#search-status')?.textContent === '');
  assert.match(await page.locator('#rows').innerText(), /Digite um título/);

  // Composição IME: não pesquisar no meio da composição; pesquisar ao final.
  const beforeIme = (await calls()).length;
  await page.locator('#sq').evaluate(input => {
    input.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    input.value = 'nihongo';
    input.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertCompositionText', isComposing: true }));
  });
  await page.waitForTimeout(550);
  assert.equal((await calls()).length, beforeIme, 'IME não deve pesquisar durante composição');
  await page.locator('#sq').evaluate(input => input.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: 'nihongo' })));
  await page.waitForTimeout(1200);
  assert.equal((await calls()).length, beforeIme, 'fim da composição não deve pesquisar automaticamente');
  await submit('#sq');
  await waitForQuery('nihongo', null);

  // Barra superior navega para a pesquisa e mantém a mesma normalização.
  await page.goto(`${base}/#/movies`);
  await page.locator('#q').fill('  topo  ');
  await page.waitForTimeout(1200);
  assert.equal((await calls()).length, 0, 'campo superior não deve pesquisar durante digitação');
  await submit('#q', '#top-search-submit');
  await waitForQuery('topo', null);
  await page.waitForFunction(() => location.hash.startsWith('#/search?q=topo'));
  await page.waitForFunction(() => document.querySelector('#search-status')?.textContent === '1 resultado');
  assert.match(await page.locator('#rows').innerText(), /Resultado topo/);

  // Campo específico de animes encaminha kind=anime e apresenta resultados.
  await page.goto(`${base}/#/animes`);
  await page.locator('#anime-search-input').fill('naruto');
  await page.waitForTimeout(1200);
  assert.equal((await calls()).length, 0, 'campo de animes não deve pesquisar durante digitação');
  await submit('#anime-search-input', '#anime-search-form button[type="submit"]');
  await waitForQuery('naruto', 'anime');
  await page.waitForFunction(() => document.querySelector('#anime-search-status')?.textContent === '1 resultado');
  assert.match(await page.locator('#anime-search-results').innerText(), /Anime naruto/);

  // Acentos, símbolos, aspas e caracteres CJK devem atravessar a pesquisa sem corrupção.
  await page.goto(`${base}/#/search?q=inicial`);
  const unicodeQuery = 'ação "東京" 🎬';
  await page.locator('#sq').fill(unicodeQuery);
  await submit('#sq');
  await waitForQuery(unicodeQuery, null);
  assert.equal((await calls()).at(-1).q, unicodeQuery, 'caracteres Unicode devem ser preservados');

  // O limite de 200 caracteres é aplicado no próprio campo e na consulta enviada.
  const oversized = 'x'.repeat(260);
  await page.locator('#sq').fill(oversized);
  assert.equal(await page.locator('#sq').inputValue().then(value => value.length), 200);
  await submit('#sq');
  await waitForQuery('x'.repeat(200), null);
  assert.equal((await calls()).at(-1).q.length, 200);

  assert.deepEqual(errors, [], 'a pesquisa não deve produzir erros JavaScript/console');
  console.log('Pesquisa UI: confirmação explícita, normalização, limpar, concorrência, vazio/erro, Escape, IME, barra superior e animes — OK');
} finally {
  await page.close();
  await browser.close();
  server.close();
}
