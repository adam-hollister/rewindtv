mod channels;
mod diagnose;
mod library;
mod markers;
mod media;
mod plex;
mod sources;
mod users;

use axum::routing::{get, patch, post, put};
use axum::Router;

use crate::state::AppState;
use crate::stream::{hls, iptv, tuner};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/status", get(media::status))
        .route("/api/users", get(users::list).post(users::create))
        .route("/api/users/{id}", patch(users::update).delete(users::remove))
        .route("/api/users/{uid}/channels/{cid}", put(users::set_channel_prefs))
        .route("/api/sources", get(sources::list).post(sources::create))
        .route("/api/sources/test", post(sources::test))
        .route("/api/sources/{id}", patch(sources::update).delete(sources::remove))
        .route("/api/sources/{id}/diagnose", post(diagnose::diagnose))
        .route("/api/plex/pin", post(plex::create_pin))
        .route("/api/plex/pin/{id}", get(plex::check_pin))
        .route("/api/plex/servers", get(plex::servers))
        .route("/api/libraries/{id}", patch(sources::set_library))
        .route("/api/sync", get(sources::sync_status).post(sources::sync_now))
        .route("/api/library/facets", get(library::facets))
        .route("/api/library/preview", post(library::preview))
        .route("/api/autogen", get(library::suggestions).post(channels::create_from_suggestions))
        .route("/api/channels", get(channels::list).post(channels::create))
        .route("/api/channels/preview", post(channels::preview))
        .route("/api/channels/rebuild", post(channels::rebuild_all))
        .route("/api/channels/{id}", get(channels::get).put(channels::update).delete(channels::remove))
        .route("/api/channels/{id}/rebuild", post(channels::rebuild))
        .route("/api/channels/{id}/markers", get(markers::channel_coverage))
        .route("/api/markers/settings", get(markers::settings).put(markers::set_settings))
        .route("/api/markers/scan", get(markers::scan_status).post(markers::scan))
        .route("/api/markers/shows", get(markers::shows))
        .route("/api/markers/show", get(markers::show))
        .route("/api/markers/episode", put(markers::edit_episode))
        .route("/api/markers/episode/{id}/kept", get(markers::preview_cuts))
        .route("/api/items/{id}/play", post(markers::play_item))
        .route("/api/markers/rule", put(markers::save_rule).delete(markers::delete_rule))
        .route("/api/channels/{id}/logo.svg", get(media::logo))
        .route("/api/channels/{id}/tune", post(media::tune))
        .route("/api/sessions/{sid}", axum::routing::delete(media::stop_session))
        .route("/api/guide", get(channels::guide))
        .route("/api/img/{source}", get(media::image))
        .route("/stream/{sid}/{rest}", get(hls::proxy))
        .route("/iptv/{user}/playlist.m3u", get(iptv::playlist))
        .route("/iptv/{user}/xmltv.xml", get(iptv::xmltv))
        .route("/iptv/stream/{file}", get(tuner::stream))
        // HDHomeRun emulation: Plex probes these at the server root.
        .route("/discover.json", get(iptv::hdhr_root))
        .route("/lineup.json", get(iptv::hdhr_root))
        .route("/lineup_status.json", get(iptv::hdhr_root))
        .route("/device.xml", get(iptv::hdhr_root))
        .route("/lineup.post", get(iptv::hdhr_root).post(iptv::hdhr_root))
        .route("/hdhr/{user}/{file}", get(iptv::hdhr_profile).post(iptv::hdhr_profile))
}
