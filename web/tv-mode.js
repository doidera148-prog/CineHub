// Ativa a interface de sala de estar somente em Android de tela grande (Android TV).
// Celulares Android e navegadores desktop mantêm a interface original.
(() => {
  const ua = navigator.userAgent || '';
  const android = /Android/i.test(ua);
  const largeScreen = Math.max(screen.width || 0, screen.height || 0) >= 960;
  const televisionHint = /TV|BRAVIA|AFT|SHIELD|MiBOX|GoogleTV|Android TV/i.test(ua);
  const noTouch = matchMedia('(pointer: none)').matches && matchMedia('(hover: none)').matches;
  if (android && largeScreen && (televisionHint || noTouch)) {
    document.documentElement.classList.add('tv-mode');
    document.body?.classList.add('tv-mode');
  }
  // body pode ainda não existir quando o script é executado no head.
  if (document.documentElement.classList.contains('tv-mode')) {
    document.addEventListener('DOMContentLoaded', () => document.body.classList.add('tv-mode'), { once: true });
  }
})();
