//! Home mixes from the local listening history, as on the SoundWave Android home (`DailyMixManager.kt`):
//! "Recently played" (the last songs, each once), "Daily Mix" (30 songs "based on your history") and
//! "Your Mix" (60 songs: liked ones first, then favourites, then discoveries — here also songs of the
//! YouTube Music home feed that were never played). Both mixes are seeded by the day, so they stay the same
//! until tomorrow. Everything here is pure: `HomeService::publish_mixes` feeds it the history.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use md3_model::{HomeShelf, TrackItem};

use crate::history::PlayRecord;

pub const RECENT_LIMIT: usize = 30;
pub const DAILY_LIMIT: usize = 30;
pub const YOUR_LIMIT: usize = 60;
/// Your Mix is seeded apart from Daily Mix (the same offset as on Android).
const YOUR_SEED_OFFSET: u64 = 17;
const DAY: i64 = 86_400;

/// The three home sections; all empty until something was played.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HomeMixes {
    pub recent: Arc<Vec<TrackItem>>,
    pub daily: Arc<Vec<TrackItem>>,
    pub your: Arc<Vec<TrackItem>>,
}

/// What the mixes were built from: they are rebuilt on another day, or while they are short and there is
/// more to pick from (a song ending must not reshuffle the cards on Home).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MixKey {
    pub day: u64,
    pub pool: usize,
}

/// Year × 1000 + day of the year, local time: the seed of today's mixes.
pub fn day_seed(now: &jiff::Zoned) -> u64 {
    now.year().max(0) as u64 * 1000 + now.day_of_year() as u64
}

fn playable(track: &TrackItem) -> bool {
    !track.is_episode && !track.uri.is_empty()
}

/// The last played songs, newest first, each once.
pub fn recently_played(plays: &[PlayRecord], limit: usize) -> Vec<TrackItem> {
    let mut seen = HashSet::new();
    plays
        .iter()
        .map(|play| &play.track)
        .filter(|track| playable(track) && seen.insert(track.uri.as_str()))
        .take(limit)
        .cloned()
        .collect()
}

/// Songs of the "tracks" shelves of a home feed (YouTube Music quick picks and the like).
pub fn feed_tracks(shelves: &[HomeShelf]) -> Vec<TrackItem> {
    let mut seen = HashSet::new();
    shelves
        .iter()
        .filter(|shelf| shelf.kind == "tracks")
        .flat_map(|shelf| shelf.items.iter())
        .filter_map(|item| serde_json::from_value::<TrackItem>(item.clone()).ok())
        .filter(|track| playable(track) && seen.insert(track.uri.clone()))
        .collect()
}

struct Candidate {
    track: TrackItem,
    artist: String,
    plays: usize,
    /// seconds; `None` — never played here (a feed song) or the play has no time
    last_played: Option<i64>,
    liked: bool,
    score: f64,
    discovery: f64,
}

fn artist_key(track: &TrackItem) -> String {
    match track.artist_refs.first() {
        Some(artist) if !artist.id.is_empty() => artist.id.clone(),
        _ => track.artists.split(',').next().unwrap_or_default().trim().to_lowercase(),
    }
}

/// 0..1, the same for a song and a seed (the rotation of the day; no state, so a longer history does not
/// reorder the rest).
fn jitter(uri: &str, seed: u64) -> f64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for byte in uri.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    // splitmix64 finaliser
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 31;
    (hash >> 11) as f64 / (1u64 << 53) as f64
}

/// Played yesterday scores lower than played two weeks ago: the mix brings songs back.
fn recency(last_played: Option<i64>, now: i64) -> f64 {
    let Some(at) = last_played else { return 0.6 };
    match (now - at).max(0) / DAY {
        0 => 0.2,
        1..=2 => 0.5,
        3..=6 => 0.7,
        7..=13 => 0.85,
        _ => 1.0,
    }
}

/// Every song of the history (newest first) and then the never played `extra` ones, scored for `seed`.
fn candidates(plays: &[PlayRecord], extra: &[TrackItem], liked: &dyn Fn(&TrackItem) -> bool, seed: u64, now: i64) -> Vec<Candidate> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut pool: Vec<Candidate> = Vec::new();
    let mut add = |track: &TrackItem, played: Option<Option<i64>>, pool: &mut Vec<Candidate>| {
        if !playable(track) {
            return;
        }
        match index.get(track.uri.as_str()) {
            Some(&at) => pool[at].plays += usize::from(played.is_some()),
            None => {
                index.insert(track.uri.clone(), pool.len());
                pool.push(Candidate {
                    track: track.clone(),
                    artist: artist_key(track),
                    plays: usize::from(played.is_some()),
                    last_played: played.flatten(),
                    liked: liked(track),
                    score: 0.0,
                    discovery: 0.0,
                });
            }
        }
    };
    for play in plays {
        let at = play.played_at.parse::<jiff::Timestamp>().ok().map(|at| at.as_second());
        add(&play.track, Some(at), &mut pool);
    }
    for track in extra {
        add(track, None, &mut pool);
    }

    let mut by_artist: HashMap<&str, usize> = HashMap::new();
    for candidate in &pool {
        *by_artist.entry(candidate.artist.as_str()).or_default() += candidate.plays;
    }
    let most_plays = pool.iter().map(|c| c.plays).max().unwrap_or(0).max(1) as f64;
    let most_by_artist = by_artist.values().copied().max().unwrap_or(0).max(1) as f64;
    let scores: Vec<(f64, f64)> = pool
        .iter()
        .map(|c| {
            let affinity = c.plays as f64 / most_plays;
            let preference = by_artist[c.artist.as_str()] as f64 / most_by_artist;
            let novelty = if c.plays == 0 { 1.0 } else { 0.0 };
            let score = preference * 0.40
                + affinity * 0.20
                + recency(c.last_played, now) * 0.15
                + if c.liked { 0.10 } else { 0.0 }
                + jitter(&c.track.uri, seed) * 0.30;
            let discovery =
                (1.0 - affinity) * 0.5 + novelty * 0.3 + preference * 0.2 + jitter(&c.track.uri, seed.wrapping_mul(31)) * 0.15;
            (score, discovery)
        })
        .collect();
    for (candidate, (score, discovery)) in pool.iter_mut().zip(scores) {
        candidate.score = score;
        candidate.discovery = discovery;
    }
    pool
}

/// Indexes of `pool` sorted by `key`, best first (ties: by URI, so the order never depends on the input).
fn ranked(pool: &[Candidate], key: impl Fn(&Candidate) -> f64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..pool.len()).collect();
    order.sort_by(|&a, &b| key(&pool[b]).total_cmp(&key(&pool[a])).then_with(|| pool[a].track.uri.cmp(&pool[b].track.uri)));
    order
}

/// Up to `limit` of `order` not yet `taken`: one song per artist (two for a liked song), and only when
/// that leaves the section short — the rest in order.
fn pick(pool: &[Candidate], order: &[usize], limit: usize, taken: &mut HashSet<usize>, out: &mut Vec<usize>) {
    let mut picked: Vec<usize> = Vec::new();
    let mut per_artist: HashMap<&str, usize> = HashMap::new();
    for &at in order {
        if picked.len() >= limit {
            break;
        }
        if taken.contains(&at) {
            continue;
        }
        let count = per_artist.entry(pool[at].artist.as_str()).or_default();
        if *count >= if pool[at].liked { 2 } else { 1 } {
            continue;
        }
        *count += 1;
        picked.push(at);
    }
    for &at in order {
        if picked.len() >= limit {
            break;
        }
        if !taken.contains(&at) && !picked.contains(&at) {
            picked.push(at);
        }
    }
    taken.extend(picked.iter().copied());
    out.extend(picked);
}

fn tracks(pool: Vec<Candidate>, order: Vec<usize>, limit: usize) -> Vec<TrackItem> {
    let mut pool: Vec<Option<TrackItem>> = pool.into_iter().map(|c| Some(c.track)).collect();
    order.into_iter().take(limit).filter_map(|at| pool[at].take()).collect()
}

/// "Daily Mix", based on the history alone: 45 % the day's favourites, 35 % songs played less, the rest in
/// the day's random order.
pub fn daily_mix(plays: &[PlayRecord], liked: &dyn Fn(&TrackItem) -> bool, seed: u64, now: i64, limit: usize) -> Vec<TrackItem> {
    let pool = candidates(plays, &[], liked, seed, now);
    let core = (limit * 45 / 100).max(3).min(limit);
    let discovery = (limit * 35 / 100).max(2).min(limit - core);
    let (mut taken, mut order) = (HashSet::new(), Vec::new());
    pick(&pool, &ranked(&pool, |c| c.score), core, &mut taken, &mut order);
    pick(&pool, &ranked(&pool, |c| c.discovery), discovery, &mut taken, &mut order);
    let variety = ranked(&pool, |c| jitter(&c.track.uri, seed ^ 0x5eed));
    pick(&pool, &variety, limit.saturating_sub(order.len()), &mut taken, &mut order);
    tracks(pool, order, limit)
}

/// "Your Mix": a larger blend — 30 % liked songs, 45 % the favourites of the history, the rest discoveries,
/// where the never played `feed` songs (the YouTube Music home) come first. Empty without a history.
pub fn your_mix(
    plays: &[PlayRecord],
    feed: &[TrackItem],
    liked: &dyn Fn(&TrackItem) -> bool,
    seed: u64,
    now: i64,
    limit: usize,
) -> Vec<TrackItem> {
    if !plays.iter().any(|play| playable(&play.track)) {
        return Vec::new();
    }
    let seed = seed + YOUR_SEED_OFFSET;
    let pool = candidates(plays, feed, liked, seed, now);
    let favourites = (limit * 30 / 100).max(5).min(limit);
    let core = (limit * 45 / 100).max(10).min(limit - favourites);
    let by_score = ranked(&pool, |c| c.score);
    let (mut taken, mut order) = (HashSet::new(), Vec::new());
    let liked_first: Vec<usize> = by_score.iter().copied().filter(|&at| pool[at].liked && pool[at].plays > 0).collect();
    pick(&pool, &liked_first, favourites, &mut taken, &mut order);
    let played: Vec<usize> = by_score.iter().copied().filter(|&at| pool[at].plays > 0).collect();
    pick(&pool, &played, core, &mut taken, &mut order);
    pick(&pool, &ranked(&pool, |c| c.discovery), limit.saturating_sub(order.len()), &mut taken, &mut order);
    tracks(pool, order, limit)
}

/// How many different songs the mixes can pick from (see [`MixKey`]).
pub(super) fn pool_size(plays: &[PlayRecord], feed: &[TrackItem]) -> usize {
    let mut seen: HashSet<&str> = HashSet::new();
    plays.iter().map(|play| &play.track).chain(feed).filter(|track| playable(track)).filter(|track| seen.insert(track.uri.as_str())).count()
}

/// The sections for `plays` (newest first). `previous` and its `key` are what is shown now: the mixes are
/// kept unless the day changed, or they are short and the pool has changed.
pub(super) fn build(
    plays: &[PlayRecord],
    feed: &[TrackItem],
    liked: &dyn Fn(&TrackItem) -> bool,
    now: &jiff::Zoned,
    previous: &HomeMixes,
    key: MixKey,
) -> (HomeMixes, MixKey) {
    let recent = recently_played(plays, RECENT_LIMIT);
    if recent.is_empty() {
        return (HomeMixes::default(), MixKey::default());
    }
    let next = MixKey { day: day_seed(now), pool: pool_size(plays, feed) };
    let full = previous.daily.len() >= DAILY_LIMIT && previous.your.len() >= YOUR_LIMIT;
    let keep = !previous.daily.is_empty() && key.day == next.day && (full || key.pool == next.pool);
    let recent = if *previous.recent == recent { previous.recent.clone() } else { Arc::new(recent) };
    if keep {
        return (HomeMixes { recent, daily: previous.daily.clone(), your: previous.your.clone() }, MixKey { day: key.day, pool: next.pool });
    }
    let at = now.timestamp().as_second();
    let daily = daily_mix(plays, liked, next.day, at, DAILY_LIMIT);
    let your = your_mix(plays, feed, liked, next.day, at, YOUR_LIMIT);
    (HomeMixes { recent, daily: Arc::new(daily), your: Arc::new(your) }, next)
}

#[cfg(test)]
mod tests {
    use md3_model::ArtistRef;

    use super::*;

    const NOW: i64 = 1_790_000_000;

    fn track(id: &str, artist: &str) -> TrackItem {
        TrackItem {
            track_id: id.to_owned(),
            uri: format!("youtube:track:{id}"),
            title: format!("Song {id}"),
            artists: artist.to_owned(),
            artist_refs: vec![ArtistRef { id: format!("UC{artist}"), name: artist.to_owned() }],
            ..Default::default()
        }
    }

    /// A play `days` days before NOW.
    fn play(id: &str, artist: &str, days: i64) -> PlayRecord {
        let at = jiff::Timestamp::from_second(NOW - days * DAY - 60).unwrap();
        PlayRecord { track: track(id, artist), played_at: at.to_string(), ..Default::default() }
    }

    fn ids(tracks: &[TrackItem]) -> Vec<&str> {
        tracks.iter().map(|t| t.track_id.as_str()).collect()
    }

    fn nobody(_: &TrackItem) -> bool {
        false
    }

    /// 40 songs of 20 artists; song i was played (i % 4 + 1) times.
    fn history() -> Vec<PlayRecord> {
        let mut plays = Vec::new();
        for round in 0..4 {
            for i in 0..40 {
                if round <= i % 4 {
                    plays.push(play(&format!("s{i}"), &format!("a{}", i % 20), round as i64 * 3));
                }
            }
        }
        plays
    }

    fn today() -> jiff::Zoned {
        jiff::Timestamp::from_second(NOW).unwrap().to_zoned(jiff::tz::TimeZone::UTC)
    }

    #[test]
    fn recently_played_lists_each_song_once_newest_first() {
        let episode = PlayRecord { track: TrackItem { uri: "spotify:episode:e".into(), is_episode: true, ..Default::default() }, ..Default::default() };
        let plays = vec![play("b", "x", 0), episode, play("a", "x", 0), play("b", "x", 1), play("c", "y", 2)];
        assert_eq!(ids(&recently_played(&plays, 10)), ["b", "a", "c"]);
        assert_eq!(ids(&recently_played(&plays, 2)), ["b", "a"]);
        assert!(recently_played(&[], 10).is_empty());
    }

    #[test]
    fn daily_mix_is_the_same_all_day_and_differs_the_next() {
        let plays = history();
        let monday = daily_mix(&plays, &nobody, 2_026_316, NOW, 30);
        assert_eq!(monday.len(), 30);
        assert_eq!(ids(&monday), ids(&daily_mix(&plays, &nobody, 2_026_316, NOW, 30)));
        assert_ne!(ids(&monday), ids(&daily_mix(&plays, &nobody, 2_026_317, NOW, 30)));
        // every song once
        let unique: HashSet<&str> = ids(&monday).into_iter().collect();
        assert_eq!(unique.len(), 30);
    }

    #[test]
    fn daily_mix_starts_with_one_song_per_artist() {
        let mix = daily_mix(&history(), &nobody, 2_026_316, NOW, 30);
        let core: HashSet<&str> = mix[..13].iter().map(|t| t.artists.as_str()).collect();
        assert_eq!(core.len(), 13);
    }

    #[test]
    fn a_short_history_gives_a_short_daily_mix() {
        let plays = vec![play("a", "x", 0), play("b", "x", 0), play("a", "x", 1)];
        let mix = daily_mix(&plays, &nobody, 1, NOW, 30);
        let mut got = ids(&mix);
        got.sort();
        assert_eq!(got, ["a", "b"]);
        assert!(daily_mix(&[], &nobody, 1, NOW, 30).is_empty());
    }

    #[test]
    fn your_mix_opens_with_liked_songs_and_adds_feed_songs() {
        let plays = history();
        let feed: Vec<TrackItem> = (0..30).map(|i| track(&format!("f{i}"), &format!("fa{i}"))).collect();
        let liked = |t: &TrackItem| ["s1", "s2", "s3"].contains(&t.track_id.as_str());
        let mix = your_mix(&plays, &feed, &liked, 2_026_316, NOW, 60);
        assert_eq!(mix.len(), 60);
        let mut first = ids(&mix[..3]);
        first.sort();
        assert_eq!(first, ["s1", "s2", "s3"]);
        let from_feed = mix.iter().filter(|t| t.track_id.starts_with('f')).count();
        assert!(from_feed >= 15, "{from_feed} feed songs");
        // the played part comes before the discoveries
        assert!(mix[3..30].iter().all(|t| t.track_id.starts_with('s')));
        // not the Daily Mix in another order
        assert_ne!(ids(&mix[..30]), ids(&daily_mix(&plays, &liked, 2_026_316, NOW, 30)));
    }

    #[test]
    fn your_mix_needs_a_history() {
        let feed = vec![track("f", "x")];
        assert!(your_mix(&[], &feed, &nobody, 1, NOW, 60).is_empty());
    }

    #[test]
    fn feed_tracks_come_from_track_shelves_only() {
        let value = |t: TrackItem| serde_json::to_value(t).unwrap();
        let shelves = vec![
            HomeShelf { key: "a".into(), title: "Quick picks".into(), kind: "tracks".into(), items: vec![value(track("1", "x")), value(track("2", "y"))] },
            HomeShelf { key: "b".into(), title: "Albums".into(), kind: "tiles".into(), items: vec![serde_json::json!({ "kind": "album", "uri": "youtube:album:z" })] },
            HomeShelf { key: "c".into(), title: "More".into(), kind: "tracks".into(), items: vec![value(track("2", "y")), value(track("3", "z"))] },
        ];
        assert_eq!(ids(&feed_tracks(&shelves)), ["1", "2", "3"]);
    }

    #[test]
    fn mixes_are_kept_until_the_day_changes() {
        let mut plays = history();
        let (first, key) = build(&plays, &[], &nobody, &today(), &HomeMixes::default(), MixKey::default());
        assert_eq!((first.recent.len(), first.daily.len(), first.your.len()), (30, 30, 40));
        // Your Mix is short (40 songs known): a new song rebuilds the mixes
        plays.insert(0, play("new", "z", 0));
        let (second, key2) = build(&plays, &[], &nobody, &today(), &first, key);
        assert_eq!(second.recent[0].track_id, "new");
        assert_eq!(second.your.len(), 41);
        // a replay changes "Recently played" only
        plays.insert(0, play("s5", "a5", 0));
        let (third, key3) = build(&plays, &[], &nobody, &today(), &second, key2);
        assert_eq!(third.recent[0].track_id, "s5");
        assert!(Arc::ptr_eq(&third.daily, &second.daily) && Arc::ptr_eq(&third.your, &second.your));
        // tomorrow: new mixes
        let tomorrow = today().checked_add(jiff::Span::new().days(1)).unwrap();
        let (fourth, key4) = build(&plays, &[], &nobody, &tomorrow, &third, key3);
        assert_eq!(key4.day, key3.day + 1);
        assert_ne!(ids(&fourth.daily), ids(&third.daily));
    }

    #[test]
    fn nothing_played_means_no_sections() {
        let feed = vec![track("f", "x")];
        let (mixes, key) = build(&[], &feed, &nobody, &today(), &HomeMixes::default(), MixKey::default());
        assert_eq!(mixes, HomeMixes::default());
        assert_eq!(key, MixKey::default());
    }
}
