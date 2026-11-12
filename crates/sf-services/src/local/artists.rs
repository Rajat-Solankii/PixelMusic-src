//! Artist names from a tag: "A feat. B / C" is three artists. Ported from soundwave1's
//! `splitArtistsByDelimiters` / `extractArtistsFromTitle` (Android `Extensions.kt`): character delimiters split
//! anywhere, word delimiters ("feat.", "vs") only between spaces, and a backslash keeps a delimiter ("AC\/DC").
use std::sync::LazyLock;

use regex::Regex;

/// soundwave1's `DEFAULT_ARTIST_DELIMITERS`
pub const CHAR_DELIMITERS: &[&str] = &["/", ";", ",", "+", "&"];
/// soundwave1's `DEFAULT_ARTIST_WORD_DELIMITERS`
pub const WORD_DELIMITERS: &[&str] =
    &["featuring", "feat.", "feat", "ft.", "ft", "vs.", "vs", "versus", "with", "prod.", "prod"];

const ESCAPE: char = '\\';
/// stands for an escaped delimiter while the rest is split (a private-use character no tag holds)
const PLACEHOLDER: char = '\u{E000}';

static SPLITTER: LazyLock<Regex> = LazyLock::new(|| splitter(CHAR_DELIMITERS, WORD_DELIMITERS));

/// "(feat. B)" / "[ft. B & C]" at the end of a title
static TITLE_FEAT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*[\(\[]\s*(?:featuring|feat\.?|ft\.?|with)\s+([^\)\]]+)[\)\]]").expect("title regex")
});

fn splitter(chars: &[&str], words: &[&str]) -> Regex {
    let mut words: Vec<&str> = words.to_vec();
    words.sort_by_key(|word| std::cmp::Reverse(word.len()));
    let mut parts: Vec<String> = Vec::new();
    for word in words {
        let escaped = regex::escape(word);
        if word.chars().count() == 1 {
            parts.push(format!(r"\s+{escaped}\s+"));
        } else {
            parts.push(format!(r"\s+{escaped}\s+|\s+{escaped}$|^{escaped}\s+"));
        }
    }
    let mut chars: Vec<&str> = chars.to_vec();
    chars.sort_by_key(|delimiter| std::cmp::Reverse(delimiter.len()));
    for delimiter in chars {
        parts.push(format!(r"\s*{}\s*", regex::escape(delimiter)));
    }
    Regex::new(&format!("(?i){}", parts.join("|"))).expect("artist delimiters")
}

/// The artists of a tag value, in order, without duplicates (case-insensitive).
pub fn split_artists(raw: &str) -> Vec<String> {
    split_with(&SPLITTER, raw)
}

fn split_with(splitter: &Regex, raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    // "\/" keeps the slash: hide escaped delimiters from the splitter
    let mut escaped: std::collections::VecDeque<char> = std::collections::VecDeque::new();
    let mut working = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ESCAPE
            && let Some(&next) = chars.peek()
            && CHAR_DELIMITERS.iter().any(|delimiter| delimiter.starts_with(next))
        {
            escaped.push_back(next);
            working.push(PLACEHOLDER);
            chars.next();
            continue;
        }
        working.push(c);
    }
    let mut out: Vec<String> = Vec::new();
    for part in splitter.split(&working) {
        let restored: String =
            part.chars().map(|c| if c == PLACEHOLDER { escaped.pop_front().unwrap_or(c) } else { c }).collect();
        let name = restored.trim();
        if !name.is_empty() && !out.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
            out.push(name.to_owned());
        }
    }
    out
}

/// "Song (feat. B & C)" → ("Song", ["B", "C"]); a title without a featured part comes back whole.
pub fn artists_from_title(title: &str) -> (String, Vec<String>) {
    let Some(found) = TITLE_FEAT.captures(title) else { return (title.trim().to_owned(), Vec::new()) };
    let whole = found.get(0).expect("match");
    let artists = split_artists(found.get(1).map_or("", |m| m.as_str()));
    let cleaned = format!("{}{}", &title[..whole.start()], &title[whole.end()..]);
    (cleaned.trim().to_owned(), artists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_characters_and_words() {
        assert_eq!(split_artists("Alpha feat. Beta"), ["Alpha", "Beta"]);
        assert_eq!(split_artists("Gamma / Delta"), ["Gamma", "Delta"]);
        assert_eq!(split_artists("A, B & C; D + E"), ["A", "B", "C", "D", "E"]);
        assert_eq!(split_artists("A vs B FT. C"), ["A", "B", "C"]);
        assert_eq!(split_artists("  "), Vec::<String>::new());
    }

    #[test]
    fn words_need_spaces_around_them() {
        assert_eq!(split_artists("Withered Hand"), ["Withered Hand"]);
        assert_eq!(split_artists("Daft Punk"), ["Daft Punk"]);
        assert_eq!(split_artists("Prodigy"), ["Prodigy"]);
    }

    #[test]
    fn escaped_delimiters_stay() {
        assert_eq!(split_artists(r"AC\/DC / Queen"), ["AC/DC", "Queen"]);
        assert_eq!(split_artists(r"Simon \& Garfunkel"), ["Simon & Garfunkel"]);
    }

    #[test]
    fn duplicates_are_dropped() {
        assert_eq!(split_artists("A / a / B"), ["A", "B"]);
    }

    #[test]
    fn featured_artists_come_out_of_the_title() {
        assert_eq!(artists_from_title("Song (feat. B & C)"), ("Song".to_owned(), vec!["B".to_owned(), "C".to_owned()]));
        assert_eq!(artists_from_title("Song [ft. B] (Remix)"), ("Song (Remix)".to_owned(), vec!["B".to_owned()]));
        assert_eq!(artists_from_title("Plain"), ("Plain".to_owned(), Vec::new()));
    }
}
