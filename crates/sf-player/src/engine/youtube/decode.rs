//! AAC in MP4 (YouTube's itag 140) decoded with symphonia into what the output chain takes: interleaved
//! stereo f64 at 44 100 Hz. Mono is doubled, more than two channels keep the first two, other rates go
//! through [`Resampler`](crate::engine::resample::Resampler).
use std::sync::{Arc, Mutex};

use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder as CodecDecoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use super::download::DownloadError;
use crate::engine::resample::Resampler;

/// The listener's text for a failure that came from the stream's download (a refused or wrong answer), so it is
/// not reported as "unsupported audio"; the details go to the log.
fn download_failure(error: &Error) -> Option<String> {
    let Error::IoError(io) = error else { return None };
    let download = io.get_ref()?.downcast_ref::<DownloadError>()?;
    log::info!("youtube decode: {download}");
    Some(download.user_message())
}

/// The source with a note of the download failure its reads met: the probe answers a source that fails at once
/// with "no suitable format reader" and drops the read's own error.
struct Watched {
    inner: Box<dyn MediaSource>,
    failure: Arc<Mutex<Option<DownloadError>>>,
}

impl std::io::Read for Watched {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf).inspect_err(|error| {
            if let Some(download) = error.get_ref().and_then(|inner| inner.downcast_ref::<DownloadError>()) {
                *self.failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(download.clone());
            }
        })
    }
}

impl std::io::Seek for Watched {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}

impl MediaSource for Watched {
    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }
}

/// Packets that fail to decode in a row before the stream counts as broken.
const MAX_BAD_PACKETS: u32 = 32;

pub struct Decoder {
    format: Box<dyn FormatReader>,
    codec: Box<dyn CodecDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    /// the source rate the resampler was made for
    rate: u32,
    resampler: Option<Resampler>,
    samples: Option<SampleBuffer<f32>>,
    /// frames before this timestamp are dropped (an accurate seek lands mid-packet)
    skip_until: u64,
    position_ms: u64,
    duration_ms: Option<u64>,
}

impl Decoder {
    /// Probes the container (MP4 first) and opens the first decodable track.
    pub fn new(source: Box<dyn MediaSource>) -> Result<Self, String> {
        Self::open(source, "audio/mp4")
    }

    /// Probes the container the mime names first (`audio/flac`, `audio/mp4`) and opens the first decodable
    /// track; the probe still recognises the other containers by their magic bytes.
    pub fn open(source: Box<dyn MediaSource>, mime: &str) -> Result<Self, String> {
        let failure = Arc::new(Mutex::new(None));
        let stream = MediaSourceStream::new(Box::new(Watched { inner: source, failure: failure.clone() }), Default::default());
        let mime = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
        let mut hint = Hint::new();
        hint.with_extension(if mime.ends_with("flac") { "flac" } else { "m4a" });
        if !mime.is_empty() {
            hint.mime_type(&mime);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
            .map_err(|error| {
                let refused = failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take().map(|download| {
                    log::info!("youtube decode: {download}");
                    download.user_message()
                });
                download_failure(&error).or(refused).unwrap_or_else(|| format!("unsupported audio: {error}"))
            })?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "no audio track".to_string())?;
        let codec = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|error| format!("unsupported codec: {error}"))?;
        let time_base = track.codec_params.time_base;
        let duration_ms = match (time_base, track.codec_params.n_frames) {
            (Some(base), Some(frames)) => Some(time_ms(base.calc_time(frames))),
            (None, Some(frames)) => track.codec_params.sample_rate.map(|rate| frames * 1000 / u64::from(rate.max(1))),
            _ => None,
        };
        Ok(Self {
            track_id: track.id,
            time_base,
            rate: 0,
            resampler: None,
            samples: None,
            skip_until: 0,
            position_ms: 0,
            duration_ms,
            format,
            codec,
        })
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Where the audio returned so far ends.
    pub fn position_ms(&self) -> u64 {
        self.position_ms
    }

    /// The next piece of audio; `None` at the end of the stream.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<f64>>, String> {
        let mut bad = 0u32;
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(Error::ResetRequired) => {
                    self.codec.reset();
                    continue;
                }
                Err(error) => return Err(download_failure(&error).unwrap_or_else(|| format!("read failed: {error}"))),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let (ts, frames) = (packet.ts(), packet.dur());
            let decoded = match self.codec.decode(&packet) {
                Ok(decoded) => decoded,
                Err(Error::DecodeError(error)) => {
                    bad += 1;
                    log::debug!("youtube decode: skipped a packet: {error}");
                    if bad > MAX_BAD_PACKETS {
                        return Err(format!("decode failed: {error}"));
                    }
                    continue;
                }
                Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(error) => return Err(format!("decode failed: {error}")),
            };
            bad = 0;
            self.position_ms = ts_ms(self.time_base, ts + frames);
            if ts + frames <= self.skip_until || decoded.frames() == 0 {
                continue;
            }
            let spec = *decoded.spec();
            let channels = spec.channels.count().max(1);
            let samples = match &mut self.samples {
                Some(buffer) if buffer.capacity() >= decoded.capacity() * channels => buffer,
                slot => slot.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
            };
            samples.copy_interleaved_ref(decoded);
            let mut interleaved = samples.samples();
            // drop the part before an accurate seek target
            if self.skip_until > ts {
                let drop_frames = ((self.skip_until - ts) as usize).min(interleaved.len() / channels);
                interleaved = &interleaved[drop_frames * channels..];
            }
            self.skip_until = 0;
            let stereo = to_stereo(interleaved, channels);
            if spec.rate != self.rate {
                self.rate = spec.rate;
                self.resampler = Resampler::new(spec.rate, SAMPLE_RATE, NUM_CHANNELS as usize);
            }
            let out = match &mut self.resampler {
                Some(resampler) => resampler.process(&stereo),
                None => stereo,
            };
            if out.is_empty() {
                continue;
            }
            return Ok(Some(out.into_iter().map(f64::from).collect()));
        }
    }

    /// Moves to `ms`; the next chunk starts there (within a packet). Past the end it goes to the last moment: the
    /// track's length shown may be another copy's (a lossless copy can be shorter than the YouTube song).
    pub fn seek(&mut self, ms: u64) -> Result<u64, String> {
        let ms = match self.duration_ms {
            Some(duration) if duration > 0 => ms.min(duration - 1),
            _ => ms,
        };
        let time = Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0);
        let seeked = self
            .format
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(|error| format!("seek failed: {error}"))?;
        self.codec.reset();
        // a fresh filter state: the old tail belongs to another place in the track
        self.rate = 0;
        self.resampler = None;
        self.skip_until = seeked.required_ts;
        self.position_ms = ts_ms(self.time_base, seeked.required_ts);
        Ok(self.position_ms)
    }

}

fn ts_ms(time_base: Option<TimeBase>, ts: u64) -> u64 {
    match time_base {
        Some(base) => time_ms(base.calc_time(ts)),
        None => ts * 1000 / u64::from(SAMPLE_RATE),
    }
}

fn time_ms(time: Time) -> u64 {
    time.seconds * 1000 + (time.frac * 1000.0).round() as u64
}

fn to_stereo(interleaved: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        2 => interleaved.to_vec(),
        1 => interleaved.iter().flat_map(|&sample| [sample, sample]).collect(),
        _ => interleaved.chunks_exact(channels).flat_map(|frame| [frame[0], frame[1]]).collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    pub(crate) fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/youtube").join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    fn open(name: &str) -> Decoder {
        Decoder::new(Box::new(std::io::Cursor::new(fixture(name)))).unwrap()
    }

    pub(crate) fn lossless_fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lossless").join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    #[test]
    fn decodes_flac_by_mime() {
        let mut decoder =
            Decoder::open(Box::new(std::io::Cursor::new(lossless_fixture("tone_44k_stereo.flac"))), "audio/flac").unwrap();
        assert_eq!(decoder.duration_ms(), Some(1_000));
        let samples = decode_all(&mut decoder);
        // one second of 44.1 kHz stereo, sample-exact (FLAC has no priming)
        assert_eq!(samples.len() / 2, 44_100);
        assert!(samples.iter().all(|sample| sample.is_finite() && sample.abs() <= 1.0));
        let middle = &samples[40_000..60_000];
        assert!(tone_power(middle, 440.0) > 100.0 * tone_power(middle, 1_000.0));
    }

    #[test]
    fn flac_seeks() {
        let mut decoder =
            Decoder::open(Box::new(std::io::Cursor::new(lossless_fixture("tone_44k_stereo.flac"))), "audio/flac").unwrap();
        let landed = decoder.seek(500).unwrap();
        assert!(landed.abs_diff(500) < 100, "landed at {landed}");
        // past the end (the length shown was another copy's): the song ends instead of failing
        assert!(decoder.seek(5_000).unwrap() >= 990);
        assert!(decode_all(&mut decoder).len() / 2 <= 441);
    }

    /// A Monochrome FLAC (no seek table) fetched by ranges from a server that, like Monochrome's at times, stops
    /// sending in the middle of an answer: a seek far ahead lands and the song keeps playing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_downloading_flac_keeps_playing_after_a_far_seek() {
        use crate::engine::youtube::download::{self, DownloadSpec, tests::{Mode, serve}};
        let body = lossless_fixture("sweep_20s_noseektable.flac");
        let length = body.len() as u64;
        let (url, _) = serve(body, Mode::Stalls(24 * 1024)).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // chunks small next to the file, as 4 MiB are next to a 30 MB song
        let spec = DownloadSpec {
            chunk_bytes: 64 * 1024,
            idle_timeout: std::time::Duration::from_millis(200),
            ..DownloadSpec::new(&url, "test-agent", Some(length))
        };
        let mut reader = download::start(client, &tokio::runtime::Handle::current(), spec);
        reader.set_timeout(std::time::Duration::from_secs(5));
        let started = std::time::Instant::now();
        let (landed, after, position) = tokio::task::spawn_blocking(move || {
            let mut decoder = Decoder::open(Box::new(reader), "audio/flac").unwrap();
            assert_eq!(decoder.duration_ms(), Some(20_000));
            for _ in 0..4 {
                decoder.next_chunk().unwrap().expect("audio before the seek");
            }
            let landed = decoder.seek(16_000).unwrap();
            // three seconds of audio after the seek
            let mut after = Vec::new();
            while after.len() < 3 * 44_100 * 2 {
                match decoder.next_chunk().unwrap() {
                    Some(chunk) => after.extend(chunk),
                    None => break,
                }
            }
            (landed, after, decoder.position_ms())
        })
        .await
        .unwrap();
        assert!(landed.abs_diff(16_000) < 100, "landed at {landed}");
        assert!(after.len() >= 3 * 44_100 * 2, "only {} samples after the seek", after.len());
        assert!((18_900..=19_200).contains(&position), "position {position}");
        // it is sound, not silence
        let middle = &after[44_100..88_200];
        assert!(middle.iter().map(|sample| sample * sample).sum::<f64>() / middle.len() as f64 > 0.01);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn the_mime_is_only_a_hint() {
        // an MP4 stream mislabelled as FLAC still opens
        assert!(Decoder::open(Box::new(std::io::Cursor::new(fixture("tone_44k_stereo.m4a"))), "audio/flac").is_ok());
    }

    fn decode_all(decoder: &mut Decoder) -> Vec<f64> {
        let mut all = Vec::new();
        while let Some(chunk) = decoder.next_chunk().unwrap() {
            all.extend(chunk);
        }
        all
    }

    /// Strength of a sine at `hz` in a stretch of the left channel (Goertzel).
    fn tone_power(samples: &[f64], hz: f64) -> f64 {
        let left: Vec<f64> = samples.as_chunks::<2>().0.iter().map(|frame| frame[0]).collect();
        let coefficient = 2.0 * (2.0 * std::f64::consts::PI * hz / f64::from(SAMPLE_RATE)).cos();
        let (mut previous, mut before) = (0.0, 0.0);
        for sample in &left {
            let current = sample + coefficient * previous - before;
            before = previous;
            previous = current;
        }
        (previous * previous + before * before - coefficient * previous * before) / left.len() as f64
    }

    #[test]
    fn decodes_stereo_44k() {
        let mut decoder = open("tone_44k_stereo.m4a");
        let duration = decoder.duration_ms().unwrap();
        assert!((1_950..=2_100).contains(&duration), "duration {duration}");
        let samples = decode_all(&mut decoder);
        let frames = samples.len() / 2;
        // 2 s plus the encoder's priming and padding (symphonia's MP4 reader keeps them): 88 AAC frames
        assert!((86_000..=91_000).contains(&frames), "frames {frames}");
        assert!(samples.iter().all(|sample| sample.is_finite() && sample.abs() <= 1.5));
        let middle = &samples[40_000..60_000];
        assert!(tone_power(middle, 440.0) > 100.0 * tone_power(middle, 1_000.0));
        assert!(decoder.position_ms() >= 1_950);
    }

    #[test]
    fn resamples_and_widens_48k_mono() {
        let mut decoder = open("tone_48k_mono.m4a");
        let samples = decode_all(&mut decoder);
        let frames = samples.len() / 2;
        // one second at 44.1 kHz, give or take the encoder's padding and the filter's delay
        assert!((42_000..=46_000).contains(&frames), "frames {frames}");
        let middle = &samples[20_000..60_000];
        assert!(middle.as_chunks::<2>().0.iter().all(|frame| frame[0] == frame[1]), "mono is copied to both sides");
        assert!(tone_power(middle, 1_000.0) > 100.0 * tone_power(middle, 440.0));
    }

    #[test]
    fn seek_lands_near_its_target() {
        let mut decoder = open("tone_44k_stereo.m4a");
        decoder.next_chunk().unwrap();
        let landed = decoder.seek(1_000).unwrap();
        assert!(landed.abs_diff(1_000) <= 30, "landed at {landed}");
        let rest = decode_all(&mut decoder);
        let frames = rest.len() / 2;
        assert!((43_000..=47_000).contains(&frames), "frames after the seek {frames}");
        // and back to the start
        assert_eq!(decoder.seek(0).unwrap(), 0);
        assert!(decode_all(&mut decoder).len() / 2 > 86_000);
    }

    #[test]
    fn garbage_is_refused() {
        assert!(Decoder::new(Box::new(std::io::Cursor::new(vec![0u8; 4096]))).is_err());
    }

    /// A source whose download already failed.
    struct Refused(DownloadError);

    impl std::io::Read for Refused {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::InvalidData, self.0.clone()))
        }
    }

    impl std::io::Seek for Refused {
        fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
            Ok(0)
        }
    }

    impl MediaSource for Refused {
        fn is_seekable(&self) -> bool {
            true
        }
        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    #[test]
    fn a_refused_download_is_not_unsupported_audio() {
        let invalid = DownloadError::Invalid("200 text/html".into());
        let error = Decoder::new(Box::new(Refused(invalid.clone()))).err().expect("refused");
        assert_eq!(error, invalid.user_message());
        assert!(!error.contains("unsupported"));
    }
}
