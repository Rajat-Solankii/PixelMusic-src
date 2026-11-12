//! What one music file says about itself, read with symphonia (the decoder the player already has, so no
//! second tag library): ID3v2 (MP3), Vorbis comments (FLAC, Ogg), MP4 atoms (M4A/ALAC) and RIFF INFO (WAV);
//! the embedded cover; the duration and the stream's format. soundwave1 read the same with jaudiotagger.
use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::{CODEC_TYPE_ALAC, CODEC_TYPE_FLAC, CODEC_TYPE_NULL, CodecType};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey, Visual};
use symphonia::core::probe::Hint;

/// Files the scanner reads (by extension). Opus has no decoder in symphonia, so it is left out.
pub const EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "mp4", "aac", "wav", "wave", "ogg", "oga"];

/// The tags and format of one file; empty strings and zeros are "unknown".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: u32,
    pub disc_no: u32,
    pub year: String,
    pub genre: String,
    pub duration_ms: i64,
    pub sample_rate: u32,
    pub bit_depth: u32,
    pub channels: u32,
    /// "FLAC", "MP3", "AAC", "ALAC", "PCM", "VORBIS"
    pub codec: String,
    pub lossless: bool,
    /// the embedded picture (the front cover first) and its mime
    pub cover: Option<(String, Vec<u8>)>,
}

/// Whether a file name has one of [`EXTENSIONS`].
pub fn is_audio(path: &Path) -> bool {
    extension(path).is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()))
}

pub(crate) fn extension(path: &Path) -> Option<String> {
    path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase)
}

/// The mime the decoder's probe is hinted with.
pub fn mime_of(path: &Path) -> &'static str {
    match extension(path).as_deref() {
        Some("flac") => "audio/flac",
        Some("mp3") => "audio/mpeg",
        Some("wav" | "wave") => "audio/wav",
        Some("ogg" | "oga") => "audio/ogg",
        Some("aac") => "audio/aac",
        _ => "audio/mp4",
    }
}

/// Reads the file's tags, cover and format; `Err` when symphonia can't open it.
pub fn read(path: &Path) -> Result<FileTags, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = extension(path) {
        hint.with_extension(&ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|error| format!("unsupported audio: {error}"))?;
    let mut tags = FileTags::default();
    // tags in the container first, then those in front of it (ID3v2 before an MP3)
    if let Some(revision) = probed.format.metadata().current() {
        apply(&mut tags, revision);
    }
    if let Some(mut metadata) = probed.metadata.get()
        && let Some(revision) = metadata.skip_to_latest()
    {
        apply(&mut tags, revision);
    }
    let format = &mut probed.format;
    let Some(track) = format.tracks().iter().find(|track| track.codec_params.codec != CODEC_TYPE_NULL) else {
        return Err("no audio track".into());
    };
    let params = track.codec_params.clone();
    let track_id = track.id;
    tags.sample_rate = params.sample_rate.unwrap_or(0);
    tags.bit_depth = params.bits_per_sample.or(params.bits_per_coded_sample).unwrap_or(0);
    tags.channels = params.channels.map_or(0, |channels| channels.count() as u32);
    tags.codec = codec_name(params.codec);
    tags.lossless = is_lossless(params.codec);
    if !tags.lossless {
        tags.bit_depth = 0;
    }
    let frames_ms = |frames: u64| match (params.time_base, params.sample_rate) {
        (Some(base), _) => {
            let time = base.calc_time(frames);
            (time.seconds * 1000) as i64 + (time.frac * 1000.0) as i64
        }
        (None, Some(rate)) if rate > 0 => (frames * 1000 / u64::from(rate)) as i64,
        _ => 0,
    };
    tags.duration_ms = params.n_frames.map_or(0, frames_ms);
    if tags.duration_ms <= 0 {
        // no frame count in the header (an MP3 without a Xing frame): add up the packets, no decoding
        let mut frames = 0u64;
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() == track_id {
                frames = frames.max(packet.ts() + packet.dur());
            }
        }
        tags.duration_ms = frames_ms(frames);
    }
    Ok(tags)
}

fn apply(tags: &mut FileTags, revision: &MetadataRevision) {
    for tag in revision.tags() {
        let Some(key) = tag.std_key else { continue };
        let value = tag.value.to_string();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let slot = match key {
            StandardTagKey::TrackTitle => &mut tags.title,
            StandardTagKey::Artist => &mut tags.artist,
            StandardTagKey::Album => &mut tags.album,
            StandardTagKey::AlbumArtist => &mut tags.album_artist,
            StandardTagKey::Genre => &mut tags.genre,
            StandardTagKey::Date | StandardTagKey::ReleaseDate | StandardTagKey::OriginalDate => {
                if tags.year.is_empty() {
                    tags.year = value.chars().take(4).collect();
                }
                continue;
            }
            StandardTagKey::TrackNumber => {
                if tags.track_no == 0 {
                    tags.track_no = leading_number(value);
                }
                continue;
            }
            StandardTagKey::DiscNumber => {
                if tags.disc_no == 0 {
                    tags.disc_no = leading_number(value);
                }
                continue;
            }
            _ => continue,
        };
        if slot.is_empty() {
            *slot = value.to_owned();
        }
    }
    if tags.cover.is_none()
        && let Some(visual) = front_cover(revision.visuals())
    {
        tags.cover = Some((visual.media_type.clone(), visual.data.to_vec()));
    }
}

fn front_cover(visuals: &[Visual]) -> Option<&Visual> {
    visuals
        .iter()
        .find(|visual| visual.usage == Some(StandardVisualKey::FrontCover))
        .or_else(|| visuals.iter().find(|visual| !visual.data.is_empty()))
        .filter(|visual| !visual.data.is_empty())
}

/// "3/12" → 3
fn leading_number(value: &str) -> u32 {
    value.trim().split(|c: char| !c.is_ascii_digit()).next().and_then(|digits| digits.parse().ok()).unwrap_or(0)
}

fn is_lossless(codec: CodecType) -> bool {
    codec == CODEC_TYPE_FLAC || codec == CODEC_TYPE_ALAC || codec_name(codec) == "PCM"
}

fn codec_name(codec: CodecType) -> String {
    let short = symphonia::default::get_codecs().get_codec(codec).map_or("", |descriptor| descriptor.short_name);
    if short.starts_with("pcm") {
        return "PCM".into();
    }
    if short.is_empty() { "AUDIO".into() } else { short.to_ascii_uppercase() }
}

/// "FLAC 24/96", "PCM 16/44.1", "MP3 192" — soundwave1's quality line.
pub fn quality_label(codec: &str, lossless: bool, bit_depth: u32, sample_rate: u32, bitrate_kbps: u32) -> String {
    if lossless {
        return match (bit_depth, sample_rate) {
            (0, _) | (_, 0) => codec.to_owned(),
            (bits, rate) => {
                let khz = format!("{:.1}", f64::from(rate) / 1000.0);
                format!("{codec} {bits}/{}", khz.strip_suffix(".0").unwrap_or(&khz))
            }
        };
    }
    if bitrate_kbps > 0 { format!("{codec} {bitrate_kbps}") } else { codec.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_labels() {
        assert_eq!(leading_number("3/12"), 3);
        assert_eq!(leading_number("x"), 0);
        assert_eq!(quality_label("FLAC", true, 24, 96_000, 0), "FLAC 24/96");
        assert_eq!(quality_label("PCM", true, 16, 44_100, 0), "PCM 16/44.1");
        assert_eq!(quality_label("FLAC", true, 0, 0, 0), "FLAC");
        assert_eq!(quality_label("MP3", false, 0, 44_100, 192), "MP3 192");
        assert!(is_audio(Path::new("a/B.FLAC")) && !is_audio(Path::new("a/b.txt")) && !is_audio(Path::new("noext")));
        assert_eq!(mime_of(Path::new("x.mp3")), "audio/mpeg");
    }
}
