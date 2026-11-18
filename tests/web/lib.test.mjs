// Testes das funções puras da interface (web/lib.js), sem navegador.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const src = readFileSync(join(here, '..', '..', 'web', 'lib.js'), 'utf8');
const ctx = { URL };
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

test('fontes de torrent vêm antes das de link direto, mesmo com resolução menor', () => {
  const out = L.sortStreams([
    { url: 'https://a/direto', title: '4K HDR' },
    { infoHash: 'a'.repeat(40), title: '720p WEB' },
    { infoHash: 'b'.repeat(40), title: '1080p BluRay' },
  ]);
  assert.deepEqual(out.map((s) => s.infoHash || s.url), ['b'.repeat(40), 'a'.repeat(40), 'https://a/direto']);
});

test('dublagem em português vem antes de torrent e de resolução maior', () => {
  const out = L.sortStreams([
    { infoHash: 'a'.repeat(40), title: '4K Legendado' },
    { url: 'https://a/pt-direto', title: '720p Dublado' },
    { infoHash: 'b'.repeat(40), title: '1080p Dublado' },
  ]);
  assert.deepEqual(out.map((s) => s.infoHash || s.url), ['b'.repeat(40), 'https://a/pt-direto', 'a'.repeat(40)]);
});

test('idioma do arquivo de torrent sai do nome', () => {
  assert.equal(L.torrentLanguage('Show.S01E02.1080p.DUBLADO.mkv'), 'pt');
  assert.equal(L.torrentLanguage('Movie.2020.PT-BR.mkv'), 'pt');
  assert.equal(L.torrentLanguage('Show.S01E02.English.mkv'), 'en');
  assert.equal(L.torrentLanguage('Show.S01E02.mkv'), 'other');
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

test('animes têm episódios como as séries; filmes não', () => {
  assert.equal(L.isSeriesLike('series'), true);
  assert.equal(L.isSeriesLike('anime'), true);
  assert.equal(L.isSeriesLike('movie'), false);
  assert.equal(L.isSeriesLike(undefined), false);
});

test('rótulo do tipo: filme, série e anime', () => {
  assert.equal(L.typeLabel('movie'), 'Filme');
  assert.equal(L.typeLabel('series'), 'Série');
  assert.equal(L.typeLabel('anime'), 'Anime');
  assert.equal(L.typeLabel('tv'), '');
});

test('filtro de fontes por qualidade, idioma e addon', () => {
  const list = [
    { name: 'Torrentio', title: 'Filme 1080p dublado', addonName: 'Torrentio' },
    { name: 'Torrentio', title: 'Filme 720p english', addonName: 'Torrentio' },
    { name: 'Outro', title: 'Filme 4K', addonName: 'Outro' },
  ];
  assert.equal(L.qualityLabel('x 1080p'), '1080p');
  assert.equal(L.qualityLabel('x 4K HDR'), '4K');
  assert.equal(L.qualityLabel('x'), 'Outras');
  assert.equal(L.filterStreams(list, { quality: ['1080p'], lang: [], addon: [] }).length, 1);
  assert.equal(L.filterStreams(list, { quality: [], lang: ['en'], addon: [] })[0].title, 'Filme 720p english');
  assert.equal(L.filterStreams(list, { quality: [], lang: [], addon: ['Outro'] }).length, 1);
  assert.equal(L.filterStreams(list, { quality: [], lang: [], addon: [] }).length, 3);
});

test('detecta contêineres e rejeita fontes MKV', () => {
  assert.equal(L.streamContainer({ url: 'https://cdn.example/movie.mp4?token=x' }), 'mp4');
  assert.equal(L.streamContainer({ url: 'https://cdn.example/movie.webm' }), 'webm');
  assert.equal(L.streamContainer({ mimeType: 'video/x-matroska' }), 'mkv');
  assert.equal(L.streamContainer({ name: 'Filme.1080p.mkv' }), 'mkv');
  assert.equal(L.streamContainer({ url: 'https://cdn.example/play?id=123' }), 'unknown');
  assert.equal(L.isMkvStream({ title: 'Filme.mkv' }), true);
  assert.equal(L.isMkvStream({ mimeType: 'video/mp4' }), false);
});

test('fontes MP4/WebM são priorizadas na ordenação', () => {
  const list = [
    { url: 'https://cdn.example/movie.mkv', title: 'Filme 4K' },
    { url: 'https://cdn.example/movie.mp4', title: 'Filme 720p' },
    { url: 'https://cdn.example/movie.webm', title: 'Filme 1080p' },
  ];
  assert.deepEqual(L.sortStreams(list).map((s) => s.url), [
    'https://cdn.example/movie.webm',
    'https://cdn.example/movie.mp4',
    'https://cdn.example/movie.mkv',
  ]);
});

test('ordenação das fontes: maior qualidade e português primeiro', () => {
  const list = [
    { title: 'a 720p' },
    { title: 'b 4K' },
    { title: 'c 1080p dublado' },
  ];
  assert.equal(L.sortStreamsBy(list, 'quality').map((s) => s.title).join('|'), 'b 4K|c 1080p dublado|a 720p');
  assert.equal(L.sortStreamsBy(list, 'pt').map((s) => s.title).join('|'), 'c 1080p dublado|a 720p|b 4K');
  assert.equal(L.sortStreamsBy(list, 'default'), list);
});

test('o trecho ativo vale até meio segundo antes do fim', () => {
  const segs = [{ type: 'intro', start: 5, end: 90 }, { type: 'credits', start: 1300, end: 1400 }];
  assert.equal(L.activeSegment(segs, 4.9), null);
  assert.equal(L.activeSegment(segs, 5).type, 'intro');
  assert.equal(L.activeSegment(segs, 89.7), null);
  assert.equal(L.activeSegment(segs, 1350).type, 'credits');
  assert.equal(L.activeSegment(undefined, 10), null);
  assert.equal(L.activeSegment(segs, NaN), null);
});

test('teclas 0–9 pulam para 0–90% do vídeo', () => {
  assert.equal(L.seekFraction('0', 1000), 0);
  assert.equal(L.seekFraction('5', 1000), 500);
  assert.equal(L.seekFraction('9', 1000), 900);
  assert.equal(L.seekFraction('a', 1000), null);
  assert.equal(L.seekFraction('5', NaN), null);
  assert.equal(L.seekFraction('5', 0), null);
});

test('streams torrent locais preparados nunca passam pelo proxy HTTP', () => {
  const localStreams = [
    { url: 'http://127.0.0.1:43127/stream/token' },
    { url: 'http://localhost:43127/stream/token?file=2' },
    { url: 'http://[::1]:43127/stream/token' },
  ];
  for (const stream of localStreams) {
    assert.equal(L.isLocalTorrentStream(stream), true);
    assert.equal(L.shouldUseMediaProxy(stream), false);
  }
});

test('somente endpoint /stream em loopback é tratado como torrent preparado', () => {
  const external = [
    'https://cdn.example/stream/token',
    'http://192.168.1.20:43127/stream/token',
    'http://127.0.0.1:43127/media/token',
    'not a url',
  ];
  for (const url of external) {
    const stream = { url };
    assert.equal(L.isLocalTorrentStream(stream), false);
    assert.equal(L.shouldUseMediaProxy(stream), true);
  }
});

test('URL local preparada não é reaberta como magnet mesmo herdando infoHash', () => {
  assert.equal(L.isTorrentSource({
    url: 'http://127.0.0.1:43127/stream/token',
    infoHash: 'a'.repeat(40),
  }), false);
  assert.equal(L.isTorrentSource({ url: 'magnet:?xt=urn:btih:abc' }), true);
  assert.equal(L.isTorrentSource({ infoHash: 'a'.repeat(40) }), true);
  assert.equal(L.isTorrentSource({ hash: 'b'.repeat(40) }), true);
  assert.equal(L.isTorrentSource({ url: 'https://cdn.example/video.mp4' }), false);
});


test('backup mescla favoritos e mantém o progresso mais recente sem aceitar chaves perigosas', () => {
  const result = L.mergeBackupData(
    [{ id: 'movie:1', name: 'Atual' }],
    { 'movie:1': { t: 10, duration: 100, updated: 20 }, 'series:1': { t: 5, duration: 50, updated: 10 } },
    {
      app: 'CineHub', version: 1,
      favs: [{ id: 'movie:1', name: 'Duplicado' }, { id: 'movie:2', name: 'Novo' }, { id: '__proto__' }],
      progress: JSON.parse('{"movie:1":{"t":80,"duration":100,"updated":15},"series:1":{"t":30,"duration":50,"updated":30},"__proto__":{"t":999,"duration":1000,"updated":999},"constructor":{"t":999,"duration":1000,"updated":999}}'),
    },
  );
  assert.deepEqual(result.favs.map((f) => f.id), ['movie:1', 'movie:2', '__proto__']);
  assert.equal(result.progress['movie:1'].t, 10);
  assert.equal(result.progress['series:1'].t, 30);
  assert.equal(Object.prototype.hasOwnProperty.call(result.progress, '__proto__'), false);
});

test('backup inválido ou de versão desconhecida não é mesclado', () => {
  assert.throws(() => L.mergeBackupData([], {}, { app: 'Outro', version: 1 }), /backup incompatível/);
  assert.throws(() => L.mergeBackupData([], {}, { app: 'CineHub', version: 2 }), /backup incompatível/);
});

test('registros importados de progresso não mantêm tempos negativos', () => {
  const result = L.mergeBackupData([], {}, {
    app: 'CineHub', version: 1,
    progress: { 'movie:1': { t: -5, duration: -10, updated: -1 } },
  });
  assert.equal(result.progress['movie:1'].t, 0);
  assert.equal(result.progress['movie:1'].duration, 0);
  assert.equal(result.progress['movie:1'].updated, 0);
});


test('gravação em lote salva todas as chaves quando não há falha', () => {
  const values = new Map([['favs', 'old-favs'], ['progress', 'old-progress']]);
  const storage = {
    getItem: (key) => values.has(key) ? values.get(key) : null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  assert.equal(L.writeStorageBatch(storage, { favs: ['new'], progress: { a: 1 } }), true);
  assert.deepEqual(JSON.parse(values.get('favs')), ['new']);
  assert.deepEqual(JSON.parse(values.get('progress')), { a: 1 });
});

test('gravação em lote restaura todas as chaves se uma delas falhar', () => {
  const values = new Map([['favs', 'old-favs'], ['progress', 'old-progress']]);
  const storage = {
    getItem: (key) => values.has(key) ? values.get(key) : null,
    setItem: (key, value) => {
      if (key === 'progress' && value !== 'old-progress') throw new Error('quota excedida');
      values.set(key, value);
    },
    removeItem: (key) => values.delete(key),
  };
  assert.equal(L.writeStorageBatch(storage, { favs: ['new'], progress: { a: 1 } }), false);
  assert.equal(values.get('favs'), 'old-favs');
  assert.equal(values.get('progress'), 'old-progress');
});

test('gravação em lote rejeita valores não serializáveis sem alterar o armazenamento', () => {
  const values = new Map([['favs', 'old-favs']]);
  const storage = {
    getItem: (key) => values.has(key) ? values.get(key) : null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  const circular = {};
  circular.self = circular;
  assert.equal(L.writeStorageBatch(storage, { favs: ['new'], progress: circular }), false);
  assert.equal(values.get('favs'), 'old-favs');
});


test('backup exportado é canônico, sanitizado e pode ser importado novamente', () => {
  const payload = L.createBackupData(
    [{ id: 'movie:1', name: 'Filme', poster: 'https://img.example/poster.jpg', extra: 'não exportar' }],
    { 'movie:1': { t: 42, duration: 100, updated: 10, watched: false, extra: 'não exportar' } },
    '2026-11-18T12:00:00.000Z',
  );
  assert.deepEqual(Object.keys(payload), ['app', 'version', 'exportedAt', 'favs', 'progress']);
  assert.equal(payload.app, 'CineHub');
  assert.equal(payload.version, 1);
  assert.equal(payload.favs[0].extra, undefined);
  assert.equal(payload.progress['movie:1'].extra, undefined);
  const restored = L.mergeBackupData([], {}, JSON.parse(JSON.stringify(payload)));
  assert.equal(JSON.stringify(restored.favs), JSON.stringify(payload.favs));
  assert.equal(JSON.stringify(restored.progress), JSON.stringify(payload.progress));
});

test('backup exportado respeita os limites de favoritos e progresso', () => {
  const favs = Array.from({ length: 5 }, (_, i) => ({ id: 'movie:' + i, name: 'Filme ' + i }));
  const progress = Object.fromEntries(Array.from({ length: 6 }, (_, i) => ['movie:' + i, { t: i, duration: 100, updated: i }]));
  const payload = L.createBackupData(favs, progress, 'data', 3, 2);
  assert.equal(payload.favs.length, 3);
  assert.deepEqual(Object.keys(payload.progress), ['movie:5', 'movie:4']);
});
