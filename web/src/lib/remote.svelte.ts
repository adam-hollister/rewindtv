// TV side of "phone as a remote": hold a pairing code and act on commands
// that a phone sends to it. Lives for the whole tab, so the code stays the
// same while you flip channels.
import { api } from './api';
import { go, route, session } from './state.svelte';

export type RemoteCommand =
  | { cmd: 'up' | 'down' | 'last' | 'mute' | 'info' | 'guide' | 'start_over' }
  | { cmd: 'number'; number: number }
  | { cmd: 'tune'; channel_id: number };

export const tvRemote = $state({ code: null as string | null, card: false });

const KEY = 'rewindtv.remote';
let source: EventSource | null = null;

function key(k: string) {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: k }));
}

function currentChannel(): string | null {
  try {
    return sessionStorage.getItem('rewindtv.current');
  } catch {
    return null;
  }
}

async function handle(c: RemoteCommand) {
  tvRemote.card = false;
  if (c.cmd === 'tune') return go(`watch/${c.channel_id}`);
  if (route.parts[0] !== 'watch') {
    // Not watching right now: these all mean "turn the TV on".
    if (c.cmd === 'number') {
      const list = await api.channels(session.userId ?? undefined);
      const ch = list.find((x) => x.number === c.number);
      if (ch) return go(`watch/${ch.id}`);
      return;
    }
    const cur = currentChannel();
    if (cur) return go(`watch/${cur}`);
    const list = await api.channels(session.userId ?? undefined);
    const first = list.find((x) => !x.hidden);
    if (first) go(`watch/${first.id}`);
    return;
  }
  switch (c.cmd) {
    case 'up': return key('ArrowUp');
    case 'down': return key('ArrowDown');
    case 'last': return key('l');
    case 'mute': return key('m');
    case 'info': return key('i');
    case 'guide': return key('g');
    case 'start_over': return key('s');
    case 'number':
      for (const d of String(c.number)) key(d);
      return key('Enter');
  }
}

/** Start listening (once per tab). */
export async function startTvRemote() {
  if (source) return;
  let saved: string | null = null;
  try { saved = sessionStorage.getItem(KEY); } catch { /* ignore */ }
  try {
    const { code } = await api.remotePair(saved);
    tvRemote.code = code;
    try { sessionStorage.setItem(KEY, code); } catch { /* ignore */ }
    // EventSource reconnects by itself; the server keeps the code.
    source = new EventSource(`/api/remote/${code}/events`);
    source.onmessage = (e) => {
      try { handle(JSON.parse(e.data)); } catch { /* ignore bad message */ }
    };
  } catch {
    source = null;
  }
}
