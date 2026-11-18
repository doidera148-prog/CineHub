// Testes de regressão da navegação D-pad usando o script real da aplicação.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const WEB = resolve(fileURLToPath(new URL('../../web/', import.meta.url)));
const server = createServer(async (req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname));
  const file = join(WEB, path === '/' ? 'index.html' : path);
  if (!file.startsWith(WEB)) return res.writeHead(403).end();
  try {
    const body = await readFile(file);
    res.writeHead(200, { 'Content-Type': extname(file) === '.js' ? 'text/javascript' : 'text/html' });
    res.end(body);
  } catch { res.writeHead(404).end(); }
});
await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen));
const base = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch();
const failures = [];
const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));

await page.setContent(`<!doctype html><html><head><style>
  * { box-sizing: border-box; }
  body { margin: 0; font: 16px sans-serif; }
  .sidebar { position: absolute; left: 0; top: 0; width: 180px; height: 600px; }
  #nav-main { display: flex; flex-direction: column; gap: 12px; }
  #main-content { position: absolute; left: 200px; top: 0; width: 780px; height: 650px; }
  .topbar { position: absolute; top: 0; right: 0; }
  #view { padding-top: 20px; }
  .strip { display: flex; gap: 12px; width: 600px; height: 90px; margin-bottom: 80px; }
  .strip a, #nav-main a, button, input { display: inline-flex; align-items: center; justify-content: center; width: 90px; height: 60px; }
  dialog { margin: 100px auto; }
</style></head><body>
  <aside class="sidebar"><nav id="nav-main">
    <a id="nav-home" href="#home" class="active" aria-current="page">Início</a>
    <a id="nav-movies" href="#movies">Filmes</a>
  </nav></aside>
  <main id="main-content">
    <header class="topbar"><input id="search" value="ABCD" aria-label="Busca"></header>
    <section id="view">
      <div class="strip" id="row-one">
        <a id="card-1" href="#one">Cartaz 1</a><a id="card-2" href="#two">Cartaz 2</a><a id="card-3" href="#three">Cartaz 3</a>
      </div>
      <div class="strip" id="row-two">
        <a id="card-4" href="#four">Cartaz 4</a><a id="card-5" href="#five">Cartaz 5</a><a id="card-6" href="#six">Cartaz 6</a>
      </div>
      <button id="action">Ação</button>
    </section>
  </main>
  <dialog id="test-dialog"><button id="dialog-first">Primeiro</button><button id="dialog-last">Último</button></dialog>
  <div id="toast" class="hidden"></div>
</body></html>`);
await page.addScriptTag({ url: `${base}/remote-navigation.js` });

async function press(key) { await page.keyboard.press(key); }
async function focused() { return page.evaluate(() => document.activeElement?.id); }
async function expectFocus(id, label) {
  const actual = await focused();
  if (actual !== id) failures.push(`[remote-navigation] ${label}: esperado ${id}, recebido ${actual}`);
}

// Horizontal: avança e recua dentro do mesmo trilho.
await page.locator('#card-1').focus();
await press('ArrowRight');
await expectFocus('card-2', 'seta direita no trilho');
await press('ArrowRight');
await expectFocus('card-3', 'segunda seta direita no trilho');
await press('ArrowLeft');
await expectFocus('card-2', 'seta esquerda no trilho');

// O menu só é alcançado pela esquerda na primeira coluna.
await page.locator('#card-2').focus();
await press('ArrowLeft');
await expectFocus('card-1', 'coluna interna não deve saltar para o menu');
await page.locator('#card-1').focus();
await press('ArrowLeft');
await expectFocus('nav-home', 'primeira coluna permite entrar no menu');

// Cima/baixo navegam entre linhas sem desviar para o menu lateral.
await page.locator('#card-1').focus();
await press('ArrowDown');
await expectFocus('card-4', 'seta para baixo entre trilhos');
await press('ArrowUp');
await expectFocus('card-1', 'seta para cima entre trilhos');
await press('ArrowUp');
await expectFocus('card-1', 'limite superior não deve mover foco para outro eixo');

await page.locator('#card-3').focus();
await press('ArrowUp');
await expectFocus('card-3', 'limite superior não pode saltar diagonalmente para a busca');
await page.locator('#card-6').focus();
await press('ArrowDown');
await expectFocus('card-6', 'limite inferior não pode saltar diagonalmente para ação');

// Menu: cima/baixo percorrem itens e direita retorna ao conteúdo.
await page.locator('#nav-home').focus();
await press('ArrowDown');
await expectFocus('nav-movies', 'navegação vertical no menu');
await press('ArrowUp');
await expectFocus('nav-home', 'retorno vertical no menu');
await press('ArrowRight');
if (!(await page.evaluate(() => document.querySelector('#main-content').contains(document.activeElement)))) {
  failures.push('[remote-navigation] seta direita no menu não retornou ao conteúdo');
}

// Compatibilidade com códigos de tecla DPAD usados por alguns controles Android.
await page.locator('#card-1').focus();
await page.evaluate(() => document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {
  key: 'Unidentified', keyCode: 22, which: 22, bubbles: true, cancelable: true
})));
await expectFocus('card-2', 'keyCode DPAD_RIGHT (22)');

// Campos de texto mantêm o cursor nas setas horizontais.
await page.locator('#search').focus();
await page.locator('#search').evaluate((el) => el.setSelectionRange(2, 2));
await press('ArrowLeft');
const cursor = await page.locator('#search').evaluate((el) => el.selectionStart);
if (cursor !== 1) failures.push(`[remote-navigation] seta esquerda moveu o foco em vez do cursor (posição ${cursor})`);

// Enter ativa o elemento focado exatamente uma vez.
await page.locator('#action').focus();
await page.evaluate(() => {
  window.__actionCount = 0;
  document.querySelector('#action').addEventListener('click', () => window.__actionCount++);
});
await press('Enter');
if (await page.evaluate(() => window.__actionCount) !== 1) failures.push('[remote-navigation] Enter não ativou o botão exatamente uma vez');

// Diálogos isolam o D-pad e não deixam o foco escapar para a tela de fundo.
await page.locator('#test-dialog').evaluate((el) => el.showModal());
await page.locator('#dialog-last').focus();
await press('ArrowRight');
await expectFocus('dialog-last', 'seta não pode escapar do limite do diálogo');
await page.locator('#dialog-last').press('Tab');
await expectFocus('dialog-first', 'Tab deve circular dentro do diálogo');
await page.locator('#test-dialog').evaluate((el) => el.close());

if (errors.length) failures.push(...errors.map((error) => `[remote-navigation] erro JS: ${error}`));
await browser.close();
server.close();
if (failures.length) {
  console.error('Falhas nos testes de controle remoto:\\n- ' + failures.join('\\n- '));
  process.exit(1);
}
console.log('Remote navigation tests passed: D-pad, trilhos, colunas, menu, campos, OK e diálogos.');
