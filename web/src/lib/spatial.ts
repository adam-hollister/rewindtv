// Minimal spatial navigation so the UI works with a TV remote / D-pad.
// Arrow keys move focus to the nearest focusable element in that direction.

const SELECTOR = 'button:not([disabled]), a[href], input:not([type=hidden]):not([disabled]), select, textarea, [tabindex]:not([tabindex="-1"])';

function visible(el: HTMLElement) {
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden';
}

export function moveFocus(dir: 'up' | 'down' | 'left' | 'right', root: ParentNode = document): boolean {
  const active = document.activeElement as HTMLElement | null;
  const all = [...root.querySelectorAll<HTMLElement>(SELECTOR)].filter(visible);
  if (!active || active === document.body || !all.includes(active)) {
    all[0]?.focus();
    return !!all[0];
  }
  const a = active.getBoundingClientRect();
  const ax = a.left + a.width / 2;
  const ay = a.top + a.height / 2;
  let best: HTMLElement | null = null;
  let bestScore = Infinity;
  for (const el of all) {
    if (el === active) continue;
    const r = el.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    const dx = x - ax;
    const dy = y - ay;
    const ok =
      (dir === 'up' && r.bottom <= a.top + 2) ||
      (dir === 'down' && r.top >= a.bottom - 2) ||
      (dir === 'left' && r.right <= a.left + 2) ||
      (dir === 'right' && r.left >= a.right - 2);
    if (!ok) continue;
    const primary = dir === 'up' || dir === 'down' ? Math.abs(dy) : Math.abs(dx);
    const secondary = dir === 'up' || dir === 'down' ? Math.abs(dx) : Math.abs(dy);
    const score = primary + secondary * 2;
    if (score < bestScore) {
      bestScore = score;
      best = el;
    }
  }
  if (best) {
    best.focus();
    best.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }
  return !!best;
}

const KEYS: Record<string, 'up' | 'down' | 'left' | 'right'> = {
  ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right',
};

/** Global handler; components that manage arrows themselves call preventDefault first. */
export function installSpatialNav() {
  window.addEventListener('keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const dir = KEYS[e.key];
    if (!dir) return;
    const t = e.target as HTMLElement;
    // Let text inputs keep left/right for the caret.
    if ((t.tagName === 'INPUT' && !['checkbox', 'radio', 'button', 'range', 'color'].includes((t as HTMLInputElement).type)) || t.tagName === 'TEXTAREA') {
      if (dir === 'left' || dir === 'right') return;
    }
    if (t.tagName === 'SELECT') return;
    if (moveFocus(dir)) e.preventDefault();
  });
}
