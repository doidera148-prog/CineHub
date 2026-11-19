// Ativa a interface de sala de estar somente em Android de tela grande (Google TV).
// Celulares Android e navegadores desktop mantêm a interface original.
(() => {
  const ua = navigator.userAgent || '';
  const android = /Android/i.test(ua);
  const screenPixels = Math.max(screen.width || 0, screen.height || 0);
  const largeScreen = screenPixels >= 960;
  // Algumas TVs reportam 1920x1080 em CSS pixels e outras 3840x2160.
  // Considera também o devicePixelRatio para reconhecer painéis 4K sem depender do UA.
  const is4k = screenPixels * Math.max(1, window.devicePixelRatio || 1) >= 3000;
  const televisionHint = /TV|BRAVIA|AFT|SHIELD|MiBOX|GoogleTV|Google TV/i.test(ua);
  const noTouch = matchMedia('(pointer: none)').matches && matchMedia('(hover: none)').matches;
  if (android && largeScreen && (televisionHint || noTouch)) {
    document.documentElement.classList.add('tv-mode');
    document.body?.classList.add('tv-mode');
    if (is4k) {
      document.documentElement.classList.add('tv-4k');
      document.body?.classList.add('tv-4k');
    }
  }
  // body pode ainda não existir quando o script é executado no head.
  if (document.documentElement.classList.contains('tv-mode')) {
    document.addEventListener('DOMContentLoaded', () => {
      document.body.classList.add('tv-mode');
      if (document.documentElement.classList.contains('tv-4k')) document.body.classList.add('tv-4k');
    }, { once: true });
  }
})();
