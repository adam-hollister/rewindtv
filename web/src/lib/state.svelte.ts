import { api, type User } from './api';

const USER_KEY = 'rewindtv.user';

function readUserId(): number | null {
  try {
    const v = localStorage.getItem(USER_KEY);
    return v ? Number(v) : null;
  } catch {
    return null;
  }
}

export const session = $state({
  users: [] as User[],
  userId: readUserId(),
  loaded: false,
});

export function currentUser(): User | undefined {
  return session.users.find((u) => u.id === session.userId);
}

export async function loadUsers() {
  session.users = await api.users();
  if (session.userId !== null && !session.users.some((u) => u.id === session.userId)) selectUser(null);
  session.loaded = true;
}

export function selectUser(id: number | null) {
  session.userId = id;
  try {
    if (id === null) localStorage.removeItem(USER_KEY);
    else localStorage.setItem(USER_KEY, String(id));
  } catch { /* storage unavailable */ }
}

// --- Tiny hash router -------------------------------------------------------

function parse(): string[] {
  return location.hash.replace(/^#\/?/, '').split('/').filter(Boolean);
}

export const route = $state({ parts: parse() });

window.addEventListener('hashchange', () => (route.parts = parse()));

export function go(path: string) {
  location.hash = '#/' + path.replace(/^\//, '');
}

// --- Toasts -----------------------------------------------------------------

export const toasts = $state({ list: [] as { id: number; text: string; kind: 'ok' | 'err' }[] });
let toastId = 0;

export function toast(text: string, kind: 'ok' | 'err' = 'ok') {
  const id = ++toastId;
  toasts.list.push({ id, text, kind });
  setTimeout(() => (toasts.list = toasts.list.filter((t) => t.id !== id)), 4000);
}

// Shared clock that ticks every 15s, for "now" lines and progress bars.
export const clockState = $state({ now: Date.now() });
setInterval(() => (clockState.now = Date.now()), 15_000);
