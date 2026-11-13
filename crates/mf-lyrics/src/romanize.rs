//! Romanization of lyric lines without dictionaries: Devanagari (Hindi) and Gurmukhi (Punjabi) by letter tables
//! with schwa deletion, Hangul by the Revised Romanization rules (with liaison), kana by table (small kana, っ, ー),
//! Cyrillic and Greek by table. Kanji/hanzi have no reading without a dictionary and stay as they are; Latin and
//! everything else passes through.

/// The line in Latin letters, or None when it has nothing to romanize (no letter of the scripts above).
pub fn romanize(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if !chars.iter().any(|&c| script(c) != Script::Other) {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let kind = script(chars[i]);
        let mut j = i + 1;
        while j < chars.len() && script(chars[j]) == kind {
            j += 1;
        }
        let run = &chars[i..j];
        match kind {
            Script::Devanagari => indic(run, &DEVANAGARI, &mut out),
            Script::Gurmukhi => indic(run, &GURMUKHI, &mut out),
            Script::Hangul => hangul(run, &mut out),
            Script::Kana => kana(run, &mut out),
            Script::Cyrillic => cyrillic(run, &mut out),
            Script::Greek => greek(run, &mut out),
            Script::Other => out.extend(run),
        }
        i = j;
    }
    Some(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Devanagari,
    Gurmukhi,
    Hangul,
    Kana,
    Cyrillic,
    Greek,
    Other,
}

fn script(c: char) -> Script {
    match c as u32 {
        0x0900..=0x097F => Script::Devanagari,
        0x0A00..=0x0A7F => Script::Gurmukhi,
        0xAC00..=0xD7A3 => Script::Hangul,
        0x3041..=0x3096 | 0x30A1..=0x30FA | 0x30FC => Script::Kana,
        0x0400..=0x04FF => Script::Cyrillic,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        _ => Script::Other,
    }
}

// ── Devanagari / Gurmukhi ────────────────────────────────────────────────────

enum Sign {
    Consonant(&'static str),
    Vowel(&'static str),
    Matra(&'static str),
    Virama,
    Nukta,
    Nasal,
    Visarga,
    Addak,
    Digit(char),
    Other(&'static str),
}

struct Indic {
    base: u32,
    table: &'static [&'static str],
    /// nukta: the letter it follows → its romanization
    nukta: &'static [(&'static str, &'static str)],
}

// Each entry: "c:x" consonant, "v:x" independent vowel, "m:x" matra, "~" nasal, "h" visarga, "_" virama,
// "." nukta, "+" addak, "" nothing, "o:x" other text.
const DEVANAGARI: Indic = Indic {
    base: 0x0900,
    table: &[
        "~", "~", "~", "h", "v:a", "v:a", "v:a", "v:i", "v:i", "v:u", "v:u", "v:ri", "v:li", "v:e", "v:e", "v:e", // 0900
        "v:ai", "v:o", "v:o", "v:o", "v:au", "c:k", "c:kh", "c:g", "c:gh", "c:n", "c:ch", "c:chh", "c:j", "c:jh", "c:n", "c:t", // 0910
        "c:th", "c:d", "c:dh", "c:n", "c:t", "c:th", "c:d", "c:dh", "c:n", "c:n", "c:p", "c:ph", "c:b", "c:bh", "c:m", "c:y", // 0920
        "c:r", "c:r", "c:l", "c:l", "c:l", "c:v", "c:sh", "c:sh", "c:s", "c:h", "m:e", "m:e", ".", "", "m:a", "m:i", // 0930
        "m:i", "m:u", "m:u", "m:ri", "m:ri", "m:e", "m:e", "m:e", "m:ai", "m:o", "m:o", "m:o", "m:au", "_", "m:e", "m:au", // 0940
        "o:om", "", "", "", "", "m:e", "m:u", "m:u", "c:q", "c:kh", "c:gh", "c:z", "c:r", "c:rh", "c:f", "c:y", // 0950
        "v:ri", "v:li", "m:li", "m:li", "o:.", "o:.", "d:0", "d:1", "d:2", "d:3", "d:4", "d:5", "d:6", "d:7", "d:8", "d:9", // 0960
        "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", // 0970
    ],
    nukta: &[("k", "q"), ("kh", "kh"), ("g", "gh"), ("j", "z"), ("d", "r"), ("dh", "rh"), ("ph", "f"), ("y", "y")],
};

const GURMUKHI: Indic = Indic {
    base: 0x0A00,
    table: &[
        "", "~", "~", "h", "", "v:a", "v:a", "v:i", "v:i", "v:u", "v:u", "", "", "", "", "v:e", // 0A00
        "v:ai", "", "", "v:o", "v:au", "c:k", "c:kh", "c:g", "c:gh", "c:n", "c:ch", "c:chh", "c:j", "c:jh", "c:n", "c:t", // 0A10
        "c:th", "c:d", "c:dh", "c:n", "c:t", "c:th", "c:d", "c:dh", "c:n", "", "c:p", "c:ph", "c:b", "c:bh", "c:m", "c:y", // 0A20
        "c:r", "", "c:l", "c:l", "", "c:v", "c:sh", "", "c:s", "c:h", "", "", ".", "", "m:a", "m:i", // 0A30
        "m:i", "m:u", "m:u", "", "", "", "", "m:e", "m:ai", "", "", "m:o", "m:au", "_", "", "", // 0A40
        "", "", "", "", "", "", "", "", "", "c:kh", "c:gh", "c:z", "c:r", "", "c:f", "", // 0A50
        "", "", "", "", "o:.", "o:.", "d:0", "d:1", "d:2", "d:3", "d:4", "d:5", "d:6", "d:7", "d:8", "d:9", // 0A60
        "~", "+", "v:", "v:", "o:ik", "", "", "", "", "", "", "", "", "", "", "", // 0A70
    ],
    nukta: &[("kh", "kh"), ("g", "gh"), ("j", "z"), ("ph", "f"), ("s", "sh"), ("l", "l")],
};

impl Indic {
    fn sign(&self, c: char) -> Sign {
        let code = c as u32;
        // the danda is shared (U+0964/0965 are in the Devanagari block)
        if code == 0x0964 || code == 0x0965 {
            return Sign::Other(".");
        }
        let entry = code.checked_sub(self.base).and_then(|i| self.table.get(i as usize)).copied().unwrap_or("");
        match entry.split_once(':') {
            Some(("c", x)) => Sign::Consonant(x),
            Some(("v", x)) => Sign::Vowel(x),
            Some(("m", x)) => Sign::Matra(x),
            Some(("o", x)) => Sign::Other(x),
            Some(("d", x)) => Sign::Digit(x.chars().next().unwrap_or('0')),
            _ => match entry {
                "~" => Sign::Nasal,
                "h" => Sign::Visarga,
                "_" => Sign::Virama,
                "." => Sign::Nukta,
                "+" => Sign::Addak,
                _ => Sign::Other(""),
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Vowel {
    /// the inherent "a", which schwa deletion may drop
    Schwa,
    Explicit(&'static str),
    /// virama: no vowel
    Halant,
}

struct Unit {
    cons: String,
    /// romanization of the last consonant (a following nukta changes it)
    last: &'static str,
    conjunct: bool,
    vowel: Vowel,
    nasal: String,
    deleted: bool,
}

impl Unit {
    fn sounds(&self) -> bool {
        match self.vowel {
            Vowel::Schwa => !self.deleted,
            Vowel::Explicit(v) => !v.is_empty(),
            Vowel::Halant => false,
        }
    }
}

/// One run of Devanagari or Gurmukhi: split into words (consonant clusters with their vowel), delete schwas, write.
fn indic(run: &[char], table: &Indic, out: &mut String) {
    let mut word: Vec<Unit> = Vec::new();
    let mut double = false;
    for &c in run {
        match table.sign(c) {
            Sign::Consonant(x) => {
                let mut cons = String::new();
                if std::mem::take(&mut double) {
                    cons.push_str(&x[..1]);
                }
                cons.push_str(x);
                match word.last_mut() {
                    Some(u) if u.vowel == Vowel::Halant && !u.cons.is_empty() => {
                        u.cons.push_str(&cons);
                        u.last = x;
                        u.conjunct = true;
                        u.vowel = Vowel::Schwa;
                    }
                    _ => word.push(Unit { cons, last: x, conjunct: false, vowel: Vowel::Schwa, nasal: String::new(), deleted: false }),
                }
            }
            Sign::Nukta => {
                if let Some(u) = word.last_mut()
                    && let Some(&(_, to)) = table.nukta.iter().find(|(from, _)| *from == u.last)
                    && u.cons.ends_with(u.last)
                {
                    let keep = u.cons.len() - u.last.len();
                    u.cons.truncate(keep);
                    u.cons.push_str(to);
                    u.last = to;
                }
            }
            Sign::Matra(m) => match word.last_mut() {
                Some(u) if u.vowel == Vowel::Schwa && u.nasal.is_empty() => u.vowel = Vowel::Explicit(m),
                _ => word.push(Unit { cons: String::new(), last: "", conjunct: false, vowel: Vowel::Explicit(m), nasal: String::new(), deleted: false }),
            },
            Sign::Vowel(v) => {
                word.push(Unit { cons: String::new(), last: "", conjunct: false, vowel: Vowel::Explicit(v), nasal: String::new(), deleted: false })
            }
            Sign::Virama => {
                if let Some(u) = word.last_mut() {
                    u.vowel = Vowel::Halant;
                }
            }
            Sign::Nasal | Sign::Visarga => {
                let mark = if matches!(table.sign(c), Sign::Visarga) { "h" } else { "n" };
                match word.last_mut() {
                    Some(u) => u.nasal.push_str(mark),
                    None => word.push(Unit { cons: String::new(), last: "", conjunct: false, vowel: Vowel::Explicit(""), nasal: mark.into(), deleted: false }),
                }
            }
            Sign::Addak => double = true,
            Sign::Digit(d) => {
                write_word(&mut word, out);
                out.push(d);
            }
            Sign::Other(x) => {
                write_word(&mut word, out);
                out.push_str(x);
            }
        }
    }
    write_word(&mut word, out);
}

/// Schwa deletion (Ohala's rule, right to left: V C(a) C V drops that a; a word-final a after a single consonant
/// drops when the word has more than one syllable), then the letters.
fn write_word(word: &mut Vec<Unit>, out: &mut String) {
    let n = word.len();
    if n > 1 {
        let last = &mut word[n - 1];
        if last.vowel == Vowel::Schwa && !last.conjunct && last.nasal.is_empty() && !last.cons.is_empty() {
            last.deleted = true;
        }
        for i in (1..n - 1).rev() {
            let (prev, unit, next) = (&word[i - 1], &word[i], &word[i + 1]);
            if unit.vowel == Vowel::Schwa && !unit.conjunct && unit.nasal.is_empty() && !unit.cons.is_empty()
                && prev.sounds() && next.sounds() && !next.cons.is_empty() && !next.conjunct
            {
                word[i].deleted = true;
            }
        }
    }
    for i in 0..n {
        let u = &word[i];
        out.push_str(&u.cons);
        match u.vowel {
            Vowel::Schwa if !u.deleted => out.push('a'),
            Vowel::Explicit(v) => out.push_str(v),
            _ => {}
        }
        if u.nasal.starts_with('n') && word.get(i + 1).is_some_and(|n| n.cons.starts_with(['p', 'b', 'm'])) {
            out.push('m');
            out.push_str(&u.nasal[1..]);
        } else {
            out.push_str(&u.nasal);
        }
    }
    word.clear();
}

// ── Hangul (Revised Romanization) ────────────────────────────────────────────

const INITIALS: [&str; 19] = ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"];
const MEDIALS: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui", "i",
];
/// final consonant before a consonant or the end of the word
const FINALS: [&str; 28] =
    ["", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "l", "l", "l", "p", "l", "m", "p", "p", "t", "t", "ng", "t", "t", "k", "t", "p", "t"];
/// final consonant carried over to a following silent ㅇ (liaison)
const LIAISON: [&str; 28] =
    ["", "g", "kk", "ks", "n", "nj", "n", "d", "r", "lg", "lm", "lb", "ls", "lt", "lp", "r", "m", "b", "ps", "s", "ss", "ng", "j", "ch", "k", "t", "p", ""];

fn jamo(c: char) -> Option<(usize, usize, usize)> {
    let s = (c as u32).checked_sub(0xAC00).filter(|&s| s < 11172)? as usize;
    Some((s / 588, s % 588 / 28, s % 28))
}

fn hangul(run: &[char], out: &mut String) {
    let mut carried = false;
    let mut prev_final = 0;
    for (i, &c) in run.iter().enumerate() {
        let Some((l, v, t)) = jamo(c) else { continue };
        if carried {
            // the previous final already sounds as this syllable's initial
        } else if l == 5 && prev_final == 8 {
            out.push('l');
        } else {
            out.push_str(INITIALS[l]);
        }
        out.push_str(MEDIALS[v]);
        let next_silent = run.get(i + 1).and_then(|&n| jamo(n)).is_some_and(|(nl, _, _)| nl == 11);
        carried = t != 0 && t != 21 && next_silent;
        out.push_str(if carried { LIAISON[t] } else { FINALS[t] });
        prev_final = t;
    }
}

// ── Kana (Hepburn) ───────────────────────────────────────────────────────────

/// Hiragana U+3041..=U+3096 (katakana is the same 0x60 higher)
const KANA: [&str; 86] = [
    "a", "a", "i", "i", "u", "u", "e", "e", "o", "o", "ka", "ga", "ki", "gi", "ku", "gu", "ke", "ge", "ko", "go", // 3041
    "sa", "za", "shi", "ji", "su", "zu", "se", "ze", "so", "zo", "ta", "da", "chi", "ji", "", "tsu", "zu", "te", "de", "to", // 3055
    "do", "na", "ni", "nu", "ne", "no", "ha", "ba", "pa", "hi", "bi", "pi", "fu", "bu", "pu", "he", "be", "pe", "ho", "bo", // 3069
    "po", "ma", "mi", "mu", "me", "mo", "ya", "ya", "yu", "yu", "yo", "yo", "ra", "ri", "ru", "re", "ro", "wa", "wa", "i", // 307D
    "e", "o", "n", "vu", "ka", "ke", // 3091
];

fn kana_index(c: char) -> Option<usize> {
    let code = c as u32;
    let code = if (0x30A1..=0x30F6).contains(&code) { code - 0x60 } else { code };
    (0x3041..=0x3096).contains(&code).then(|| (code - 0x3041) as usize)
}

fn small_vowel(i: usize) -> Option<&'static str> {
    [0usize, 2, 4, 6, 8].contains(&i).then(|| KANA[i])
}

fn small_y(i: usize) -> Option<&'static str> {
    [0x42usize, 0x44, 0x46].contains(&i).then(|| KANA[i])
}

const SMALL_TSU: usize = 0x22;

fn kana(run: &[char], out: &mut String) {
    let mut double = false;
    let mut i = 0;
    while i < run.len() {
        let c = run[i];
        i += 1;
        let mut syl: String = match c as u32 {
            0x30FC => {
                // ー: hold the last vowel
                if let Some(v) = out.chars().last().filter(|v| "aeiou".contains(*v)) {
                    out.push(v);
                }
                continue;
            }
            0x30F7..=0x30FA => ["va", "vi", "ve", "vo"][(c as u32 - 0x30F7) as usize].to_owned(),
            _ => match kana_index(c) {
                Some(SMALL_TSU) => {
                    double = true;
                    continue;
                }
                Some(k) => KANA[k].to_owned(),
                None => continue,
            },
        };
        // きゃ → kya, しゃ → sha; ファ → fa, ティ → ti, ウィ → wi
        if let Some(next) = run.get(i).and_then(|&n| kana_index(n)) {
            if let Some(y) = small_y(next).filter(|_| syl.len() > 1 && syl.ends_with('i')) {
                syl.pop();
                if matches!(syl.as_str(), "sh" | "ch" | "j") {
                    syl.push_str(&y[1..]);
                } else {
                    syl.push_str(y);
                }
                i += 1;
            } else if let Some(v) = small_vowel(next).filter(|_| syl != "a" && syl != "i" && syl != "e" && syl != "o") {
                if syl == "u" {
                    syl = "w".into();
                } else {
                    syl.pop();
                }
                syl.push_str(v);
                i += 1;
            }
        }
        if std::mem::take(&mut double) {
            let first = if syl.starts_with("ch") { 't' } else { syl.chars().next().unwrap_or(' ') };
            if !"aeiou".contains(first) {
                out.push(first);
            }
        }
        out.push_str(&syl);
    }
}

// ── Cyrillic, Greek ──────────────────────────────────────────────────────────

fn push_cased(out: &mut String, latin: &str, upper: bool) {
    if !upper {
        out.push_str(latin);
        return;
    }
    let mut chars = latin.chars();
    if let Some(first) = chars.next() {
        out.extend(first.to_uppercase());
        out.push_str(chars.as_str());
    }
}

fn cyrillic(run: &[char], out: &mut String) {
    for &c in run {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let latin = match lower {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' => "e", 'ё' => "yo", 'ж' => "zh",
            'з' => "z", 'и' => "i", 'й' => "y", 'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o",
            'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u", 'ф' => "f", 'х' => "kh", 'ц' => "ts",
            'ч' => "ch", 'ш' => "sh", 'щ' => "shch", 'ъ' | 'ь' => "", 'ы' => "y", 'э' => "e", 'ю' => "yu",
            'я' => "ya", 'і' => "i", 'ї' => "yi", 'є' => "ye", 'ґ' => "g", 'ў' => "w", 'ђ' => "dj", 'ј' => "j",
            'љ' => "lj", 'њ' => "nj", 'ћ' => "c", 'џ' => "dz", 'ѓ' => "gj", 'ќ' => "kj", 'ѕ' => "dz",
            _ => {
                out.push(c);
                continue;
            }
        };
        push_cased(out, latin, c != lower);
    }
}

fn greek(run: &[char], out: &mut String) {
    let mut i = 0;
    while i < run.len() {
        let c = run[i];
        i += 1;
        let lower = c.to_lowercase().next().unwrap_or(c);
        let base = strip_greek_accent(lower);
        // ου → ou
        if base == 'ο' && run.get(i).is_some_and(|&n| matches!(strip_greek_accent(n.to_lowercase().next().unwrap_or(n)), 'υ')) {
            push_cased(out, "ou", c != lower);
            i += 1;
            continue;
        }
        let latin = match base {
            'α' => "a", 'β' => "v", 'γ' => "g", 'δ' => "d", 'ε' => "e", 'ζ' => "z", 'η' => "i", 'θ' => "th",
            'ι' => "i", 'κ' => "k", 'λ' => "l", 'μ' => "m", 'ν' => "n", 'ξ' => "x", 'ο' => "o", 'π' => "p",
            'ρ' => "r", 'σ' | 'ς' => "s", 'τ' => "t", 'υ' => "y", 'φ' => "f", 'χ' => "ch", 'ψ' => "ps", 'ω' => "o",
            '\u{37e}' => "?",
            _ => {
                out.push(c);
                continue;
            }
        };
        push_cased(out, latin, c != lower);
    }
}

fn strip_greek_accent(c: char) -> char {
    match c {
        'ά' => 'α', 'έ' => 'ε', 'ή' => 'η', 'ί' | 'ϊ' | 'ΐ' => 'ι', 'ό' => 'ο', 'ύ' | 'ϋ' | 'ΰ' => 'υ', 'ώ' => 'ω',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::romanize;

    fn r(s: &str) -> String {
        romanize(s).unwrap()
    }

    #[test]
    fn latin_and_han_are_left_alone() {
        assert_eq!(romanize("Hello, world"), None);
        assert_eq!(romanize("夜に駆ける"), Some("夜ni駆keru".into()));
    }

    #[test]
    fn hindi_drops_schwas_like_it_is_spoken() {
        assert_eq!(r("दिल"), "dil");
        assert_eq!(r("कमला"), "kamla");
        assert_eq!(r("समझना"), "samajhna");
        assert_eq!(r("तुम ही हो"), "tum hi ho");
        assert_eq!(r("मैं तेरा प्यार"), "main tera pyar");
        assert_eq!(r("क्या है"), "kya hai");
        assert_eq!(r("नहीं"), "nahin");
        assert_eq!(r("मित्र"), "mitra");
        assert_eq!(r("ज़िन्दगी"), "zindagi");
        assert_eq!(r("संग"), "sang");
        assert_eq!(r("चंपा"), "champa");
        assert_eq!(r("न"), "na");
        assert_eq!(r("दिल, जान।"), "dil, jan.");
    }

    #[test]
    fn punjabi_doubles_after_addak() {
        assert_eq!(r("ਸਿੱਧੂ ਮੂਸੇ ਵਾਲਾ"), "siddhu muse wala".replace('w', "v"));
        assert_eq!(r("ਪੰਜਾਬ"), "panjab");
        assert_eq!(r("ਦਿਲ"), "dil");
        assert_eq!(r("ਗੱਲ"), "gall");
    }

    #[test]
    fn hangul_follows_revised_romanization() {
        assert_eq!(r("사랑해"), "saranghae");
        assert_eq!(r("음악"), "eumak");
        assert_eq!(r("한국어"), "hangugeo");
        assert_eq!(r("서울"), "seoul");
        assert_eq!(r("별빛"), "byeolbit");
        assert_eq!(r("빨리"), "ppalli");
    }

    #[test]
    fn kana_by_hepburn() {
        assert_eq!(r("ありがとう"), "arigatou");
        assert_eq!(r("きょう"), "kyou");
        assert_eq!(r("しゃしん"), "shashin");
        assert_eq!(r("ちょっと"), "chotto");
        assert_eq!(r("マッチ"), "matchi");
        assert_eq!(r("コーヒー"), "koohii");
        assert_eq!(r("ファン"), "fan");
        assert_eq!(r("パーティー"), "paatii");
        assert_eq!(r("ウィンドウ"), "windou");
    }

    #[test]
    fn cyrillic_and_greek_by_table() {
        assert_eq!(r("Группа крови"), "Gruppa krovi");
        assert_eq!(r("Щастя ї"), "Shchastya yi");
        assert_eq!(r("Σ' αγαπώ"), "S' agapo");
        assert_eq!(r("Ουρανός"), "Ouranos");
    }
}
