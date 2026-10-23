# RewindTV

Turn your **Plex** and **Jellyfin** libraries into old-school live TV channels. Shows air on a schedule, you tune in mid-episode, and there's a real TV guide. It's a self-hosted take on the Nostalgia TV idea, built to run as a single Docker container on TrueNAS SCALE or anything else that runs Docker.

<p align="center">
  <img src="docs/screenshots/guide.png" width="49%" alt="TV guide" />
  <img src="docs/screenshots/watch.png" width="49%" alt="Live TV mode" />
</p>
<p align="center">
  <img src="docs/screenshots/editor.png" width="49%" alt="Channel editor" />
  <img src="docs/screenshots/guide-mobile.png" width="20%" alt="Guide on a phone" />
</p>

*(Screenshots use generated test clips; your own posters and backdrops fill the guide's hero and channel cards.)*

## Features

- **Plex and Jellyfin, together.** Connect any number of servers. Channels can mix libraries from both.
- **Real live TV.** Each channel has a 7-day schedule generated on the server, so every TV in the house shows the same thing at the same moment. Tuning in starts mid-show, at exactly where "live" is.
- **TV guide.** A scrolling program grid with a now line, a live progress bar on each channel and poster/backdrop previews. It works with mouse, touch, keyboard or a TV remote's D-pad.
- **Full-screen TV mode.**
  - Channel up/down with arrow keys, PgUp/PgDn or swipes.
  - Type a channel number like a real remote.
  - An info banner shows what's on and what's up next.
  - A quick channel list (`G`), last channel (`L`), and a burst of analog static when you change channels.
- **Channel builder.**
  - Filter by genre, decade/year, specific shows, collections, networks/studios, ratings and libraries.
  - A live "matches N items / H hours" counter updates as you go.
  - A 24-hour schedule preview shows what will air before you save.
- **Four ways to program a channel:**
  - *Shuffle*: everything airs before anything repeats.
  - *Rotation*: one episode per show, each show in order.
  - *Blocks*: N episodes of one show, then the next show.
  - *Marathon*: whole series back-to-back.
- **Time-of-day blocks.** Saturday-morning cartoons, prime-time sitcoms, late-night movies. A weekly timeline shows the layout, and blocks start on time.
- **Auto-create.** Build channels in one click from your genres, decades, collections, studios, or "Show 24/7" marathons.
- **Profiles.** A Netflix-style "Who's watching?" screen. Each profile gets its own favorites, hidden channels and private channels; shared channels show for everyone. There are no passwords, since this is meant for your home network.
- **Watch anywhere (IPTV).** Every profile gets an M3U playlist and an XMLTV guide for **Jellyfin Live TV**, TiviMate, VLC and similar apps. RewindTV also acts as an HDHomeRun tuner for **Plex Live TV**. Channels are continuous MPEG-TS streams that roll from one program to the next.
- **Light on your NAS.** Browser playback uses your media server's own transcoder, including hardware acceleration if you have it. RewindTV proxies the stream, so server tokens never reach the browser. Abandoned streams are stopped automatically when someone channel-surfs.

## Install on TrueNAS SCALE

These steps are for TrueNAS SCALE 24.10 "Electric Eel" or newer.

1. **Create a dataset** for the app's data, for example `tank/apps/rewindtv`.
2. Go to **Apps → Discover Apps → ⋮ (top right) → Install via YAML**.
3. Paste [`truenas/rewindtv.yaml`](truenas/rewindtv.yaml). Change the host path to your dataset and `TZ` to your time zone.
4. Open `http://<truenas-ip>:8420`, create a profile, then go to **Settings → Add server**.

> **Private repository note:** GitHub publishes the image to `ghcr.io/adam-hollister/rewindtv`. If this repository is private, the package is private too, and TrueNAS can't pull it without credentials. You have two options:
> - Make the package public: on GitHub, go to **your profile → Packages → rewindtv → Package settings → Change visibility**.
> - Or add a GHCR credential in TrueNAS: **Apps → Configuration → Manage Container Image Registries**. Use your GitHub username and a personal access token with the `read:packages` scope.

### Docker Compose (any host)

```sh
docker compose up -d   # uses docker-compose.yml in this repo
```

## Connecting your media servers

| Server | URL example | Credential |
| --- | --- | --- |
| Jellyfin | `http://192.168.1.10:8096` | **Dashboard → API Keys → +**, then paste the key |
| Plex | `http://192.168.1.10:32400` | Your Plex token: open any item in Plex Web → **⋯ → Get Info → View XML**, and copy the `X-Plex-Token=` value from the URL |

Libraries sync on startup and every 6 hours. **Settings → Sync now** syncs immediately. You can switch individual libraries off.

## Using it as a tuner in Plex / Jellyfin / IPTV apps

**Settings → Watch in other apps** lists two URLs per profile:

- `http://<host>:8420/iptv/<profile>/playlist.m3u`
- `http://<host>:8420/iptv/<profile>/xmltv.xml`

Profile `0` is the shared channels only.

- **Jellyfin:** **Dashboard → Live TV → Tuner devices → +** (M3U Tuner), then **TV guide data providers → XMLTV**.
- **Plex:** RewindTV also emulates an **HDHomeRun** tuner, which is what Plex Live TV expects.
  1. In Plex go to **Settings → Live TV & DVR → Set up Plex DVR**. If RewindTV isn't found automatically, enter `<host>:8420` manually.
  2. When asked for a guide, choose **XMLTV** and paste the XMLTV URL above.

  Plex DVR requires Plex Pass. The root-level tuner shows the shared channels. Set `REWINDTV_HDHR_PROFILE=<id>` to expose a specific profile's lineup instead; per-profile tuners also live at `http://<host>:8420/hdhr/<profile>/`.

By default IPTV streams copy the source video untouched and only convert audio to AAC, which keeps the CPU near idle. If your library mixes codecs and a picky IPTV player stutters when a new program starts, set `REWINDTV_IPTV_VIDEO=h264` to re-encode everything to identical 720p H.264.

## Remote / keyboard shortcuts (TV mode)

| Key | Action |
| --- | --- |
| ↑ / ↓, PgUp / PgDn, + / − | Channel up / down |
| 0–9 | Type a channel number |
| G | Channel list |
| L | Last channel |
| ← / → / I / Enter | Show info banner |
| M / F | Mute / fullscreen |
| Esc / Backspace | Back to the guide |

In the guide, arrows move between programs, Enter tunes in, and typing a number jumps straight to that channel.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `TZ` | `UTC` | Time zone for time-of-day blocks, alignment and the guide |
| `REWINDTV_DATA` | `/data` | Where the SQLite database lives |
| `REWINDTV_PORT` | `8420` | HTTP port |
| `REWINDTV_PUBLIC_URL` | *(from request)* | Base URL written into M3U/XMLTV links |
| `REWINDTV_IPTV_VIDEO` | `copy` | `copy` or `h264` for IPTV streams |
| `REWINDTV_SYNC_HOURS` | `6` | Library sync interval |
| `REWINDTV_HDHR_PROFILE` | `0` | Profile whose lineup the HDHomeRun tuner (for Plex) exposes; `0` = shared channels |

## How it works

- **Backend:** Rust (axum, sqlx/SQLite, reqwest), in `src/`.
  - `schedule/` holds the pure scheduling engine: filters, ordering cursors and time-of-day blocks. It's unit-tested, including DST transitions.
  - `media/` holds the Plex and Jellyfin clients.
  - `stream/` holds the HLS proxy and the IPTV muxer.
- **Frontend:** Svelte 5 + Vite, in `web/`. It's compiled into the binary, so the image is a single executable plus ffmpeg.
- **Guide:** Each channel keeps a rolling 7-day guide that is extended hourly. Editing a channel rebuilds its future schedule while the program on air keeps playing.

## Development

```sh
# backend (serves the API on :8420; uses ./data)
REWINDTV_DATA=./data TZ=America/Chicago cargo run

# frontend with hot reload on :5173 (proxies /api to :8420)
cd web && npm install && npm run dev

cargo test           # scheduler, filters, parsers, HLS rewriting, TS scanner
cd web && npm run check
```

Pushing to `main` runs the tests and publishes `ghcr.io/<owner>/rewindtv:latest`. Pushing a `v*` tag also publishes a version-tagged image.

## Credits

Inspired by [Nostalgiavision](https://github.com/nexusjuan12/Nostalgiavision) and the Nostalgia TV app.
