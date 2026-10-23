use std::path::PathBuf;

use chrono_tz::Tz;

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub port: u16,
    /// Time zone used for time-of-day blocks and guide alignment.
    pub tz: Tz,
    /// Base URL used in M3U/XMLTV links. Derived from the request when unset.
    pub public_url: Option<String>,
    /// "copy" remuxes the source video for IPTV streams; "h264" re-encodes so
    /// every program has identical codec parameters.
    pub iptv_video: String,
    pub ffmpeg: String,
    pub sync_hours: u64,
    /// Profile whose lineup the root-level HDHomeRun tuner exposes (0 = shared only).
    pub hdhr_profile: i64,
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
        Ok(Self {
            data_dir: env("REWINDTV_DATA").unwrap_or_else(|| "/data".into()).into(),
            port: env("REWINDTV_PORT").and_then(|p| p.parse().ok()).unwrap_or(8420),
            tz,
            public_url: env("REWINDTV_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string()),
            iptv_video: env("REWINDTV_IPTV_VIDEO").unwrap_or_else(|| "copy".into()),
            ffmpeg: env("REWINDTV_FFMPEG").unwrap_or_else(|| "ffmpeg".into()),
            sync_hours: env("REWINDTV_SYNC_HOURS").and_then(|h| h.parse().ok()).unwrap_or(6),
            hdhr_profile: env("REWINDTV_HDHR_PROFILE").and_then(|h| h.parse().ok()).unwrap_or(0),
        })
    }
}
