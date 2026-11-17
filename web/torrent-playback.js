// A URL local do stream ainda herda infoHash/hash da fonte original. Nunca a reabra como magnet.
const previousTorrentCheck = window.isTorrentStream;
window.isTorrentStream = function isTorrentStream(stream) {
  const url = String(stream?.url ?? '').trim();
  try {
    const parsed = new URL(url);
    if (['127.0.0.1', 'localhost', '[::1]'].includes(parsed.hostname) && parsed.pathname.startsWith('/stream/')) return false;
  } catch { /* delega a validação do restante à função anterior */ }
  return previousTorrentCheck(stream);
};
