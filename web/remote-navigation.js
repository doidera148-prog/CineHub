// Navegação por setas/OK/Voltar e atalhos de reprodução em teclado e controles remotos.
(() => {
  const SELECTOR = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
  const DIRS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', DpadUp: 'up', DpadDown: 'down', DpadLeft: 'left', DpadRight: 'right' };
  const GROUPS = '.strip, .episodes, .seasons, #streams, #nav-main, .dataactions, .kodi-results, #kodi-listing, .dialog-actions';
  let remoteMode = false;
  let focusBeforePlayer = null;

  function playerOpen() { return Boolean(document.querySelector('#player:not(.hidden)')); }
  function visibleFocusables(root = document) {
    return [...root.querySelectorAll(SELECTOR)].filter((el) => {
      if (el.closest('.hidden, [aria-hidden="true"]') || el.getAttribute('aria-hidden') === 'true' || el.tabIndex < 0 || el.disabled) return false;
      const rect = el.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0 && getComputedStyle(el).visibility !== 'hidden';
    });
  }
  function isTextEntry(el) {
    return el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && !['button', 'checkbox', 'radio', 'file', 'range'].includes(el.type));
  }
  function focus(el) {
    if (!el) return false;
    el.focus({ preventScroll: true });
    el.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' });
    el.closest('.strip')?.scrollTo({ left: Math.max(0, el.offsetLeft - 24), behavior: 'smooth' });
    el.closest('#nav-main')?.scrollTo({ left: Math.max(0, el.offsetLeft - 16), behavior: 'smooth' });
    return document.activeElement === el;
  }
  function nearest(from, candidates, direction) {
    const a = from.getBoundingClientRect(), ax = a.left + a.width / 2, ay = a.top + a.height / 2;
    const horizontal = direction === 'left' || direction === 'right';
    let best = null, bestScore = Infinity;
    for (const el of candidates) {
      if (el === from) continue;
      const b = el.getBoundingClientRect(), dx = b.left + b.width / 2 - ax, dy = b.top + b.height / 2 - ay;
      const ahead = direction === 'right' ? dx > 3 : direction === 'left' ? dx < -3 : direction === 'down' ? dy > 3 : dy < -3;
      if (!ahead) continue;
      const score = (horizontal ? Math.abs(dx) : Math.abs(dy)) + (horizontal ? Math.abs(dy) : Math.abs(dx)) * 2.4;
      if (score < bestScore) { best = el; bestScore = score; }
    }
    return best;
  }
  function wrapRow(from, candidates, direction) {
    const r = from.getBoundingClientRect(), cy = r.top + r.height / 2;
    const same = candidates.filter((el) => el !== from && Math.abs(el.getBoundingClientRect().top + el.getBoundingClientRect().height / 2 - cy) < Math.max(32, r.height));
    return same.reduce((best, el) => {
      if (!best) return el;
      return direction === 'right'
        ? (el.getBoundingClientRect().left < best.getBoundingClientRect().left ? el : best)
        : (el.getBoundingClientRect().left > best.getBoundingClientRect().left ? el : best);
    }, null);
  }
  function isOk(event) { return ['OK', 'Select', 'Accept', 'DPAD_CENTER', 'NumpadEnter'].includes(event.key) || event.keyCode === 23 || event.keyCode === 66; }
  function announce(message) {
    const toast = document.querySelector('#toast');
    if (!toast) return;
    toast.textContent = message;
    toast.classList.remove('hidden');
    clearTimeout(announce.timer);
    announce.timer = setTimeout(() => toast.classList.add('hidden'), 1300);
  }
  function seekVideo(seconds) {
    const video = document.querySelector('#video');
    if (!video || !Number.isFinite(video.duration)) return false;
    const target = Math.max(0, Math.min(video.duration, (video.currentTime || 0) + seconds));
    video.currentTime = target;
    announce(`${seconds > 0 ? 'Avançando' : 'Voltando'} ${Math.abs(seconds)} segundos`);
    return true;
  }

  // Detecta abertura/fechamento do player web para colocar foco dentro dele e devolvê-lo ao sair.
  const player = document.querySelector('#player');
  if (player) new MutationObserver(() => {
    if (!player.classList.contains('hidden')) {
      if (!player.contains(document.activeElement)) focusBeforePlayer = document.activeElement;
      if (remoteMode) requestAnimationFrame(() => focus(player.querySelector('#close-player') || player.querySelector('button')));
    } else if (focusBeforePlayer?.isConnected && remoteMode) {
      requestAnimationFrame(() => focus(focusBeforePlayer));
      focusBeforePlayer = null;
    }
  }).observe(player, { attributes: true, attributeFilter: ['class'] });

  document.addEventListener('keydown', (event) => {
    if (!playerOpen() || event.altKey || event.ctrlKey || event.metaKey || isTextEntry(document.activeElement)) return;
    const video = document.querySelector('#video');
    const focused = document.activeElement;
    const onVideo = focused === video || focused?.closest?.('.video-js');
    if (event.key === ' ' || event.key.toLowerCase() === 'k' || ['MediaPlayPause', 'Play', 'Pause'].includes(event.key)) {
      if (focused.matches?.('button, input, select')) return;
      event.preventDefault();
      if (video?.paused) video.play?.().catch?.(() => {}); else video?.pause?.();
      return;
    }
    if (!onVideo) return; // deixa os botões/controles Video.js usarem as setas naturalmente
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      event.preventDefault();
      seekVideo(event.key === 'ArrowRight' ? 10 : -10);
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault();
      if (video) video.volume = Math.max(0, Math.min(1, video.volume + (event.key === 'ArrowUp' ? 0.1 : -0.1)));
      announce(`Volume ${Math.round((video?.volume || 0) * 100)}%`);
    } else if (event.key.toLowerCase() === 'm') {
      event.preventDefault();
      if (video) video.muted = !video.muted;
      announce(video?.muted ? 'Sem áudio' : 'Áudio ativado');
    }
  }, true);

  // Campo de busca na TV: OK abre o teclado da TV (um clique no campo é o que faz o Android mostrá-lo);
  // Enter (ou Buscar no teclado) tira o foco do campo, e a próxima seta já cai nos resultados.
  document.addEventListener('keydown', (event) => {
    const target = document.activeElement;
    if (!target || target.id !== 'q' || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      target.blur();
      return;
    }
    if (isOk(event)) {
      event.preventDefault();
      target.click();
    }
  }, true);

  document.addEventListener('keydown', (event) => {
    if (!isOk(event) || event.altKey || event.ctrlKey || event.metaKey || document.querySelector('#confirm[open]')) return;
    const target = document.activeElement;
    if (!target || target === document.body || isTextEntry(target)) return;
    if (target.matches('a[href], button:not([disabled]), summary, [role="button"]')) {
      event.preventDefault(); event.stopImmediatePropagation(); target.click();
    }
  }, true);

  document.addEventListener('keydown', (event) => {
    const direction = DIRS[event.key];
    if (!direction || event.altKey || event.ctrlKey || event.metaKey || document.querySelector('#confirm[open]')) return;
    if (playerOpen()) return; // atalhos e controles do próprio player cuidam das setas.
    if (isTextEntry(document.activeElement) && (direction === 'left' || direction === 'right')) return;
    const items = visibleFocusables();
    if (!items.length) return;
    event.preventDefault(); event.stopImmediatePropagation(); remoteMode = true;
    const current = document.activeElement;
    if (!items.includes(current)) {
      focus(document.querySelector('#view a[href], #view button:not([disabled]), #view input:not([disabled]), #view select:not([disabled])') || items[0]);
      return;
    }
    const group = current.closest(GROUPS);
    if (group && (direction === 'left' || direction === 'right')) {
      const local = visibleFocusables(group);
      focus(nearest(current, local, direction) || wrapRow(current, local, direction));
      return;
    }
    focus((group && nearest(current, visibleFocusables(group), direction)) || nearest(current, items, direction));
  }, true);

  document.addEventListener('keydown', (event) => {
    if (!['Escape', 'Backspace', 'BrowserBack', 'GoBack'].includes(event.key)) return;
    if (event.key === 'Backspace' && isTextEntry(document.activeElement)) return;
    const dialog = document.querySelector('#confirm[open]');
    if (dialog) { event.preventDefault(); dialog.querySelector('[value="cancel"]')?.click(); return; }
    if (playerOpen()) { event.preventDefault(); document.querySelector('#close-player')?.click(); return; }
    if (location.hash && location.hash !== '#/' && location.hash !== '#') {
      event.preventDefault();
      if (history.length > 1) history.back(); else location.hash = '#/';
    }
  }, true);

  // Chamado pelo Android (plugin ExoPlayer, ao apertar Voltar) antes de sair do app.
  // Devolve true quando a página tratou o Voltar; false deixa o sistema fechar o app.
  let lastBackAt = 0;
  window.__cinehubBack = () => {
    const dialog = document.querySelector('dialog[open]');
    if (dialog) {
      const cancel = dialog.querySelector('[value="cancel"]');
      if (cancel) cancel.click();
      else dialog.close();
      return true;
    }
    if (playerOpen()) {
      document.querySelector('#close-player')?.click();
      return true;
    }
    if (location.hash && location.hash !== '#/' && location.hash !== '#') {
      history.back();
      return true;
    }
    const now = Date.now();
    if (now - lastBackAt < 2500) return false;
    lastBackAt = now;
    announce('Pressione Voltar de novo para sair');
    return true;
  };

  document.addEventListener('focusin', () => { remoteMode = true; });
  document.addEventListener('pointerdown', () => { remoteMode = false; }, true);
  window.addEventListener('hashchange', () => {
    if (!remoteMode) return;
    requestAnimationFrame(() => {
      if (document.activeElement && document.activeElement !== document.body && document.activeElement.isConnected) return;
      focus(document.querySelector('#view a[href], #view button:not([disabled]), #view input:not([disabled]), #view select:not([disabled])'));
    });
  });
})();
