// Comportamentos acessíveis que não devem interferir no roteamento por hash do app.
(() => {
  const skip = document.querySelector('.skip-link');
  skip?.addEventListener('click', () => {
    const target = document.querySelector('#view');
    if (!target) return;
    target.focus({ preventScroll: true });
    target.scrollIntoView({ block: 'start', behavior: 'auto' });
  });
})();
