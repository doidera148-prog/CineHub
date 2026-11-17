// Testes das funções puras da interface (web/lib.js), sem navegador.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const src = readFileSync(join(here, '..', '..', 'web', 'lib.js'), 'utf8');
const ctx = {};
vm.createContext(ctx);
vm.runInContext(src, ctx);
const L = ctx.CineHubLib;

test('só http(s) é aceito como URL de imagem', () => {
  assert.equal(L.isWebUrl('https://a.example/p.jpg'), true);
  assert.equal(L.isWebUrl('javascript:alert(1)'), false);
  assert.equal(L.isWebUrl(undefined), false);
  assert.equal(L.cssUrl('file:///etc/passwd'), '');
});

test('cssUrl escapa aspas e parênteses', () => {
  assert.equal(L.cssUrl("https://a.example/x(1)'.jpg"), "url('https://a.example/x%281%29%27.jpg')");
});

test('progresso fica entre 0 e 100 e aceita duração desconhecida', () => {
  assert.equal(L.progressPercent({ t: 50, duration: 100 }), 50);
  assert.equal(L.progressPercent({ t: 300, duration: 100 }), 100);
  assert.equal(L.progressPercent({ t: 10, duration: 0 }), 0);
  assert.equal(L.progressPercent(null), 0);
});

test('cleanProgress preserva o marcador de assistido e descarta lixo', () => {
  const p = L.cleanProgress({ t: '12', duration: 'x', watched: true });
  assert.equal(p.t, 12);
  assert.equal(p.duration, 0);
  assert.equal(p.watched, true);
  assert.equal(L.cleanProgress({ watched: 'sim' }).watched, false);
});

test('fontes reproduzíveis vêm antes das que não tocam', () => {
  const out = L.sortStreams([
    { name: 'Sem link', title: '1080p' },
    { url: 'https://a.example/v.mp4', title: '720p' },
  ]);
  assert.equal(out[0].url, 'https://a.example/v.mp4');
});

test('entre fontes tocáveis, a de maior resolução vem primeiro', () => {
  const out = L.sortStreams([
    { url: 'https://a/1', title: '720p WEB' },
    { url: 'https://a/2', title: '1080p BluRay' },
    { url: 'https://a/3', title: '4K HDR' },
  ]);
  assert.deepEqual(out.map((s) => s.url), ['https://a/3', 'https://a/2', 'https://a/1']);
});

test('dublagem em português desempata a favor, e a ordem original é mantida', () => {
  const out = L.sortStreams([
    { url: 'https://a/eng', title: '1080p Legendado' },
    { url: 'https://a/pt', title: '1080p Dublado' },
    { url: 'https://a/eng2', title: '1080p Legendado' },
  ]);
  assert.deepEqual(out.map((s) => s.url), ['https://a/pt', 'https://a/eng', 'https://a/eng2']);
});

test('próximo episódio segue a ordem de temporada e episódio', () => {
  const videos = [
    { id: 'b:1:2', season: 1, episode: 2 },
    { id: 'b:1:1', season: 1, episode: 1 },
    { id: 'b:2:1', season: 2, episode: 1 },
  ];
  assert.equal(L.nextEpisode(videos, 'b:1:1').id, 'b:1:2');
  assert.equal(L.nextEpisode(videos, 'b:1:2').id, 'b:2:1');
  assert.equal(L.nextEpisode(videos, 'b:2:1'), null);
  assert.equal(L.nextEpisode(videos, 'inexistente'), null);
});

test('temporada e episódio vindos do addon viram número (regressão de XSS)', () => {
  const malicioso = '<img src=x onerror=alert(1)>';
  assert.equal(L.seasonOf({ season: malicioso }), 0);
  assert.equal(L.seasonOf({ season: '2' }), 2);
  assert.equal(L.seasonOf({}), 0);
  assert.equal(L.episodeOf({ episode: malicioso }), null);
  assert.equal(L.episodeOf({ episode: '7' }), 7);
  assert.equal(L.episodeOf({ episode: NaN }), null);
  assert.equal(L.episodeOf(undefined), null);
});
