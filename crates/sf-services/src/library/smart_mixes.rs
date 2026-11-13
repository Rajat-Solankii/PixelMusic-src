//! Saved Smart Mix playlists (Settings → Last.fm Integration → Smart Mix Playlists). A mix saved from the Smart
//! Mix page becomes a private YouTube Music playlist; its URI and save time are kept in the setting
//! `lastfmMixPlaylists` (JSON `[{"uri","created"}]`), so that:
//!  - "Show Smart Mix playlists in Library" (`lastfmShowMixes`) can hide them from the Library playlists list;
//!  - "Generated Playlists Retention" (`lastfmMixRetentionDays`, 0 — keep) deletes them from the account once
//!    they are older than that (checked after each library read and when the setting changes).
use std::collections::HashSet;

use mf_platform::settings::SettingKey;
use serde::{Deserialize, Serialize};

use super::{LibraryEvent, LibraryService};
use crate::youtube::playlist_uri;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedMix {
    pub uri: String,
    /// unix seconds
    pub created: i64,
}

pub fn parse(text: &str) -> Vec<SavedMix> {
    serde_json::from_str(text).unwrap_or_default()
}

/// The mixes past `days` (0 — none) at `now` (unix seconds).
pub fn expired(mixes: &[SavedMix], days: i64, now: i64) -> Vec<SavedMix> {
    if days <= 0 {
        return Vec::new();
    }
    mixes.iter().filter(|mix| now - mix.created >= days * 86_400).cloned().collect()
}

fn unix_now() -> i64 {
    jiff::Timestamp::now().as_second()
}

impl LibraryService {
    fn saved_mixes(&self) -> Vec<SavedMix> {
        parse(&self.settings.get_str(SettingKey::LastFmMixPlaylists))
    }

    fn write_saved_mixes(&self, mixes: &[SavedMix]) {
        let text = if mixes.is_empty() { String::new() } else { serde_json::to_string(mixes).unwrap_or_default() };
        self.settings.set(SettingKey::LastFmMixPlaylists, mf_platform::settings::SettingValue::Str(text));
    }

    /// URIs of the saved Smart Mix playlists the Library list leaves out (empty while they are shown).
    pub(super) fn hidden_mix_uris(&self) -> HashSet<String> {
        if self.settings.get_bool(SettingKey::LastFmShowMixes) {
            return HashSet::new();
        }
        self.saved_mixes().into_iter().map(|mix| mix.uri).collect()
    }

    /// Save a generated Smart Mix: a YouTube Music playlist remembered as a Smart Mix; songs from elsewhere
    /// go the usual way (`create_playlist_from`) and are not tracked.
    pub fn save_smart_mix(&self, title: String, uris: Vec<String>) {
        if uris.is_empty() || !uris.iter().all(|uri| super::yt::is_youtube(uri)) {
            return self.create_playlist_from(title, uris);
        }
        let title = if title.trim().is_empty() { mf_i18n::t("Smart Mix") } else { title.trim().to_owned() };
        self.create_youtube_playlist_then(title, uris, |service, playlist_id| {
            let mut mixes = service.saved_mixes();
            mixes.push(SavedMix { uri: playlist_uri(&playlist_id), created: unix_now() });
            service.write_saved_mixes(&mixes);
        });
    }

    /// Show / hide setting changed: the Library list is built again.
    pub fn smart_mix_visibility_changed(&self) {
        self.send([LibraryEvent::EntriesChanged]);
    }

    /// Delete the saved mixes past the retention period from the account, and forget the ones that are gone
    /// from the account's playlists (deleted elsewhere). Runs after a full library read.
    pub fn sweep_smart_mixes(&self) {
        let mixes = self.saved_mixes();
        if mixes.is_empty() {
            return;
        }
        let (signed_in, loaded, present) = self.yt_playlist_uris();
        if !signed_in || !loaded {
            return;
        }
        let days = self.settings.get_int(SettingKey::LastFmMixRetentionDays);
        let expired = expired(&mixes, days, unix_now());
        let kept: Vec<SavedMix> =
            mixes.iter().filter(|mix| present.contains(&mix.uri) && !expired.contains(mix)).cloned().collect();
        if kept != mixes {
            self.write_saved_mixes(&kept);
        }
        let doomed: Vec<String> =
            expired.into_iter().filter(|mix| present.contains(&mix.uri)).map(|mix| mix.uri).collect();
        if doomed.is_empty() {
            return;
        }
        tracing::info!("Smart Mix retention: deleting {} playlist(s) older than {days} days", doomed.len());
        self.spawn(move |service| async move {
            let yt = service.yt();
            for uri in &doomed {
                let id = uri.trim_start_matches("youtube:playlist:");
                if let Err(error) = yt.delete_playlist(id).await {
                    tracing::warn!("Smart Mix retention: {uri}: {error:?}");
                }
            }
            service.refresh_youtube();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tolerates_empty_and_garbage() {
        assert!(parse("").is_empty());
        assert!(parse("not json").is_empty());
        let mixes = parse(r#"[{"uri":"youtube:playlist:PL1","created":100}]"#);
        assert_eq!(mixes, vec![SavedMix { uri: "youtube:playlist:PL1".into(), created: 100 }]);
    }

    #[test]
    fn expired_respects_days() {
        let day = 86_400;
        let mixes = vec![
            SavedMix { uri: "a".into(), created: 0 },
            SavedMix { uri: "b".into(), created: 6 * day },
        ];
        assert!(expired(&mixes, 0, 100 * day).is_empty(), "0 keeps everything");
        let gone = expired(&mixes, 7, 7 * day);
        assert_eq!(gone.iter().map(|m| m.uri.as_str()).collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(expired(&mixes, 1, 7 * day).len(), 2);
    }
}
