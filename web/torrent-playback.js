// A URL local do stream ainda herda infoHash/hash da fonte Torrentio.
// Não deve ser classificada como torrent novamente pelo play(), senão ele reabre
// a mesma fonte recursivamente em vez de iniciar o player sobre o HTTP local.
const isTorrentStreamBeforeLocalResolution = window.isTorrentStream;
window.isTorrentStream = function (stream) {
  const url = String(stream?.url ?? '').trim();
  try {
    const parsed = new URL(url);
    const loopback = ['127.0.0.1', 'localhost', '[::1]'].includes(parsed.hostname);
    if (loopback && parsed.pathname.startsWith('/stream/')) return false;
  } catch {
    // Deixa a função original validar URLs que não sejam absolutas.
  }
  return isTorrentStreamBeforeLocalResolution(stream);
};
