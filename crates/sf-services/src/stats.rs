//! Listening statistics, ported from soundwave1 (desktop `data/ListeningStats.kt`, `StatsScreen`).
//!
//! The history journal (data::journal) keeps the last 500 plays and no listening time, so the stats keep their own
//! compact log: one [`ListenEvent`] per track left on this computer (Spotify through librespot and the YouTube
//! lane alike — both end with `TrackEnded { local: true }`), with the seconds actually heard. The first time the
//! log is opened it is seeded from the history journal, so plays from before this feature still count.
//!
//! [`summary`] is pure: plays, a range and "today" in, totals and rankings out.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use sf_player::{PlayerEvent, PlayerHandle};
use tokio::runtime::Handle;
use tokio::sync::broadcast;

use crate::data::journal::Journal;
use crate::history::PlayRecord;
use crate::lock;

/// A listen counts from this many seconds (soundwave1 PlayTracker.MIN_SECONDS).
pub const MIN_SECONDS: f64 = 10.0;
/// How many listens the log keeps; the oldest go first.
pub const KEPT: usize = 20_000;
/// Rows in the rankings.
pub const TOP_TRACKS: usize = 10;
pub const TOP_ARTISTS: usize = 6;
pub const TOP_ALBUMS: usize = 6;
/// A pause longer than this between two listens starts a new listening session.
pub const SESSION_GAP_SECONDS: i64 = 15 * 60;

/// One listen: which track, when it started, and how long it was heard.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListenEvent {
    pub uri: String,
    pub title: String,
    /// all credited artists, ", "-separated
    pub artists: String,
    /// the first credited artist's id ("" when unknown, e.g. most YouTube uploads)
    pub artist_id: String,
    pub album: String,
    pub cover: String,
    /// start, Unix seconds
    pub at: i64,
    /// seconds actually heard
    pub seconds: u32,
}

impl ListenEvent {
    pub fn from_track(track: &md3_model::TrackItem, at: i64, seconds: f64) -> Self {
        Self {
            uri: track.play_uri(),
            title: track.title.clone(),
            artists: track.artists.clone(),
            artist_id: track.artist_refs.first().map(|a| a.id.clone()).unwrap_or_default(),
            album: track.album.clone(),
            cover: track.cover.clone(),
            at,
            seconds: seconds.max(0.0).round().min(f64::from(u32::MAX)) as u32,
        }
    }

    /// The source the track played from: "youtube" or "spotify".
    pub fn source(&self) -> &'static str {
        if self.uri.starts_with("youtube:") { "youtube" } else { "spotify" }
    }

    /// The first credited artist, who stands for the track ("A, B" and "A" count together).
    pub fn first_artist(&self) -> &str {
        self.artists.split(',').next().unwrap_or("").trim()
    }
}

/// A history record as a listen; the history keeps no listening time, so the whole track (capped at ten
/// minutes) stands for it.
fn from_record(record: &PlayRecord) -> Option<ListenEvent> {
    let at = record.played_at.parse::<jiff::Timestamp>().ok()?.as_second();
    let seconds = (record.track.duration_ms.max(0) as f64 / 1000.0).min(600.0);
    Some(ListenEvent::from_track(&record.track, at, seconds))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatsRange {
    Today,
    #[default]
    Week,
    Month,
    Year,
    AllTime,
}

impl StatsRange {
    /// "today" | "week" | "month" | "year" | "all" (QML)
    pub fn from_name(name: &str) -> Self {
        match name {
            "today" => Self::Today,
            "month" => Self::Month,
            "year" => Self::Year,
            "all" => Self::AllTime,
            _ => Self::Week,
        }
    }
}

/// A track or an artist with how often and how long it was heard.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranked {
    /// the track's URI, or the artist's id (or name when no id is known)
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub cover: String,
    pub plays: u32,
    pub seconds: u64,
}

/// One bar of the chart: three hours (today), a day (week and month ranges) or a month (year range).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// "2026-11-12" for a day, "2026-11" for a month, "06" for the three hours from 6:00
    pub label: String,
    pub seconds: u64,
    pub plays: u32,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSummary {
    pub total_seconds: u64,
    pub plays: u32,
    /// the total over the days of the range that have passed
    pub average_per_day_seconds: u64,
    pub distinct_tracks: u32,
    pub distinct_artists: u32,
    pub spotify_seconds: u64,
    pub youtube_seconds: u64,
    pub top_tracks: Vec<Ranked>,
    pub top_artists: Vec<Ranked>,
    /// albums by time heard (subtitle: the artist)
    pub top_albums: Vec<Ranked>,
    /// today: eight blocks of three hours; week: Monday..Sunday; month: day 1..today; year: January..December;
    /// all time: none
    pub buckets: Vec<Bucket>,
    /// the hour of the day (0..23) with the most listening
    pub peak_hour: Option<i8>,
    /// the weekday (0 = Monday) with the most listening
    pub busiest_weekday: Option<i8>,
    /// the longest stretch of listening without a pause of [`SESSION_GAP_SECONDS`]
    pub longest_session_seconds: u64,
    /// days of the range with any listening
    pub active_days: u32,
}

/// The stats of `plays` in `range`, with `today` in time zone `tz`.
pub fn summary(plays: &[ListenEvent], range: StatsRange, today: Date, tz: &TimeZone) -> StatsSummary {
    let day_of = |at: i64| jiff::Timestamp::from_second(at).ok().map(|t| t.to_zoned(tz.clone()).date());
    let dated: Vec<(Date, &ListenEvent)> = plays.iter().filter_map(|p| Some((day_of(p.at)?, p))).collect();
    let start = match range {
        StatsRange::Today => today,
        StatsRange::Week => today.checked_sub(jiff::Span::new().days(i64::from(today.weekday().to_monday_zero_offset())))
            .unwrap_or(today),
        StatsRange::Month => today.first_of_month(),
        StatsRange::Year => today.first_of_year(),
        StatsRange::AllTime => dated.iter().map(|(d, _)| *d).min().unwrap_or(today),
    };
    let in_range: Vec<(Date, &ListenEvent)> = dated.into_iter().filter(|(d, _)| *d >= start && *d <= today).collect();
    let total: u64 = in_range.iter().map(|(_, p)| u64::from(p.seconds)).sum();
    let days = today.since(start).map(|s| s.get_days()).unwrap_or(0).max(0) as u64 + 1;

    let mut tracks: HashMap<&str, Ranked> = HashMap::new();
    let mut artists: HashMap<String, (Ranked, Vec<&str>)> = HashMap::new();
    let mut albums: HashMap<String, Ranked> = HashMap::new();
    let zoned = |at: i64| jiff::Timestamp::from_second(at).ok().map(|t| t.to_zoned(tz.clone()));
    let mut hours = [0u64; 24];
    let mut weekdays = [0u64; 7];
    let (mut spotify, mut youtube) = (0u64, 0u64);
    // newest first, so the title and cover shown are the latest ones
    let mut ordered: Vec<&(Date, &ListenEvent)> = in_range.iter().collect();
    ordered.sort_by_key(|(_, p)| std::cmp::Reverse(p.at));
    for (_, play) in ordered {
        let seconds = u64::from(play.seconds);
        if play.source() == "youtube" { youtube += seconds } else { spotify += seconds }
        let track = tracks.entry(play.uri.as_str()).or_insert_with(|| Ranked {
            id: play.uri.clone(),
            title: play.title.clone(),
            subtitle: play.artists.clone(),
            cover: play.cover.clone(),
            ..Default::default()
        });
        track.plays += 1;
        track.seconds += seconds;
        if let Some(when) = zoned(play.at) {
            hours[when.hour().clamp(0, 23) as usize] += seconds;
            weekdays[when.weekday().to_monday_zero_offset().clamp(0, 6) as usize] += seconds;
        }
        let album_name = play.album.trim();
        if !album_name.is_empty() {
            let album = albums.entry(format!("{}\u{1f}{}", album_name.to_lowercase(), play.first_artist().to_lowercase()))
                .or_insert_with(|| Ranked {
                    id: album_name.to_owned(),
                    title: album_name.to_owned(),
                    subtitle: play.first_artist().to_owned(),
                    cover: play.cover.clone(),
                    ..Default::default()
                });
            album.plays += 1;
            album.seconds += seconds;
        }
        let name = play.first_artist();
        if name.is_empty() {
            continue;
        }
        let key = if play.artist_id.is_empty() { name.to_lowercase() } else { play.artist_id.clone() };
        let (artist, songs) = artists.entry(key.clone()).or_insert_with(|| {
            (Ranked { id: key, title: name.to_owned(), cover: play.cover.clone(), ..Default::default() }, Vec::new())
        });
        artist.plays += 1;
        artist.seconds += seconds;
        if !songs.contains(&play.uri.as_str()) {
            songs.push(play.uri.as_str());
        }
    }
    let distinct_tracks = tracks.len() as u32;
    let distinct_artists = artists.len() as u32;
    let mut top_tracks: Vec<Ranked> = tracks.into_values().collect();
    top_tracks.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.seconds.cmp(&a.seconds)).then(a.title.cmp(&b.title)));
    top_tracks.truncate(TOP_TRACKS);
    let mut top_artists: Vec<Ranked> = artists
        .into_values()
        .map(|(mut artist, songs)| {
            artist.subtitle = songs.len().to_string();
            artist
        })
        .collect();
    top_artists.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(b.plays.cmp(&a.plays)).then(a.title.cmp(&b.title)));
    top_artists.truncate(TOP_ARTISTS);
    let mut top_albums: Vec<Ranked> = albums.into_values().collect();
    top_albums.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(b.plays.cmp(&a.plays)).then(a.title.cmp(&b.title)));
    top_albums.truncate(TOP_ALBUMS);

    let busiest = |values: &[u64]| -> Option<i8> {
        let (index, most) = values.iter().enumerate().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))?;
        (*most > 0).then_some(index as i8)
    };
    let peak_hour = busiest(&hours);
    let busiest_weekday = busiest(&weekdays);
    // sessions: listens in time order, a pause longer than SESSION_GAP_SECONDS after one ends starts a new one
    let mut timeline: Vec<&ListenEvent> = in_range.iter().map(|(_, p)| *p).collect();
    timeline.sort_by_key(|p| p.at);
    let (mut longest, mut current, mut ends) = (0u64, 0u64, i64::MIN);
    for play in timeline {
        if ends != i64::MIN && play.at - ends > SESSION_GAP_SECONDS {
            current = 0;
        }
        current += u64::from(play.seconds);
        ends = ends.max(play.at + i64::from(play.seconds));
        longest = longest.max(current);
    }
    let mut active: Vec<Date> = in_range.iter().map(|(d, _)| *d).collect();
    active.sort();
    active.dedup();

    let buckets = match range {
        StatsRange::Today => (0..8i8)
            .map(|block| {
                let in_block: Vec<&ListenEvent> = in_range
                    .iter()
                    .map(|(_, p)| *p)
                    .filter(|p| zoned(p.at).is_some_and(|t| t.hour() / 3 == block))
                    .collect();
                Bucket {
                    label: format!("{:02}", block * 3),
                    seconds: in_block.iter().map(|p| u64::from(p.seconds)).sum(),
                    plays: in_block.len() as u32,
                }
            })
            .collect(),
        StatsRange::Week | StatsRange::Month => {
            let count = if range == StatsRange::Week { 7 } else { days as i64 };
            (0..count)
                .filter_map(|i| start.checked_add(jiff::Span::new().days(i)).ok())
                .map(|day| Bucket {
                    label: day.to_string(),
                    seconds: in_range.iter().filter(|(d, _)| *d == day).map(|(_, p)| u64::from(p.seconds)).sum(),
                    plays: in_range.iter().filter(|(d, _)| *d == day).count() as u32,
                })
                .collect()
        }
        StatsRange::Year => (1..=12i8)
            .map(|month| Bucket {
                label: format!("{:04}-{month:02}", today.year()),
                seconds: in_range.iter().filter(|(d, _)| d.month() == month).map(|(_, p)| u64::from(p.seconds)).sum(),
                plays: in_range.iter().filter(|(d, _)| d.month() == month).count() as u32,
            })
            .collect(),
        StatsRange::AllTime => Vec::new(),
    };

    StatsSummary {
        total_seconds: total,
        plays: in_range.len() as u32,
        average_per_day_seconds: total / days,
        distinct_tracks,
        distinct_artists,
        spotify_seconds: spotify,
        youtube_seconds: youtube,
        top_tracks,
        top_artists,
        top_albums,
        buckets,
        peak_hour,
        busiest_weekday,
        longest_session_seconds: longest,
        active_days: active.len() as u32,
    }
}

/// "5 h 22 m", "44 m", "38 s", "0 m" (soundwave1 ListeningStats.formatTime); the units go through mf_i18n.
pub fn format_time(seconds: u64) -> String {
    let (h, m) = (seconds / 3600, (seconds % 3600) / 60);
    if h > 0 {
        format!("{h} {} {m} {}", mf_i18n::t("h"), mf_i18n::t("m"))
    } else if m > 0 || seconds == 0 {
        format!("{m} {}", mf_i18n::t("m"))
    } else {
        format!("{seconds} {}", mf_i18n::t("s"))
    }
}

/// The listens on disk, newest last.
#[derive(Debug, Default)]
pub struct StatsLog {
    plays: Vec<ListenEvent>,
    dirty: bool,
}

impl StatsLog {
    /// The log at `path`; when there is none yet, the plays of the history journal at `history`.
    pub fn load(path: &Path, history: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self { plays: serde_json::from_str(&text).unwrap_or_default(), dirty: false },
            Err(_) => {
                let journal = Journal::load(history);
                let mut plays: Vec<ListenEvent> = journal.plays().iter().filter_map(from_record).collect();
                plays.sort_by_key(|p| p.at);
                let dirty = !plays.is_empty();
                Self { plays, dirty }
            }
        }
    }

    pub fn plays(&self) -> &[ListenEvent] {
        &self.plays
    }

    pub fn record(&mut self, play: ListenEvent) {
        self.plays.push(play);
        if self.plays.len() > KEPT {
            let excess = self.plays.len() - KEPT;
            self.plays.drain(..excess);
        }
        self.dirty = true;
    }

    /// Replace everything (a restored backup).
    pub fn replace(&mut self, mut plays: Vec<ListenEvent>) {
        plays.sort_by_key(|p| p.at);
        let excess = plays.len().saturating_sub(KEPT);
        plays.drain(..excess);
        self.plays = plays;
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        if !self.plays.is_empty() {
            self.plays.clear();
            self.dirty = true;
        }
    }

    pub fn save(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        match serde_json::to_vec(&self.plays) {
            Ok(bytes) => match crate::data::fsio::write_atomic(path, &bytes) {
                Ok(()) => self.dirty = false,
                Err(error) => tracing::warn!("could not write listening stats: {error:#}"),
            },
            Err(error) => tracing::warn!("could not write listening stats: {error:#}"),
        }
    }
}

/// The stats file: app_data_dir()/stats.json.
pub fn stats_file() -> PathBuf {
    mf_platform::dirs::app_data_dir().join("stats.json")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatsEvent {
    Changed,
}

/// Records listens from the player and answers [`summary`] for the stats page.
pub struct StatsService {
    rt: Handle,
    path: PathBuf,
    log: Mutex<StatsLog>,
    events: broadcast::Sender<StatsEvent>,
    me: Weak<StatsService>,
}

impl StatsService {
    pub fn new(rt: Handle, player: &PlayerHandle) -> Arc<Self> {
        Self::with_files(rt, player, stats_file(), mf_platform::dirs::history_file())
    }

    /// The same with other files (tests).
    pub fn with_files(rt: Handle, player: &PlayerHandle, path: PathBuf, history: PathBuf) -> Arc<Self> {
        let log = StatsLog::load(&path, &history);
        let service = Arc::new_cyclic(|me| Self {
            rt,
            path,
            log: Mutex::new(log),
            events: broadcast::channel(16).0,
            me: me.clone(),
        });
        service.save_later();
        let mut events = player.subscribe().events;
        let me = service.me.clone();
        service.rt.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(PlayerEvent::TrackEnded(ended)) if ended.local => {
                        let Some(service) = me.upgrade() else { return };
                        service.note_played(&ended.track, ended.played);
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        service
    }

    /// A track was left after `played` seconds on this computer.
    pub fn note_played(&self, track: &md3_model::TrackItem, played: f64) {
        if played < MIN_SECONDS || (track.uri.is_empty() && track.track_id.is_empty()) {
            return;
        }
        let at = jiff::Timestamp::now().as_second() - played.max(0.0) as i64;
        lock(&self.log).record(ListenEvent::from_track(track, at, played));
        self.save_later();
        let _ = self.events.send(StatsEvent::Changed);
    }

    pub fn summary(&self, range: StatsRange) -> StatsSummary {
        let tz = TimeZone::system();
        let today = jiff::Timestamp::now().to_zoned(tz.clone()).date();
        summary(lock(&self.log).plays(), range, today, &tz)
    }

    /// Every listen, oldest first (backup).
    pub fn plays(&self) -> Vec<ListenEvent> {
        lock(&self.log).plays().to_vec()
    }

    /// Replace the log (restored backup).
    pub fn replace(&self, plays: Vec<ListenEvent>) {
        lock(&self.log).replace(plays);
        self.save_later();
        let _ = self.events.send(StatsEvent::Changed);
    }

    pub fn clear(&self) {
        lock(&self.log).clear();
        self.save_later();
        let _ = self.events.send(StatsEvent::Changed);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StatsEvent> {
        self.events.subscribe()
    }

    pub fn flush(&self) {
        lock(&self.log).save(&self.path);
    }

    fn save_later(&self) {
        let me = self.me.clone();
        self.rt.spawn_blocking(move || {
            if let Some(service) = me.upgrade() {
                service.flush();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(date: &str, hour: i8) -> i64 {
        let date: Date = date.parse().unwrap();
        date.at(hour, 0, 0, 0).to_zoned(TimeZone::UTC).unwrap().timestamp().as_second()
    }

    fn play(uri: &str, artists: &str, artist_id: &str, when: i64, seconds: u32) -> ListenEvent {
        ListenEvent {
            uri: uri.into(),
            title: uri.rsplit(':').next().unwrap().into(),
            artists: artists.into(),
            artist_id: artist_id.into(),
            at: when,
            seconds,
            ..Default::default()
        }
    }

    fn sample() -> Vec<ListenEvent> {
        vec![
            play("spotify:track:a", "Alice, Bob", "alice", at("2026-10-30", 12), 200), // last month
            play("spotify:track:a", "Alice", "alice", at("2026-11-09", 12), 180),       // Monday
            play("spotify:track:a", "Alice", "alice", at("2026-11-10", 23), 180),
            play("youtube:track:y1", "Yuki", "", at("2026-11-11", 8), 300),
            play("youtube:track:y1", "yuki", "", at("2026-11-12", 8), 240),
            play("spotify:track:b", "Bob", "bob", at("2026-11-12", 9), 60),
            play("spotify:track:c", "", "", at("2026-11-12", 10), 30),
        ]
    }

    #[test]
    fn week_counts_youtube_and_spotify_from_monday() {
        let today: Date = "2026-11-12".parse().unwrap(); // a Thursday
        let s = summary(&sample(), StatsRange::Week, today, &TimeZone::UTC);
        assert_eq!(s.plays, 6);
        assert_eq!(s.total_seconds, 180 + 180 + 300 + 240 + 60 + 30);
        assert_eq!(s.youtube_seconds, 540);
        assert_eq!(s.spotify_seconds, 450);
        assert_eq!(s.average_per_day_seconds, 990 / 4);
        assert_eq!(s.top_tracks[0].id, "youtube:track:y1", "two plays each; the longer heard comes first");
        assert_eq!(s.top_tracks[1].id, "spotify:track:a");
        assert_eq!((s.top_tracks[1].plays, s.top_tracks[1].seconds), (2, 360));
        assert_eq!(s.top_tracks[2].id, "spotify:track:b", "one play ranks below two");
        // artists rank by time; YouTube artists without ids merge by name, case aside
        assert_eq!(s.top_artists[0].title, "yuki", "the newest play's spelling is shown");
        assert_eq!(s.top_artists[0].seconds, 540);
        assert_eq!(s.top_artists[1].id, "alice");
        assert_eq!(s.distinct_artists, 3, "the track without artists is not an artist");
        assert_eq!(s.buckets.len(), 7);
        assert_eq!(s.buckets[0], Bucket { label: "2026-11-09".into(), seconds: 180, plays: 1 });
        assert_eq!(s.buckets[3].plays, 3);
        assert_eq!(s.buckets[3].seconds, 330);
        assert_eq!(s.buckets[6].seconds, 0);
    }

    #[test]
    fn month_year_and_all_time() {
        let today: Date = "2026-11-12".parse().unwrap();
        let month = summary(&sample(), StatsRange::Month, today, &TimeZone::UTC);
        assert_eq!(month.plays, 6);
        assert_eq!(month.buckets.len(), 12);
        let year = summary(&sample(), StatsRange::Year, today, &TimeZone::UTC);
        assert_eq!(year.plays, 7);
        assert_eq!(year.buckets[9], Bucket { label: "2026-10".into(), seconds: 200, plays: 1 });
        let all = summary(&sample(), StatsRange::AllTime, today, &TimeZone::UTC);
        assert_eq!(all.plays, 7);
        assert_eq!(all.top_tracks[0].plays, 3);
        assert!(all.buckets.is_empty());
        let empty = summary(&[], StatsRange::AllTime, today, &TimeZone::UTC);
        assert_eq!(empty, StatsSummary::default());
    }

    #[test]
    fn today_habits_albums_and_sessions() {
        let today: Date = "2026-11-12".parse().unwrap();
        let mut plays = sample();
        for p in &mut plays {
            p.album = if p.uri.ends_with(":y1") { "Snow".into() } else { String::new() };
        }
        let s = summary(&plays, StatsRange::Today, today, &TimeZone::UTC);
        assert_eq!(s.plays, 3);
        assert_eq!(s.buckets.len(), 8);
        assert_eq!(s.buckets[2], Bucket { label: "06".into(), seconds: 240, plays: 1 }, "8:00 is in 6–9");
        assert_eq!(s.buckets[3], Bucket { label: "09".into(), seconds: 90, plays: 2 });
        assert_eq!(s.peak_hour, Some(8));
        assert_eq!(s.top_albums.len(), 1);
        assert_eq!((s.top_albums[0].title.as_str(), s.top_albums[0].subtitle.as_str()), ("Snow", "yuki"));
        // 8:00 for 240 s, then 9:00 (56 min later: a new session) for 60 s and 10:00 for 30 s
        assert_eq!(s.longest_session_seconds, 240);
        assert_eq!(s.active_days, 1);
        let week = summary(&plays, StatsRange::Week, today, &TimeZone::UTC);
        assert_eq!(week.busiest_weekday, Some(3), "Thursday: 330 s");
        assert_eq!(week.active_days, 4);
        let none = summary(&[], StatsRange::Today, today, &TimeZone::UTC);
        assert_eq!((none.peak_hour, none.busiest_weekday, none.longest_session_seconds), (None, None, 0));
    }

    #[test]
    fn days_follow_the_time_zone() {
        // 23:00 UTC on Tuesday is already Wednesday east of UTC
        let tz = TimeZone::fixed(jiff::tz::offset(3));
        let today: Date = "2026-11-12".parse().unwrap();
        let s = summary(&sample(), StatsRange::Week, today, &tz);
        assert_eq!(s.buckets[1].seconds, 0);
        assert_eq!(s.buckets[2].seconds, 180 + 300);
    }

    #[test]
    fn the_log_is_seeded_from_history_and_capped() {
        let dir = std::env::temp_dir().join(format!("mf-stats-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let history = dir.join("history.json");
        let track = md3_model::TrackItem {
            uri: "youtube:track:abc".into(),
            title: "Song".into(),
            artists: "Yuki".into(),
            duration_ms: 1_200_000,
            ..Default::default()
        };
        let records = vec![PlayRecord { track, played_at: "2026-11-12T08:00:00Z".into(), ..Default::default() }];
        std::fs::write(&history, serde_json::to_vec(&records).unwrap()).unwrap();
        let path = dir.join("stats.json");
        let mut log = StatsLog::load(&path, &history);
        assert_eq!(log.plays().len(), 1);
        assert_eq!(log.plays()[0].seconds, 600, "a history record counts as the track, capped at ten minutes");
        assert_eq!(log.plays()[0].source(), "youtube");
        log.save(&path);
        // once the log exists, the history no longer seeds it
        std::fs::write(&history, b"[]").unwrap();
        let mut log = StatsLog::load(&path, &history);
        assert_eq!(log.plays().len(), 1);
        for i in 0..KEPT {
            log.record(play("spotify:track:x", "X", "", i as i64, 20));
        }
        assert_eq!(log.plays().len(), KEPT);
        assert_eq!(log.plays()[0].uri, "spotify:track:x", "the oldest went first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn time_format() {
        assert!(format_time(0).starts_with("0 "));
        assert!(format_time(38).starts_with("38 "));
        assert!(format_time(44 * 60 + 5).starts_with("44 "));
        assert!(format_time(5 * 3600 + 22 * 60).starts_with("5 "));
    }
}
