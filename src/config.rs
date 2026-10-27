use std::path::PathBuf;

use chrono_tz::Tz;

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// Intro/credits marker files, kept apart from the media.
    pub markers_dir: PathBuf,
    /// Server path prefix -> local path prefix, for reading media straight
    /// from a mounted folder (faster marker detection). From
    /// REWINDTV_PATH_MAP="/data/media=/media;/mnt/tv=/tv".
    pub path_maps: Vec<(String, String)>,
    pub port: u16,
    /// Time zone used for time-of-day blocks and guide alignment.
    pub tz: Tz,
    /// Base URL used in M3U/XMLTV links. Derived from the request when unset.
    pub public_url: Option<String>,
    /// How Live TV / IPTV streams get their video:
    /// "server" (default) has Plex/Jellyfin transcode every program to the same
    /// 1080p H.264 + AAC; "copy" passes original files through untouched;
    /// "h264" re-encodes original files inside RewindTV.
    pub iptv_video: String,
    pub ffmpeg: String,
    pub ffprobe: String,
    pub sync_hours: u64,
    /// Profile whose lineup the root-level HDHomeRun tuner exposes (0 = shared only).
    pub hdhr_profile: i64,
    /// Tuner count advertised to Plex (how many channels can be watched at once).
    pub tuners: u8,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let tz_name = env("TZ").unwrap_or_else(|| "UTC".into());
        let tz = tz_name
            .parse::<Tz>()
            .map_err(|e| anyhow::anyhow!("invalid TZ {tz_name:?}: {e}"))?;
        let ffmpeg = env("REWINDTV_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
        let data_dir: PathBuf = env("REWINDTV_DATA").unwrap_or_else(|| "/data".into()).into();
        Ok(Self {
            path_maps: env("REWINDTV_PATH_MAP")
                .map(|v| v.split(';').filter_map(|p| p.split_once('=')).map(|(a, b)| (a.trim().to_string(), b.trim().to_string())).collect())
                .unwrap_or_default(),
            markers_dir: env("REWINDTV_MARKERS_DIR").map(PathBuf::from).unwrap_or_else(|| data_dir.join("markers")),
            data_dir,
            port: env("REWINDTV_PORT").and_then(|p| p.parse().ok()).unwrap_or(8420),
            tz,
            public_url: env("REWINDTV_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string()),
            iptv_video: env("REWINDTV_IPTV_VIDEO").unwrap_or_else(|| "server".into()),
            ffmpeg: ffmpeg.clone(),
            // ffprobe ships next to ffmpeg (also in jellyfin-ffmpeg).
            ffprobe: env("REWINDTV_FFPROBE").unwrap_or_else(|| match ffmpeg.strip_suffix("ffmpeg") {
                Some(dir) => format!("{dir}ffprobe"),
                None => "ffprobe".into(),
            }),
            sync_hours: env("REWINDTV_SYNC_HOURS").and_then(|h| h.parse().ok()).unwrap_or(6),
            hdhr_profile: env("REWINDTV_HDHR_PROFILE").and_then(|h| h.parse().ok()).unwrap_or(0),
            tuners: env("REWINDTV_TUNERS").and_then(|t| t.parse().ok()).filter(|t| *t > 0).unwrap_or(6),
        })
    }
}
