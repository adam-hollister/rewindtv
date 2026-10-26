export type Ordering = 'shuffle' | 'episodic' | 'block' | 'marathon';

export interface Rule {
  libraries: number[];
  kinds: string[];
  genres: string[];
  exclude_genres: string[];
  collections: string[];
  shows: string[];
  studios: string[];
  ratings: string[];
  year_min: number | null;
  year_max: number | null;
  include_items: number[];
  exclude_items: number[];
}

export const emptyRule = (): Rule => ({
  libraries: [], kinds: [], genres: [], exclude_genres: [], collections: [], shows: [],
  studios: [], ratings: [], year_min: null, year_max: null, include_items: [], exclude_items: [],
});

export interface User { id: number; name: string; color: string; is_admin: boolean }

export interface Program {
  id: number;
  start: number;
  end: number;
  title: string;
  subtitle: string | null;
  episode_title: string | null;
  kind: 'movie' | 'episode' | null;
  filler: boolean;
  thumb: string | null;
  art: string | null;
  summary: string | null;
  rating: string | null;
  year: number | null;
  season: number | null;
  episode: number | null;
  block: string | null;
}

export interface ChannelRow {
  id: number;
  number: number;
  name: string;
  color: string;
  logo: string | null;
  owner_user_id: number | null;
  favorite: number;
  hidden: number;
}

export interface GuideChannel extends ChannelRow { programs: Program[] }
export interface Guide { now: number; from: number; to: number; channels: GuideChannel[] }
export interface ChannelListItem extends ChannelRow { now: Program | null }

export interface Slot {
  id?: number | null;
  title: string;
  days: number;
  start_min: number;
  end_min: number;
  rule: Rule;
  ordering: Ordering;
}

export interface Channel {
  id?: number | null;
  number?: number | null;
  name: string;
  color?: string | null;
  logo?: string | null;
  owner_user_id: number | null;
  rule: Rule;
  ordering: Ordering;
  block_size: number;
  avoid_repeat_hours: number;
  align_minutes: number;
  slots: Slot[];
}

export interface Tune {
  kind: 'stream' | 'filler';
  session: string | null;
  url: string | null;
  offset: number;
  duration: number;
  server_time: number;
  program: Program;
  next: Program | null;
}

export interface Library { id: number; source_id: number; name: string; kind: string; enabled: boolean; items: number }
export interface Source {
  id: number; kind: 'plex' | 'jellyfin'; name: string; url: string; enabled: boolean;
  last_sync: number | null; last_error: string | null; libraries: Library[];
}
export interface SyncStatus { running: boolean; message: string | null; finished_at: number | null }
export interface Count { name: string; count: number }
export interface Facets {
  libraries: { id: number; name: string; source: string }[];
  genres: Count[]; collections: Count[]; studios: Count[]; ratings: Count[];
  shows: { key: string; title: string; episodes: number }[];
  year_min: number | null; year_max: number | null;
}
export interface RulePreview { items: number; hours: number; shows: number; movies: number; sample: string[] }
export interface Suggestion { kind: string; name: string; rule: Rule; ordering: Ordering; items: number; hours: number }
export interface PreviewProgram { start: number; end: number; title: string; subtitle: string | null; block: string | null; filler: boolean }
export interface PlexPin { id: number; code: string; link_url: string }
export interface PlexServer {
  name: string; owned: boolean; token: string; best: string | null;
  connections: { uri: string; local: boolean; reachable: boolean }[];
}
export interface DiagnoseCheck { name: string; ok: boolean; detail: string; ms: number }
export interface DiagnoseReport { source: string; kind: string; item: string | null; ok: boolean; checks: DiagnoseCheck[] }
export interface Status { version: string; timezone: string; sources: number; items: number; channels: number; users: number; sessions: number }

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    let msg = res.statusText;
    try { msg = (await res.json()).error ?? msg; } catch { /* not json */ }
    throw new Error(msg);
  }
  return res.json() as Promise<T>;
}

const q = (o: Record<string, unknown>) =>
  '?' + Object.entries(o).filter(([, v]) => v !== undefined && v !== null).map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`).join('&');

export const api = {
  status: () => req<Status>('GET', '/api/status'),
  users: () => req<User[]>('GET', '/api/users'),
  createUser: (u: Partial<User>) => req<User>('POST', '/api/users', u),
  updateUser: (id: number, u: Partial<User>) => req<User>('PATCH', `/api/users/${id}`, u),
  deleteUser: (id: number) => req<null>('DELETE', `/api/users/${id}`),
  setPrefs: (uid: number, cid: number, p: { hidden?: boolean; favorite?: boolean }) => req<null>('PUT', `/api/users/${uid}/channels/${cid}`, p),

  sources: () => req<Source[]>('GET', '/api/sources'),
  testSource: (s: { kind: string; url: string; token: string }) => req<{ ok: boolean; name: string | null; error: string | null }>('POST', '/api/sources/test', s),
  addSource: (s: { kind: string; url: string; token: string; name?: string }) => req<{ id: number; name: string }>('POST', '/api/sources', s),
  updateSource: (id: number, s: Partial<{ name: string; url: string; token: string; enabled: boolean }>) => req<null>('PATCH', `/api/sources/${id}`, s),
  deleteSource: (id: number) => req<null>('DELETE', `/api/sources/${id}`),
  setLibrary: (id: number, enabled: boolean) => req<null>('PATCH', `/api/libraries/${id}`, { enabled }),
  diagnose: (id: number) => req<DiagnoseReport>('POST', `/api/sources/${id}/diagnose`),
  plexPin: () => req<PlexPin>('POST', '/api/plex/pin'),
  plexPinStatus: (id: number) => req<{ token: string | null }>('GET', `/api/plex/pin/${id}`),
  plexServers: (token: string) => req<PlexServer[]>('GET', `/api/plex/servers?token=${encodeURIComponent(token)}`),
  syncStatus: () => req<SyncStatus>('GET', '/api/sync'),
  syncNow: () => req<SyncStatus>('POST', '/api/sync'),

  facets: () => req<Facets>('GET', '/api/library/facets'),
  previewRule: (r: Rule) => req<RulePreview>('POST', '/api/library/preview', r),
  suggestions: () => req<Suggestion[]>('GET', '/api/autogen'),
  autogen: (channels: { name: string; rule: Rule; ordering: Ordering }[], owner_user_id: number | null) =>
    req<number[]>('POST', '/api/autogen', { channels, owner_user_id }),

  channels: (user?: number) => req<ChannelListItem[]>('GET', '/api/channels' + q({ user })),
  channel: (id: number) => req<Channel>('GET', `/api/channels/${id}`),
  createChannel: (c: Channel) => req<{ id: number }>('POST', '/api/channels', c),
  updateChannel: (id: number, c: Channel) => req<null>('PUT', `/api/channels/${id}`, c),
  deleteChannel: (id: number) => req<null>('DELETE', `/api/channels/${id}`),
  previewChannel: (c: Channel) => req<PreviewProgram[]>('POST', '/api/channels/preview', c),
  rebuildAll: () => req<null>('POST', '/api/channels/rebuild'),

  guide: (user: number | undefined, from?: number, hours?: number) => req<Guide>('GET', '/api/guide' + q({ user, from, hours })),
  tune: (id: number, previous?: string | null) => req<Tune>('POST', `/api/channels/${id}/tune`, { previous: previous ?? null }),
  stopSession: (sid: string) => fetch(`/api/sessions/${sid}`, { method: 'DELETE', keepalive: true }),
};
