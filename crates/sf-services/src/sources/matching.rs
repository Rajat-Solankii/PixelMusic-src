//! Does another catalog's track match the one asked for? A port of the Android app's `TitleMatch`
//! (`data/lossless/audiosource/AudioSourceConfig.kt`), the metadata gate every lossless source passes: Jaro-Winkler
//! over normalised titles (and their " - " / " | " segments), artists split on `, ; & / and x`, a hard 15 s
//! duration gate and a weighted score (title 45 %, artist 30 %, duration 20 %, album 5 %) accepted from 0.78.
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Composite score a candidate needs.
pub const ACCEPT_THRESHOLD: f64 = 0.78;
/// Without artist metadata the title alone must be this close.
const TITLE_ONLY_THRESHOLD: f64 = 0.95;
/// With artist metadata the title must be this close (or contain the wanted title).
const MIN_TITLE_WITH_METADATA: f64 = 0.84;
const MIN_ARTIST: f64 = 0.72;
/// A larger duration difference rejects the candidate outright.
const DURATION_HARD_GATE_MS: i64 = 15_000;

/// Words that make a different recording; the two titles must agree on each.
const VERSION_MARKERS: [&str; 14] = [
    "live", "concert", "remix", "rework", "acoustic", "instrumental", "karaoke", "cover", "demo", "sped up",
    "nightcore", "slowed", "extended", "radio edit",
];

static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}]+").unwrap());
static FEATURING_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(feat|ft|featuring)\b.*$").unwrap());
static NOISE_WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(explicit|clean|remaster|remastered|version|audio|official)\b").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static SEGMENT_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[-/|:]\s*|\s*[－／｜：]\s*").unwrap());
static ARTIST_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[,;&/]\s*|\s+(?:and|x)\s+").unwrap());
static TOPIC_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i) - Topic").unwrap());

/// What a catalog says about one of its tracks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidate {
    pub title: String,
    /// the first (or all, joined) artist names
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
}

/// The gate's verdict, with the partial scores for the log.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub accepted: bool,
    pub score: f64,
    pub title: f64,
    pub artist: Option<f64>,
    pub duration: Option<f64>,
    pub reason: &'static str,
}

impl Verdict {
    fn rejected(reason: &'static str, title: f64, artist: Option<f64>, duration: Option<f64>) -> Self {
        Self { accepted: false, score: 0.0, title, artist, duration, reason }
    }
}

/// Lower case, accents dropped, punctuation to spaces, "feat. …" tails and noise words ("official", "remaster")
/// removed.
pub fn normalize(value: &str) -> String {
    let lower: String = value.to_lowercase().nfd().filter(|c| !is_combining_mark(*c)).collect();
    let words = NON_WORD.replace_all(&lower, " ");
    let words = FEATURING_TAIL.replace(&words, "");
    let words = NOISE_WORDS.replace_all(&words, " ");
    SPACES.replace_all(&words, " ").trim().to_owned()
}

/// Jaro-Winkler similarity 0..1 (prefix of up to four characters, weight 0.1).
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let distance = (a.len().max(b.len()) / 2).checked_sub(1);
    let Some(distance) = distance else { return 0.0 };
    let (mut a_matches, mut b_matches) = (vec![false; a.len()], vec![false; b.len()]);
    let mut matches = 0usize;
    for i in 0..a.len() {
        for j in i.saturating_sub(distance)..(i + distance + 1).min(b.len()) {
            if b_matches[j] || a[i] != b[j] {
                continue;
            }
            a_matches[i] = true;
            b_matches[j] = true;
            matches += 1;
            break;
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let mut transpositions = 0usize;
    let mut j = 0;
    for i in 0..a.len() {
        if !a_matches[i] {
            continue;
        }
        while !b_matches[j] {
            j += 1;
        }
        if a[i] != b[j] {
            transpositions += 1;
        }
        j += 1;
    }
    let m = matches as f64;
    let jaro = (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64 / 2.0) / m) / 3.0;
    let prefix = a.iter().zip(&b).take_while(|(left, right)| left == right).count().min(4);
    jaro + prefix as f64 * 0.1 * (1.0 - jaro)
}

/// "Artist - Title | Live" → its parts (none when there is only one).
fn raw_segments(raw: &str) -> Vec<String> {
    let split: Vec<String> =
        SEGMENT_SPLIT.split(raw).map(str::trim).filter(|part| !part.is_empty()).map(str::to_owned).collect();
    if split.len() <= 1 { Vec::new() } else { split }
}

/// Title similarity: the best of the whole titles and their segments against each other.
pub fn ratio(wanted: &str, candidate: &str) -> f64 {
    let (a, b) = (normalize(wanted), normalize(candidate));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let direct = jaro_winkler(&a, &b);
    let segments = |raw: &str| -> Vec<String> {
        raw_segments(raw).iter().map(|segment| normalize(segment)).filter(|segment| segment.chars().count() >= 3).collect()
    };
    let (wanted_segments, candidate_segments) = (segments(wanted), segments(candidate));
    let best = |values: Vec<f64>| values.into_iter().fold(0.0, f64::max);
    let wanted_best = best(wanted_segments.iter().map(|segment| jaro_winkler(segment, &b)).collect());
    let candidate_best = best(candidate_segments.iter().map(|segment| jaro_winkler(&a, segment)).collect());
    let cross = best(
        wanted_segments
            .iter()
            .map(|left| best(candidate_segments.iter().map(|right| jaro_winkler(left, right)).collect()))
            .collect(),
    );
    direct.max(wanted_best).max(candidate_best).max(cross)
}

fn has_version_mismatch(wanted: &str, candidate: &str) -> bool {
    let (target, option) = (format!(" {} ", normalize(wanted)), format!(" {} ", normalize(candidate)));
    VERSION_MARKERS.iter().any(|marker| {
        let token = format!(" {} ", normalize(marker));
        target.contains(&token) != option.contains(&token)
    })
}

fn duration_score(wanted_ms: Option<i64>, candidate_ms: Option<i64>) -> Option<f64> {
    let (wanted, candidate) = (wanted_ms.filter(|&ms| ms > 0)?, candidate_ms.filter(|&ms| ms > 0)?);
    Some(match (wanted - candidate).abs() {
        difference if difference > DURATION_HARD_GATE_MS => 0.0,
        difference if difference <= 3_000 => 1.0,
        difference if difference <= 10_000 => 0.8,
        _ => 0.6,
    })
}

fn artist_parts(value: &str) -> Vec<String> {
    ARTIST_SPLIT.split(value).map(normalize).filter(|part| !part.is_empty()).collect()
}

fn artist_ratio(wanted: &str, candidate: &str) -> f64 {
    let wanted_parts = artist_parts(wanted);
    let candidate_parts = artist_parts(&TOPIC_SUFFIX.replace_all(candidate, ""));
    let mut best = 0.0f64;
    for target in &wanted_parts {
        for option in &candidate_parts {
            let long = target.chars().count() >= 4 && option.chars().count() >= 4;
            let score = if target == option || (long && (option.contains(target.as_str()) || target.contains(option.as_str())))
            {
                1.0
            } else {
                jaro_winkler(target, option)
            };
            best = best.max(score);
        }
    }
    best
}

/// The candidate's title contains the wanted one (or one of its segments).
fn contains_token_run(haystack: &str, needle: &str) -> bool {
    let (target, candidate) = (normalize(needle), normalize(haystack));
    if target.chars().count() < 3 {
        return false;
    }
    if candidate.contains(&target) {
        return true;
    }
    raw_segments(needle).iter().any(|segment| {
        let segment = normalize(segment);
        segment.chars().count() >= 3 && candidate.contains(&segment)
    })
}

/// Is `candidate` the track with this title, these artists, album and duration?
pub fn evaluate(
    wanted_title: &str,
    wanted_artists: &[String],
    wanted_album: Option<&str>,
    wanted_duration_ms: Option<i64>,
    candidate: &Candidate,
) -> Verdict {
    let candidate_title = candidate.title.as_str();
    if candidate_title.trim().is_empty() {
        return Verdict::rejected("provider returned no matched metadata", 0.0, None, None);
    }
    if has_version_mismatch(wanted_title, candidate_title) {
        return Verdict::rejected("version mismatch", ratio(wanted_title, candidate_title), None, None);
    }
    let title = ratio(wanted_title, candidate_title);
    let duration = duration_score(wanted_duration_ms, candidate.duration_ms);
    let wanted_artist = wanted_artists.join(", ");
    let artist = match candidate.artist.as_deref().filter(|name| !name.trim().is_empty()) {
        Some(name) if !wanted_artist.trim().is_empty() => Some(artist_ratio(&wanted_artist, name)),
        _ => None,
    };
    if artist.is_some_and(|score| score < MIN_ARTIST) {
        return Verdict::rejected("artist mismatch", title, artist, duration);
    }
    if duration == Some(0.0) {
        return Verdict::rejected("duration differs by more than 15s", title, artist, duration);
    }
    let Some(artist_score) = artist else {
        let accepted = title >= TITLE_ONLY_THRESHOLD;
        let score = title * 0.8 + duration.unwrap_or(0.5) * 0.2;
        let reason = if accepted { "strict title fallback" } else { "artist metadata unavailable" };
        return Verdict { accepted, score, title, artist: None, duration, reason };
    };
    if title < MIN_TITLE_WITH_METADATA && !contains_token_run(candidate_title, wanted_title) {
        return Verdict::rejected("title mismatch", title, artist, duration);
    }
    let album = match (wanted_album.filter(|album| !album.trim().is_empty()), candidate.album.as_deref()) {
        (Some(wanted), Some(found)) if !found.trim().is_empty() => Some(ratio(wanted, found)),
        _ => None,
    };
    let score = title * 0.45 + artist_score * 0.30 + duration.unwrap_or(0.5) * 0.20 + album.unwrap_or(0.5) * 0.05;
    let accepted = score >= ACCEPT_THRESHOLD;
    Verdict {
        accepted,
        score,
        title,
        artist,
        duration,
        reason: if accepted { "metadata match" } else { "composite score too low" },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(title: &str, artist: Option<&str>, duration_ms: Option<i64>) -> Candidate {
        Candidate { title: title.into(), artist: artist.map(str::to_owned), album: None, duration_ms }
    }

    fn artists(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn normalisation_drops_accents_noise_and_featuring() {
        assert_eq!(normalize("Beyoncé – Halo (Official Audio)"), "beyonce halo");
        assert_eq!(normalize("Señorita feat. Camila Cabello"), "senorita");
        assert_eq!(normalize("  Song   [Remastered 2011] "), "song 2011");
        assert_eq!(normalize("Ночь"), "ночь");
    }

    #[test]
    fn jaro_winkler_matches_the_reference_values() {
        assert!((jaro_winkler("martha", "marhta") - 0.9611).abs() < 1e-3);
        assert!((jaro_winkler("dwayne", "duane") - 0.84).abs() < 1e-3);
        assert!((jaro_winkler("dixon", "dicksonx") - 0.8133).abs() < 1e-3);
        assert_eq!(jaro_winkler("same", "same"), 1.0);
        assert_eq!(jaro_winkler("", "x"), 0.0);
        assert_eq!(jaro_winkler("a", "b"), 0.0);
    }

    #[test]
    fn segments_rescue_artist_prefixed_titles() {
        assert_eq!(ratio("Queen - Bohemian Rhapsody", "Bohemian Rhapsody"), 1.0);
    }

    #[test]
    fn the_same_track_is_accepted() {
        let verdict = evaluate(
            "Bohemian Rhapsody",
            &artists(&["Queen"]),
            Some("A Night at the Opera"),
            Some(354_000),
            &Candidate {
                album: Some("A Night At The Opera (2011 Remaster)".into()),
                ..candidate("Bohemian Rhapsody - Remastered 2011", Some("Queen"), Some(355_000))
            },
        );
        assert!(verdict.accepted, "{verdict:?}");
        assert_eq!(verdict.reason, "metadata match");
        assert_eq!(verdict.duration, Some(1.0));
    }

    #[test]
    fn versions_artists_and_durations_are_gates() {
        let wanted = artists(&["Queen"]);
        let live = evaluate("Bohemian Rhapsody", &wanted, None, None, &candidate("Bohemian Rhapsody (Live)", Some("Queen"), None));
        assert_eq!((live.accepted, live.reason), (false, "version mismatch"));
        let cover = evaluate("Bohemian Rhapsody", &wanted, None, None, &candidate("Bohemian Rhapsody", Some("Panic! At The Disco"), None));
        assert_eq!((cover.accepted, cover.reason), (false, "artist mismatch"));
        let long = evaluate(
            "Bohemian Rhapsody",
            &wanted,
            None,
            Some(354_000),
            &candidate("Bohemian Rhapsody", Some("Queen"), Some(400_000)),
        );
        assert_eq!((long.accepted, long.reason), (false, "duration differs by more than 15s"));
        let other = evaluate("Bohemian Rhapsody", &wanted, None, None, &candidate("Radio Ga Ga", Some("Queen"), None));
        assert_eq!((other.accepted, other.reason), (false, "title mismatch"));
    }

    #[test]
    fn featured_artists_and_topic_channels_match() {
        let verdict = evaluate(
            "Señorita",
            &artists(&["Shawn Mendes", "Camila Cabello"]),
            None,
            Some(191_000),
            &candidate("Senorita", Some("Camila Cabello & Shawn Mendes - Topic"), Some(190_000)),
        );
        assert!(verdict.accepted, "{verdict:?}");
        assert_eq!(verdict.artist, Some(1.0));
    }

    #[test]
    fn without_artists_the_title_must_be_near_exact() {
        let exact = evaluate("Halo", &[], None, None, &candidate("Halo", Some("Beyoncé"), None));
        assert_eq!((exact.accepted, exact.reason), (true, "strict title fallback"));
        let near = evaluate("Halo", &[], None, None, &candidate("Hello", None, None));
        assert_eq!((near.accepted, near.reason), (false, "artist metadata unavailable"));
        let empty = evaluate("Halo", &[], None, None, &candidate(" ", None, None));
        assert!(!empty.accepted);
    }
}
