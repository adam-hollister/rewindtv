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
