// Navegação por setas/OK/Voltar e atalhos de reprodução em teclado e controles remotos.
(() => {
  const SELECTOR = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), summary, [role="button"], [tabindex]:not([tabindex="-1"])';
  const DIRS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', DpadUp: 'up', DpadDown: 'down', DpadLeft: 'left', DpadRight: 'right' };
  // Alguns controles TCL/Android TV entregam o DPAD apenas por keyCode.
  const KEYCODE_DIRS = { 19: 'up', 20: 'down', 21: 'left', 22: 'right' };
  const directionOf = (event) => DIRS[event.key] || KEYCODE_DIRS[event.keyCode] || null;
  const GROUPS = '.strip, .episodes, .seasons, #streams, #nav-main, .dataactions, .kodi-results, #kodi-listing, .dialog-actions, .tp-list';
  let remoteMode = false;
  let focusBeforePlayer = null;

  function playerOpen() { return Boolean(document.querySelector('#player:not(.hidden)')); }
  function visibleFocusables(root = document) {
    return [...root.querySelectorAll(SELECTOR)].filter((el) => {
      if (el.closest('.hidden, [aria-hidden="true"], [inert]') || el.getAttribute('aria-hidden') === 'true' ||
          el.getAttribute('aria-disabled') === 'true' || el.tabIndex < 0 || el.disabled) return false;
      const rect = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      const opacity = Number.parseFloat(style.opacity || '1');
      // Slides inativos e elementos transparentes não podem receber foco via DPAD.
      if (el.closest('.billboard-slide:not(.is-active), [aria-hidden="true"], [inert]')) return false;
      return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' &&
        style.display !== 'none' && opacity > 0.05;
    });
  }
  function isTextEntry(el) {
    return el instanceof HTMLTextAreaElement ||
      (el instanceof HTMLInputElement && !['button', 'checkbox', 'radio', 'file', 'range'].includes(el.type));
  }
  function focus(el) {
    if (!el) return false;
    try { el.focus({ preventScroll: true }); } catch { el.focus(); }
    // O navegador rola tanto a página quanto os trilhos horizontais sem saltos desnecessários.
    el.scrollIntoView?.({ block: 'nearest', inline: 'nearest', behavior: 'auto' });
    return document.activeElement === el;
  }
  function nearest(from, candidates, direction) {
    const a = from.getBoundingClientRect();
    const horizontal = direction === 'left' || direction === 'right';
    const aMainStart = horizontal ? a.left : a.top;
    const aMainEnd = horizontal ? a.right : a.bottom;
    const aCrossStart = horizontal ? a.top : a.left;
    const aCrossEnd = horizontal ? a.bottom : a.right;
    let best = null, bestScore = Infinity;
    for (const el of candidates) {
      if (el === from || !el.isConnected) continue;
      const b = el.getBoundingClientRect();
      const bMainStart = horizontal ? b.left : b.top;
      const bMainEnd = horizontal ? b.right : b.bottom;
      const bCrossStart = horizontal ? b.top : b.left;
      const bCrossEnd = horizontal ? b.bottom : b.right;
      // Considera a posição das bordas, não apenas o centro: evita saltos
      // diagonais quando há cards/botões de tamanhos diferentes.
      const ahead = direction === 'right' ? bMainStart >= aMainEnd - 2 :
        direction === 'left' ? bMainEnd <= aMainStart + 2 :
        direction === 'down' ? bMainStart >= aMainEnd - 2 :
        bMainEnd <= aMainStart + 2;
      if (!ahead) continue;
      const crossGap = Math.max(0, bCrossStart - aCrossEnd, aCrossStart - bCrossEnd);
      const mainGap = Math.max(0, bMainStart - aMainEnd, aMainStart - bMainEnd);
      const crossOverlap = crossGap === 0;
      // Primeiro prioriza o avanço na direção pressionada; depois a proximidade
      // lateral. A penalidade diagonal é moderada para não saltar para uma coluna
      // distante só porque ela compartilha alguns pixels do eixo perpendicular.
      const crossCenterDelta = Math.abs(((bCrossStart + bCrossEnd) - (aCrossStart + aCrossEnd)) / 2);
      const score = mainGap * 4 + crossGap * 2 + crossCenterDelta * 0.35 -
        (crossOverlap ? Math.min(aCrossEnd - aCrossStart, bCrossEnd - bCrossStart) * 0.08 : 0);
      if (score < bestScore) { best = el; bestScore = score; }
    }
    return best;
  }
  function isOk(event) {
    return ['OK', 'Select', 'Accept', 'DPAD_CENTER', 'NumpadEnter'].includes(event.key) || event.keyCode === 23 || event.keyCode === 66;
  }
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

  // O player é um diálogo modal: mantém o foco dentro dele e devolve ao acionador ao fechar.
  const player = document.querySelector('#player');
  if (player) {
    new MutationObserver(() => {
      if (!player.classList.contains('hidden')) {
        if (!player.contains(document.activeElement)) focusBeforePlayer = document.activeElement;
        requestAnimationFrame(() => focus(player.querySelector('#close-player') || player.querySelector('button')));
      } else if (focusBeforePlayer?.isConnected) {
        const previous = focusBeforePlayer;
        focusBeforePlayer = null;
        requestAnimationFrame(() => focus(previous));
      }
    }).observe(player, { attributes: true, attributeFilter: ['class'] });

    document.addEventListener('keydown', (event) => {
      if (event.key !== 'Tab' || !playerOpen()) return;
      const items = visibleFocusables(player);
      if (!items.length) {
        event.preventDefault();
        focus(player.querySelector('#close-player'));
        return;
      }
      const first = items[0], last = items[items.length - 1], active = document.activeElement;
      if (!player.contains(active) || (event.shiftKey && active === first) || (!event.shiftKey && active === last)) {
        event.preventDefault();
        focus(event.shiftKey && player.contains(active) ? last : first);
      }
    }, true);
  }

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
    if (!onVideo) return;
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

  // OK no campo de busca abre o teclado virtual; Enter confirma e libera as setas.
  document.addEventListener('keydown', (event) => {
    const target = document.activeElement;
    if (!target || target.id !== 'q' || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'Enter') { event.preventDefault(); target.blur(); return; }
    if (isOk(event)) { event.preventDefault(); target.click(); }
  }, true);

  document.addEventListener('keydown', (event) => {
    if (!isOk(event) || event.altKey || event.ctrlKey || event.metaKey || document.querySelector('#confirm[open]')) return;
    const target = document.activeElement;
    if (!target || target === document.body || isTextEntry(target)) return;
    if (target.matches('a[href], button:not([disabled]), summary, [role="button"]')) {
      event.preventDefault();
      event.stopImmediatePropagation();
      target.click();
    }
  }, true);

  document.addEventListener('keydown', (event) => {
    const direction = directionOf(event);
    if (!direction || event.altKey || event.ctrlKey || event.metaKey || document.querySelector('#confirm[open]')) return;
    if (playerOpen()) return;
    const current = document.activeElement;
    // Em diálogos (como a escolha de episódio do torrent), nunca navega para trás da janela.
    const activeDialog = document.querySelector('dialog[open]');
    if (current?.matches?.('select, input[type="range"]')) return;
    if (isTextEntry(current) && (direction === 'left' || direction === 'right')) return;
    const scope = activeDialog || document;
    const items = visibleFocusables(scope);
    if (!items.length) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    remoteMode = true;
    if (!items.includes(current)) {
      focus(items[0]);
      return;
    }
    const group = current.closest(GROUPS);
    // A lista de arquivos do torrent é uma lista vertical: navegação por índice evita
    // que o cálculo geométrico pule itens quando os nomes têm comprimentos diferentes.
    if (group?.matches('.tp-list') && (direction === 'up' || direction === 'down')) {
      const entries = visibleFocusables(group);
      const index = entries.indexOf(current);
      const nextIndex = index < 0 ? 0 : Math.max(0, Math.min(entries.length - 1, index + (direction === 'down' ? 1 : -1)));
      if (entries[nextIndex]) focus(entries[nextIndex]);
      return;
    }
    if (group && (direction === 'left' || direction === 'right')) {
      const localNext = nearest(current, visibleFocusables(group), direction);
      // Não faz loop infinito no trilho: na ponta, a seta sai do grupo e alcança o menu,
      // a busca ou o próximo bloco. Esse era o principal motivo de o controle ficar preso.
      const outside = localNext ? null : nearest(current, items.filter((el) => !group.contains(el)), direction);
      if (localNext || outside) focus(localNext || outside);
      return;
    }
    const next = nearest(current, items, direction);
    if (next) focus(next);
  }, true);

  document.addEventListener('keydown', (event) => {
    if (!['Escape', 'Backspace', 'BrowserBack', 'GoBack'].includes(event.key)) return;
    // Em Android TV, Voltar primeiro fecha o teclado virtual da busca, sem sair da tela.
    if (['Escape', 'BrowserBack', 'GoBack'].includes(event.key) && isTextEntry(document.activeElement)) {
      event.preventDefault();
      document.activeElement.blur();
      return;
    }
    if (event.key === 'Backspace' && isTextEntry(document.activeElement)) return;
    const dialog = document.querySelector('#confirm[open]');
    if (dialog) { event.preventDefault(); dialog.querySelector('[value="cancel"]')?.click(); return; }
    if (playerOpen()) { event.preventDefault(); document.querySelector('#close-player')?.click(); return; }
    if (location.hash && location.hash !== '#/' && location.hash !== '#') {
      event.preventDefault();
      if (history.length > 1) history.back(); else location.hash = '#/';
    }
  }, true);

  // Chamado pelo Android antes de sair do app.
  let lastBackAt = 0;
  window.__cinehubBack = () => {
    // O callback nativo intercepta Voltar antes do WebView; tirar o foco fecha o IME.
    if (isTextEntry(document.activeElement)) {
      document.activeElement.blur();
      return true;
    }
    const dialog = document.querySelector('dialog[open]');
    if (dialog) {
      const cancel = dialog.querySelector('[value="cancel"]');
      if (cancel) cancel.click(); else dialog.close();
      return true;
    }
    if (playerOpen()) { document.querySelector('#close-player')?.click(); return true; }
    if (location.hash && location.hash !== '#/' && location.hash !== '#') { history.back(); return true; }
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
