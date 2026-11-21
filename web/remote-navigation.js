// Navegação D-pad unificada para Google TV, controles remotos e teclado.
(() => {
  'use strict';

  const FOCUSABLE = [
    'a[href]', 'button:not(:disabled)', 'input:not(:disabled)', 'select:not(:disabled)',
    'textarea:not(:disabled)', 'summary', '[role="button"]', '[tabindex]:not([tabindex="-1"])',
    '[contenteditable="true"]'
  ].join(',');
  const DIRECTIONS = {
    ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right',
    DpadUp: 'up', DpadDown: 'down', DpadLeft: 'left', DpadRight: 'right'
  };
  const KEYCODE_DIRECTIONS = { 19: 'up', 20: 'down', 21: 'left', 22: 'right' };
  const GROUPS = '.strip, .episodes, .seasons, #streams, #nav-main, .dataactions, .kodi-results, #kodi-listing, .dialog-actions, .tp-list';
  const TEXT_INPUT_TYPES = new Set(['button', 'checkbox', 'radio', 'file', 'range', 'submit', 'reset', 'color']);
  let remoteMode = false;
  let lastMainFocus = null;
  let focusBeforeModal = null;
  let pendingRouteFocus = false;
  let routeFocusOrigin = null;
  let lastBackAt = 0;

  const directionOf = (event) => DIRECTIONS[event.key] || KEYCODE_DIRECTIONS[event.keyCode] || null;
  const isOk = (event) =>
    ['Enter', ' ', 'OK', 'Select', 'Accept', 'DPAD_CENTER', 'NumpadEnter'].includes(event.key) ||
    event.keyCode === 23 || event.keyCode === 66;
  const isTextEntry = (el) =>
    el instanceof HTMLTextAreaElement ||
    (el instanceof HTMLInputElement && !TEXT_INPUT_TYPES.has(el.type)) ||
    Boolean(el?.isContentEditable);

  function isVisible(el) {
    if (!el?.isConnected || el.disabled || el.getAttribute('aria-disabled') === 'true' ||
        el.getAttribute('aria-hidden') === 'true' || el.tabIndex < 0 ||
        el.closest('.hidden, [hidden], [inert], [aria-hidden="true"], dialog:not([open])')) return false;
    const rect = el.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) return false;
    for (let node = el; node && node.nodeType === 1; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.display === 'none' || style.visibility === 'hidden' ||
          Number.parseFloat(style.opacity || '1') <= 0.05) return false;
    }
    return true;
  }

  function focusables(root = document) {
    if (!root) return [];
    return [...root.querySelectorAll(FOCUSABLE)].filter(isVisible);
  }

  function focusElement(el) {
    if (!isVisible(el)) return false;
    try { el.focus({ preventScroll: true }); } catch { el.focus(); }
    if (document.activeElement !== el) return false;
    el.scrollIntoView?.({ block: 'nearest', inline: 'nearest', behavior: 'auto' });
    if (el.closest('.sidebar')) {
      // O menu pode ter seu próprio scroll em layouts compactos.
      el.scrollIntoView?.({ block: 'nearest', inline: 'nearest', behavior: 'auto' });
    } else if (!el.closest('dialog[open]') && !el.closest('#player')) {
      lastMainFocus = el;
    }
    return true;
  }

  function rectOf(el) {
    const r = el.getBoundingClientRect();
    return { left: r.left, right: r.right, top: r.top, bottom: r.bottom,
      x: r.left + r.width / 2, y: r.top + r.height / 2 };
  }

  function overlaps(aStart, aEnd, bStart, bEnd) {
    return Math.min(aEnd, bEnd) - Math.max(aStart, bStart) > 0;
  }

  // Escolhe primeiro elementos alinhados no eixo perpendicular; só depois aceita diagonais.
  function nearest(from, candidates, direction, { alignedOnly = false } = {}) {
    if (!from) return null;
    const a = rectOf(from);
    const horizontal = direction === 'left' || direction === 'right';
    let best = null;
    let bestScore = Infinity;
    for (const el of candidates) {
      if (el === from || !isVisible(el)) continue;
      const b = rectOf(el);
      const delta = horizontal ? b.x - a.x : b.y - a.y;
      if ((direction === 'right' || direction === 'down') ? delta <= 3 : delta >= -3) continue;
      const overlap = horizontal
        ? overlaps(a.top, a.bottom, b.top, b.bottom)
        : overlaps(a.left, a.right, b.left, b.right);
      if (alignedOnly && !overlap) continue;
      const primary = Math.abs(delta);
      const cross = horizontal ? Math.abs(b.y - a.y) : Math.abs(b.x - a.x);
      const score = primary + cross * (overlap ? 1.8 : 3.2) + (overlap ? 0 : 90);
      if (score < bestScore) { best = el; bestScore = score; }
    }
    return best;
  }

  function activeModal() {
    const dialog = document.querySelector('dialog[open]');
    if (dialog) return dialog;
    const player = document.querySelector('#player:not(.hidden)');
    return player || null;
  }

  function activeRegion(current) {
    const modal = activeModal();
    if (modal) return modal;
    if (current?.closest('.sidebar')) return document.querySelector('.sidebar');
    return document.querySelector('#main-content') || document.querySelector('#view') || document;
  }

  function announce(message) {
    const toast = document.querySelector('#toast');
    if (!toast) return;
    toast.textContent = message;
    toast.classList.remove('hidden');
    clearTimeout(announce.timer);
    announce.timer = setTimeout(() => toast.classList.add('hidden'), 1400);
  }

  function handleDirection(event, direction) {
    const current = document.activeElement;
    if (isTextEntry(current)) {
      // Não rouba as setas horizontais do cursor nem as setas do seletor nativo.
      if (direction === 'left' || direction === 'right' ||
          current instanceof HTMLSelectElement || current.type === 'range') return;
    }
    if (current?.matches('input[type="range"], select')) return;

    const modal = activeModal();
    const region = activeRegion(current);
    const items = focusables(region);
    if (!items.length) return;

    event.preventDefault();
    event.stopImmediatePropagation();
    remoteMode = true;

    if (!items.includes(current)) {
      const preferred = region.matches?.('.sidebar')
        ? items.find((el) => el.matches('[aria-current="page"], .active')) || items[0]
        : items.find((el) => el.closest('#view')) || items.find((el) => el.closest('.topbar')) || items[0];
      focusElement(preferred);
      return;
    }

    if (region.matches?.('.sidebar')) {
      if (direction === 'up' || direction === 'down') {
        const next = nearest(current, items, direction);
        if (next) focusElement(next);
      } else if (direction === 'right') {
        const target = isVisible(lastMainFocus) && document.querySelector('#main-content')?.contains(lastMainFocus)
          ? lastMainFocus
          : focusables(document.querySelector('#main-content'))[0];
        if (target) focusElement(target);
      }
      return;
    }

    const group = current.closest(GROUPS);
    if (group && region.contains(group)) {
      const localItems = focusables(group);
      if (group.matches('.tp-list') && (direction === 'up' || direction === 'down')) {
        const index = localItems.indexOf(current);
        const nextIndex = index + (direction === 'down' ? 1 : -1);
        if (localItems[nextIndex]) focusElement(localItems[nextIndex]);
        return;
      }

      const localNext = nearest(current, localItems, direction, {
        alignedOnly: group.matches('.strip, #nav-main')
      });
      if (localNext) {
        focusElement(localNext);
        return;
      }
    }

    // O cálculo global nunca inclui a sidebar. Assim, cima/baixo e movimentos
    // a partir de colunas internas não escapam para o menu.
    const mainItems = modal ? items : focusables(document.querySelector('#main-content')).filter((el) => !el.closest('.topbar'));
    const next = nearest(current, mainItems, direction, { alignedOnly: true });
    if (next) {
      focusElement(next);
      return;
    }

    // Só entra no menu ao pressionar ESQUERDA quando não existe outro elemento
    // de conteúdo alinhado à esquerda: isto equivale à primeira coluna da linha/trilho.
    if (!modal && direction === 'left') {
      const sidebarItems = focusables(document.querySelector('.sidebar'));
      const nav = document.querySelector('#nav-main');
      const active = nav?.querySelector('[aria-current="page"], .active');
      const target = active && sidebarItems.includes(active) ? active : sidebarItems[0];
      if (target) focusElement(target);
    }
  }

  // Recupera o foco assim que uma rota aberta pelo D-pad termina de renderizar.
  function restorePendingRouteFocus() {
    if (!pendingRouteFocus || !remoteMode || activeModal()) return;

    const current = document.activeElement;
    if (current && current !== document.body && current.isConnected && isVisible(current)) {
      // Aguarda o router remover o controle da tela anterior. Se o usuário já moveu
      // o foco para outro lugar, respeita essa escolha em vez de roubá-lo.
      if (current === routeFocusOrigin) return;
      pendingRouteFocus = false;
      routeFocusOrigin = null;
      return;
    }

    const first = focusables(document.querySelector('#view'))[0];
    if (first && focusElement(first)) {
      pendingRouteFocus = false;
      routeFocusOrigin = null;
    }
  }

  // Modais prendem o foco e devolvem-no ao elemento que os abriu.
  const observer = new MutationObserver(() => {
    const modal = activeModal();
    if (modal) {
      if (!modal.contains(document.activeElement)) {
        if (!focusBeforeModal || !modal.contains(focusBeforeModal)) focusBeforeModal = document.activeElement;
        requestAnimationFrame(() => {
          const first = modal.querySelector('#close-player, .tp-item, button:not(:disabled), [href], input:not(:disabled)');
          if (first && isVisible(first)) focusElement(first);
        });
      }
    } else if (focusBeforeModal?.isConnected) {
      const previous = focusBeforeModal;
      focusBeforeModal = null;
      requestAnimationFrame(() => focusElement(previous));
    } else {
      restorePendingRouteFocus();
    }
  });
  observer.observe(document.body, { subtree: true, childList: true, attributes: true,
    attributeFilter: ['class', 'open', 'hidden', 'disabled', 'aria-hidden'] });

  document.addEventListener('keydown', (event) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const direction = directionOf(event);
    if (direction) {
      if (document.querySelector('#confirm[open]') && !activeModal()) return;
      handleDirection(event, direction);
      return;
    }

    if (isOk(event)) {
      const current = document.activeElement;
      if (!current || current === document.body || isTextEntry(current) ||
          current.matches('select, input[type="range"], video')) return;
      if (current.matches('a[href], button:not(:disabled), summary, [role="button"], [tabindex]:not([tabindex="-1"])')) {
        event.preventDefault();
        event.stopImmediatePropagation();
        current.click();
      }
      return;
    }

    if (event.key === 'Tab') {
      const modal = activeModal();
      if (!modal) return;
      const items = focusables(modal);
      if (!items.length) { event.preventDefault(); return; }
      const first = items[0], last = items[items.length - 1], current = document.activeElement;
      if (!modal.contains(current) || (event.shiftKey && current === first) || (!event.shiftKey && current === last)) {
        event.preventDefault();
        focusElement(event.shiftKey && modal.contains(current) ? last : first);
      }
    }
  }, true);

  // Campos de texto: Enter envia uma vez e fecha o teclado virtual; esquerda/direita
  // continuam sendo edição do texto. Escape/Voltar primeiro dispensam o IME.
  document.addEventListener('keydown', (event) => {
    const target = document.activeElement;
    if (!isTextEntry(target) || event.altKey || event.ctrlKey || event.metaKey) return;
    if (['Enter', 'Search', 'Go', 'Done'].includes(event.key) || event.keyCode === 66) {
      event.preventDefault();
      event.stopPropagation();
      if (target.form) target.form.requestSubmit();
      else if (target.id === 'q') target.dispatchEvent(new Event('change', { bubbles: true }));
      target.blur();
    }
  }, true);

  document.addEventListener('keydown', (event) => {
    if (!['Escape', 'Backspace', 'BrowserBack', 'GoBack'].includes(event.key) && event.keyCode !== 4) return;
    const target = document.activeElement;
    if (isTextEntry(target) && (event.key !== 'Backspace' || !target.value)) {
      event.preventDefault();
      target.blur();
      return;
    }
    const dialog = document.querySelector('#confirm[open], dialog[open]');
    if (dialog) {
      event.preventDefault();
      const cancel = dialog.querySelector('[value="cancel"], [data-action="cancel"]');
      if (cancel) cancel.click(); else dialog.close();
      return;
    }
    if (document.querySelector('#player:not(.hidden)')) {
      event.preventDefault();
      document.querySelector('#close-player')?.click();
      return;
    }
    if (location.hash && location.hash !== '#/' && location.hash !== '#') {
      event.preventDefault();
      if (history.length > 1) history.back(); else location.hash = '#/';
    }
  }, true);

  // O callback nativo Android intercepta Voltar antes de o evento chegar ao WebView.
  window.__cinehubBack = () => {
    const target = document.activeElement;
    if (isTextEntry(target)) { target.blur(); return true; }
    const dialog = document.querySelector('dialog[open]');
    if (dialog) {
      const cancel = dialog.querySelector('[value="cancel"]');
      if (cancel) cancel.click(); else dialog.close();
      return true;
    }
    if (document.querySelector('#player:not(.hidden)')) {
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

  document.addEventListener('focusin', (event) => {
    const modal = activeModal();
    const previous = event.relatedTarget;
    // showModal() pode mover o foco automaticamente antes do MutationObserver
    // executar; relatedTarget preserva o controle que abriu o diálogo.
    if (modal?.contains(event.target) && previous && previous !== document.body
      && !modal.contains(previous) && previous.isConnected) {
      focusBeforeModal = previous;
    }
    if (event.target?.closest?.('#main-content') && !event.target.closest('.hidden')) lastMainFocus = event.target;
    // focusin também ocorre após toques. O modo remoto é ativado somente
    // por uma navegação explícita via D-pad/teclado em handleDirection.
  });
  document.addEventListener('pointerdown', () => {
    remoteMode = false;
    pendingRouteFocus = false;
    routeFocusOrigin = null;
  }, true);
  window.addEventListener('hashchange', () => {
    if (!remoteMode) {
      pendingRouteFocus = false;
      routeFocusOrigin = null;
      return;
    }
    const current = document.activeElement;
    routeFocusOrigin = current?.closest?.('#view') ? current : null;
    pendingRouteFocus = true;
    // A tela pode exibir um estado de carregamento antes de montar seus botões.
    // A tentativa imediata cobre rotas síncronas; o observer cobre as assíncronas.
    requestAnimationFrame(restorePendingRouteFocus);
  });
})();
