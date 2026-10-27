import Hls from 'hls.js';

/** Total length of the first variant playlist, to see whether the server
 *  started at the requested offset or returned the whole program. */
export async function playlistSeconds(url: string): Promise<number> {
  try {
    let text = await (await fetch(url)).text();
    if (text.includes('#EXT-X-STREAM-INF')) {
      const lines = text.split('\n');
      const i = lines.findIndex((l) => l.startsWith('#EXT-X-STREAM-INF'));
      const variant = lines.slice(i + 1).find((l) => l.trim() && !l.startsWith('#'));
      if (!variant) return 0;
      text = await (await fetch(new URL(variant.trim(), new URL(url, location.href)).toString())).text();
    }
    return [...text.matchAll(/#EXTINF:([\d.]+)/g)].reduce((s, m) => s + parseFloat(m[1]), 0);
  } catch {
    return 0;
  }
}

/**
 * Play `url` in `video` from file position `at` (seconds). Returns the file
 * position that currentTime 0 maps to, and a cleanup function.
 */
export async function playFrom(video: HTMLVideoElement, url: string, at: number, duration: number): Promise<{ base: number; destroy: () => void }> {
  const total = await playlistSeconds(url);
  const fullLength = total > 0 && Math.abs(total - duration) < Math.abs(total - (duration - at));
  const base = fullLength ? 0 : at;
  const start = fullLength ? at : 0;
  if (Hls.isSupported()) {
    const hls = new Hls({ startPosition: start, maxBufferLength: 30 });
    hls.loadSource(url);
    hls.attachMedia(video);
    return { base, destroy: () => hls.destroy() };
  }
  video.src = url;
  video.addEventListener('loadedmetadata', () => (video.currentTime = start), { once: true });
  return { base, destroy: () => video.removeAttribute('src') };
}
