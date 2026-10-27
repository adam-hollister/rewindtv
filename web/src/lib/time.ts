export const MIN = 60_000;

export function clock(ms: number, withAmPm = true): string {
  const d = new Date(ms);
  let h = d.getHours();
  const m = d.getMinutes().toString().padStart(2, '0');
  const ap = h >= 12 ? 'PM' : 'AM';
  h = h % 12 || 12;
  return withAmPm ? `${h}:${m} ${ap}` : `${h}:${m}`;
}

export function range(a: number, b: number): string {
  return `${clock(a, false)} – ${clock(b)}`;
}

export function minutesLeft(end: number, now: number): string {
  const m = Math.max(0, Math.round((end - now) / MIN));
  if (m >= 60) return `${Math.floor(m / 60)}h ${m % 60}m left`;
  return `${m}m left`;
}

export function duration(ms: number): string {
  const m = Math.round(ms / MIN);
  return m >= 60 ? `${Math.floor(m / 60)}h ${m % 60 ? (m % 60) + 'm' : ''}`.trim() : `${m}m`;
}

export function progress(p: { start: number; end: number }, now: number): number {
  return Math.min(1, Math.max(0, (now - p.start) / (p.end - p.start)));
}

export function minToTime(min: number): string {
  const h = Math.floor(min / 60) % 24;
  return `${h.toString().padStart(2, '0')}:${(min % 60).toString().padStart(2, '0')}`;
}

export function timeToMin(t: string): number {
  const [h, m] = t.split(':').map(Number);
  return (h || 0) * 60 + (m || 0);
}

export const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

export function daysLabel(mask: number): string {
  if (mask === 127) return 'Every day';
  if (mask === 31) return 'Weekdays';
  if (mask === 96) return 'Weekends';
  return DAYS.filter((_, i) => mask & (1 << i)).join(', ');
}

/** 92500 -> "1:32.5" (tenths only when present). */
export function mmss(ms: number): string {
  const neg = ms < 0;
  const t = Math.abs(ms) / 1000;
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  const ss = Number.isInteger(Math.round(s * 10) / 10) ? Math.round(s).toString().padStart(2, '0') : s.toFixed(1).padStart(4, '0');
  return `${neg ? '-' : ''}${m}:${ss}`;
}

/** "1:32", "92", "1:32.5" -> ms; null if not a time. */
export function parseMmss(v: string): number | null {
  const t = v.trim();
  if (!t) return null;
  const parts = t.split(':').map(Number);
  if (parts.some((p) => !Number.isFinite(p) || p < 0)) return null;
  const secs = parts.reduce((acc, p) => acc * 60 + p, 0);
  return Math.round(secs * 1000);
}
