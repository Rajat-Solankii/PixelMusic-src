//! The YouTube player thread: opens a stream (download + probe on a helper thread, so a slow network never
//! blocks the commands), decodes it and writes the samples to the output chain librespot uses
//! (`RodioSink` behind `Tapped`). `write` blocks while the output queue is full, which paces the decoder.
//!
//! Every load carries a sequence number from the caller; events name the load they belong to, so the actor
//! can drop what a superseded track still reports.
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use librespot_playback::audio_backend::Sink;
use librespot_playback::config::VolumeCtrl;
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::VolumeGetter;
use librespot_playback::mixer::mappings::MappedCtrl;
use symphonia::core::io::MediaSource;

use super::decode::Decoder;
use super::download::{self, DownloadSpec};
use super::{YtOutput, YtStream};
use crate::engine::sink::{AudioControl, ErrorHook, RodioSink};
use crate::engine::vis::{AudioTap, Tapped};

/// What the actor asks of the player.
#[derive(Clone, Debug, PartialEq)]
pub enum YtCommand {
    /// Open `stream` (load number `seq`) at `position_ms`, playing or paused.
    Load { seq: u64, stream: Box<YtStream>, position_ms: u64, play: bool },
    Play,
    Pause,
    Seek(u64),
    /// Stop and forget the track (another source takes over).
    Stop,
    /// 0..=65535, mapped like librespot's soft mixer
    Volume(u16),
    Shutdown,
}

/// What the player reports, with the load it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub enum YtEvent {
    Loading,
    Playing { position_ms: u64 },
    Paused { position_ms: u64 },
    /// while playing, about twice a second
    Position(u64),
    Ended,
    Error(String),
}

pub type Notify = Arc<dyn Fn(u64, YtEvent) + Send + Sync>;
/// Turns a stream into bytes to decode (the real one downloads it; tests read fixtures).
pub type Opener = Arc<dyn Fn(&YtStream) -> Result<Box<dyn MediaSource>, String> + Send + Sync>;
pub type SinkFactory = Box<dyn FnOnce() -> Box<dyn Sink> + Send>;

/// How often `Position` is reported while playing.
const POSITION_EVERY_MS: u64 = 500;

enum Input {
    Command(YtCommand),
    Opened { seq: u64, result: Result<Decoder, String> },
    Seeked { id: u64, decoder: Decoder, result: Result<u64, String> },
}

pub struct YtPlayer {
    tx: Sender<Input>,
    thread: Option<JoinHandle<()>>,
    tap: Arc<AudioTap>,
}

/// Downloads streams over the shared HTTP client (proxy included) on `runtime`.
pub fn http_opener(http: sf_api::Http, runtime: tokio::runtime::Handle) -> Opener {
    Arc::new(move |stream: &YtStream| {
        if let Some(source) = open_local(stream) {
            return source;
        }
        let client = http.client()?;
        let spec = DownloadSpec::new(stream.url.clone(), stream.user_agent.clone(), stream.content_length);
        Ok(Box::new(download::start(client, &runtime, spec)) as Box<dyn MediaSource>)
    })
}

/// A local file's stream (`file:<path>`, sf-services `local`) read from disk; `None` — not a local file.
pub fn open_local(stream: &YtStream) -> Option<Result<Box<dyn MediaSource>, String>> {
    let path = stream.url.strip_prefix(super::LOCAL_FILE_URL)?;
    let opened = std::fs::File::open(path).map_err(|error| format!("{path}: {error}"));
    Some(opened.map(|file| Box::new(file) as Box<dyn MediaSource>))
}

/// librespot's volume curve over a shared 0..=65535 value.
struct MappedVolume(Arc<AtomicU16>);

impl VolumeGetter for MappedVolume {
    fn attenuation_factor(&self) -> f64 {
        VolumeCtrl::Cubic(VolumeCtrl::DEFAULT_DB_RANGE).to_mapped(self.0.load(Ordering::Relaxed))
    }
}

/// The normalisation factor for a stream's loudness: attenuation only.
pub fn normalisation_factor(loudness_db: Option<f64>) -> f64 {
    match loudness_db {
        Some(db) if db.is_finite() => 10f64.powf(-db / 20.0).min(1.0),
        _ => 1.0,
    }
}

/// Parts of the output the player drives besides the sink itself.
struct Output {
    make_sink: Option<SinkFactory>,
    control: Option<Arc<AudioControl>>,
    normalise: bool,
    normalisation: Arc<AtomicU64>,
    /// what the output holds before it is heard, taken off the reported position
    latency_ms: u64,
}

impl YtPlayer {
    /// The player over the real output device.
    pub fn spawn(output: YtOutput, opener: Opener, notify: Notify) -> Self {
        let tap = AudioTap::new();
        let volume = Arc::new(AtomicU16::new(u16::MAX));
        let control = AudioControl::new(output.buffer_ms);
        let normalisation = Arc::new(AtomicU64::new(1f64.to_bits()));
        let make_sink: SinkFactory = {
            let (tap, volume, control, normalisation) =
                (tap.clone(), volume.clone(), control.clone(), normalisation.clone());
            Box::new(move || {
                // write() fails as well and is reported from there; the hook only logs
                let on_error: ErrorHook = Arc::new(|message: String| log::warn!("youtube output: {message}"));
                let sink = RodioSink::new(
                    output.audio_device,
                    on_error,
                    Box::new(MappedVolume(volume.clone())),
                    output.buffer_ms,
                    control.clone(),
                );
                Box::new(Tapped::new(
                    Box::new(sink),
                    control,
                    tap,
                    Box::new(MappedVolume(volume)),
                    false,
                    output.eq,
                    normalisation,
                ))
            })
        };
        let parts = Output {
            make_sink: Some(make_sink),
            control: Some(control),
            normalise: output.normalisation,
            normalisation,
            latency_ms: u64::from(output.buffer_ms),
        };
        Self::start(parts, volume, tap, opener, notify)
    }

    /// The player over any sink (tests record what it writes).
    pub fn with_sink(make_sink: SinkFactory, normalise: bool, opener: Opener, notify: Notify) -> Self {
        let parts = Output {
            make_sink: Some(make_sink),
            control: None,
            normalise,
            normalisation: Arc::new(AtomicU64::new(1f64.to_bits())),
            latency_ms: 0,
        };
        Self::start(parts, Arc::new(AtomicU16::new(u16::MAX)), AudioTap::new(), opener, notify)
    }

    fn start(output: Output, volume: Arc<AtomicU16>, tap: Arc<AudioTap>, opener: Opener, notify: Notify) -> Self {
        let (tx, rx) = mpsc::channel();
        let loopback = tx.clone();
        let thread = std::thread::Builder::new()
            .name("youtube-player".into())
            .spawn(move || Worker::new(output, volume, opener, notify, loopback).run(rx))
            .expect("youtube player thread");
        Self { tx, thread: Some(thread), tap }
    }

    pub fn send(&self, command: YtCommand) {
        let _ = self.tx.send(Input::Command(command));
    }

    /// The level tap for the cover's breathing.
    pub fn tap(&self) -> Arc<AudioTap> {
        self.tap.clone()
    }
}

impl Drop for YtPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Input::Command(YtCommand::Shutdown));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A load waiting for its stream to open.
struct Pending {
    seq: u64,
    play: bool,
    seek: Option<u64>,
}

/// A seek running on its helper thread (it may wait for the download), which holds the decoder meanwhile.
struct Seeking {
    id: u64,
    /// play once it lands (Play and Pause while it runs change this)
    play: bool,
    /// a newer target that arrived while it ran
    next: Option<u64>,
}

struct Worker {
    output: Output,
    sink: Option<Box<dyn Sink>>,
    started: bool,
    volume: Arc<AtomicU16>,
    opener: Opener,
    notify: Notify,
    loopback: Sender<Input>,
    converter: Converter,
    seq: u64,
    pending: Option<Pending>,
    seeking: Option<Seeking>,
    seek_id: u64,
    decoder: Option<Decoder>,
    playing: bool,
    gain: f64,
    last_report: u64,
}

impl Worker {
    fn new(output: Output, volume: Arc<AtomicU16>, opener: Opener, notify: Notify, loopback: Sender<Input>) -> Self {
        Self {
            output,
            sink: None,
            started: false,
            volume,
            opener,
            notify,
            loopback,
            converter: Converter::new(None),
            seq: 0,
            pending: None,
            seeking: None,
            seek_id: 0,
            decoder: None,
            playing: false,
            gain: 1.0,
            last_report: 0,
        }
    }

    fn emit(&self, event: YtEvent) {
        (self.notify)(self.seq, event);
    }

    fn run(mut self, rx: Receiver<Input>) {
        loop {
            let busy = self.playing && self.decoder.is_some();
            let input = if busy {
                match rx.try_recv() {
                    Ok(input) => Some(input),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            } else {
                match rx.recv_timeout(Duration::from_secs(3600)) {
                    Ok(input) => Some(input),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            };
            match input {
                Some(Input::Command(YtCommand::Shutdown)) => break,
                Some(Input::Command(command)) => self.command(command),
                Some(Input::Opened { seq, result }) => self.opened(seq, result),
                Some(Input::Seeked { id, decoder, result }) => self.seeked(id, decoder, result),
                None => self.pump(),
            }
        }
        self.halt();
    }

    fn command(&mut self, command: YtCommand) {
        match command {
            YtCommand::Load { seq, stream, position_ms, play } => self.load(seq, stream, position_ms, play),
            YtCommand::Play => {
                if let Some(pending) = &mut self.pending {
                    pending.play = true;
                } else if let Some(seeking) = &mut self.seeking {
                    seeking.play = true;
                } else if self.decoder.is_some() && !self.playing {
                    self.resume();
                }
            }
            YtCommand::Pause => {
                if let Some(pending) = &mut self.pending {
                    pending.play = false;
                } else if let Some(seeking) = &mut self.seeking {
                    seeking.play = false;
                } else if let Some(decoder) = &self.decoder
                    && self.playing
                {
                    let position_ms = self.heard(decoder.position_ms());
                    self.playing = false;
                    self.stop_sink();
                    self.emit(YtEvent::Paused { position_ms });
                }
            }
            YtCommand::Seek(ms) => {
                if let Some(pending) = &mut self.pending {
                    pending.seek = Some(ms);
                } else if let Some(seeking) = &mut self.seeking {
                    // the last target wins once the running seek comes back
                    seeking.next = Some(ms);
                } else if let Some(decoder) = self.decoder.take() {
                    self.cut();
                    let play = self.playing;
                    self.playing = false;
                    self.start_seek(decoder, ms, play);
                }
            }
            YtCommand::Stop => {
                self.pending = None;
                self.seeking = None;
                self.decoder = None;
                self.playing = false;
                self.cut();
                self.stop_sink();
            }
            YtCommand::Volume(volume) => self.volume.store(volume, Ordering::Relaxed),
            YtCommand::Shutdown => {}
        }
    }

    fn load(&mut self, seq: u64, stream: Box<YtStream>, position_ms: u64, play: bool) {
        // the old track is cut off at once
        self.cut();
        self.decoder = None;
        self.seeking = None;
        self.playing = false;
        self.seq = seq;
        self.gain = if self.output.normalise { normalisation_factor(stream.loudness_db) } else { 1.0 };
        self.output.normalisation.store(self.gain.to_bits(), Ordering::Relaxed);
        self.pending = Some(Pending { seq, play, seek: None });
        self.emit(YtEvent::Loading);
        let opener = self.opener.clone();
        let loopback = self.loopback.clone();
        let spawned = std::thread::Builder::new().name("youtube-open".into()).spawn(move || {
            let result = opener(&stream).and_then(|source| Decoder::open(source, &stream.mime)).and_then(|mut decoder| {
                if position_ms > 0 {
                    decoder.seek(position_ms)?;
                }
                Ok(decoder)
            });
            let _ = loopback.send(Input::Opened { seq, result });
        });
        if let Err(error) = spawned {
            self.pending = None;
            self.fail(format!("couldn't start a thread: {error}"));
        }
    }

    /// Seeks on a helper thread: the bytes at the target may still be on their way, and commands must not wait
    /// for them.
    fn start_seek(&mut self, mut decoder: Decoder, ms: u64, play: bool) {
        self.seek_id += 1;
        let id = self.seek_id;
        self.seeking = Some(Seeking { id, play, next: None });
        let loopback = self.loopback.clone();
        let spawned = std::thread::Builder::new().name("youtube-seek".into()).spawn(move || {
            let result = decoder.seek(ms);
            let _ = loopback.send(Input::Seeked { id, decoder, result });
        });
        if let Err(error) = spawned {
            self.seeking = None;
            self.fail(format!("couldn't start a thread: {error}"));
        }
    }

    fn seeked(&mut self, id: u64, decoder: Decoder, result: Result<u64, String>) {
        // a seek of a track that was stopped or replaced meanwhile: dropping its decoder stops the download
        let Some(seeking) = self.seeking.take_if(|seeking| seeking.id == id) else { return };
        if let Some(ms) = seeking.next {
            return self.start_seek(decoder, ms, seeking.play);
        }
        match result {
            Ok(position_ms) => {
                self.last_report = position_ms;
                self.decoder = Some(decoder);
                if seeking.play {
                    self.resume();
                } else {
                    self.stop_sink();
                    self.emit(YtEvent::Paused { position_ms });
                }
            }
            Err(error) => self.fail(error),
        }
    }

    fn opened(&mut self, seq: u64, result: Result<Decoder, String>) {
        if self.pending.as_ref().is_none_or(|pending| pending.seq != seq) {
            // a superseded load; dropping it stops its download
            return;
        }
        let pending = self.pending.take().expect("pending load");
        let mut decoder = match result {
            Ok(decoder) => decoder,
            Err(error) => return self.fail(error),
        };
        if let Some(ms) = pending.seek
            && let Err(error) = decoder.seek(ms)
        {
            return self.fail(error);
        }
        let position_ms = decoder.position_ms();
        self.last_report = position_ms;
        self.decoder = Some(decoder);
        if pending.play {
            self.resume();
        } else {
            self.emit(YtEvent::Paused { position_ms });
        }
    }

    fn resume(&mut self) {
        let position_ms = self.decoder.as_ref().map_or(0, |decoder| decoder.position_ms());
        if let Err(error) = self.start_sink() {
            return self.fail(error);
        }
        self.playing = true;
        self.emit(YtEvent::Playing { position_ms });
    }

    /// Decodes one chunk and writes it out.
    fn pump(&mut self) {
        let Some(decoder) = &mut self.decoder else { return };
        match decoder.next_chunk() {
            Ok(Some(mut samples)) => {
                let position = decoder.position_ms();
                if self.gain != 1.0 {
                    for sample in &mut samples {
                        *sample *= self.gain;
                    }
                }
                let sink = self.sink.as_mut().expect("sink is open while playing");
                if let Err(error) = sink.write(AudioPacket::Samples(samples), &mut self.converter) {
                    return self.fail(error.to_string());
                }
                let heard = self.heard(position);
                if heard >= self.last_report + POSITION_EVERY_MS {
                    self.last_report = heard;
                    self.emit(YtEvent::Position(heard));
                }
            }
            Ok(None) => {
                self.decoder = None;
                self.playing = false;
                // plays out what is queued
                self.stop_sink();
                self.emit(YtEvent::Ended);
            }
            Err(error) => self.fail(error),
        }
    }

    /// The decoder's position less what the output still holds.
    fn heard(&self, decoded_ms: u64) -> u64 {
        decoded_ms.saturating_sub(self.output.latency_ms)
    }

    fn fail(&mut self, error: String) {
        log::warn!("youtube player: {error}");
        self.decoder = None;
        self.seeking = None;
        self.playing = false;
        self.cut();
        self.stop_sink();
        self.emit(YtEvent::Error(error));
    }

    fn start_sink(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        if self.sink.is_none() {
            let make = self.output.make_sink.take().ok_or_else(|| "no output".to_string())?;
            self.sink = Some(make());
        }
        let sink = self.sink.as_mut().expect("sink");
        sink.start().map_err(|error| error.to_string())?;
        self.started = true;
        Ok(())
    }

    fn stop_sink(&mut self) {
        if self.started
            && let Some(sink) = &mut self.sink
            && let Err(error) = sink.stop()
        {
            log::debug!("youtube output stop: {error}");
        }
        self.started = false;
    }

    /// Drops what the output has queued (a skip, a seek): fades it and opens the gate for new audio.
    fn cut(&self) {
        if self.started
            && let Some(control) = &self.output.control
        {
            control.interrupt();
            control.track_changed();
        }
    }

    fn halt(&mut self) {
        self.pending = None;
        self.seeking = None;
        self.decoder = None;
        self.playing = false;
        self.stop_sink();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::youtube::decode::tests::fixture;
    use librespot_playback::audio_backend::SinkResult;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorded {
        samples: Vec<f64>,
        starts: usize,
        stops: usize,
    }

    struct RecordingSink(Arc<Mutex<Recorded>>);

    impl Sink for RecordingSink {
        fn start(&mut self) -> SinkResult<()> {
            self.0.lock().unwrap().starts += 1;
            Ok(())
        }

        fn stop(&mut self) -> SinkResult<()> {
            self.0.lock().unwrap().stops += 1;
            Ok(())
        }

        fn write(&mut self, packet: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            if let AudioPacket::Samples(samples) = packet {
                self.0.lock().unwrap().samples.extend(samples);
            }
            Ok(())
        }
    }

    struct Harness {
        player: YtPlayer,
        recorded: Arc<Mutex<Recorded>>,
        events: Receiver<(u64, YtEvent)>,
    }

    fn harness(normalise: bool) -> Harness {
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let sink = recorded.clone();
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let notify: Notify = Arc::new(move |seq, event| {
            let _ = tx.lock().unwrap().send((seq, event));
        });
        let opener: Opener = Arc::new(|stream: &YtStream| match stream.url.as_str() {
            "broken" => Err("no such stream".into()),
            name => Ok(Box::new(std::io::Cursor::new(fixture(name))) as Box<dyn MediaSource>),
        });
        let player =
            YtPlayer::with_sink(Box::new(move || Box::new(RecordingSink(sink)) as Box<dyn Sink>), normalise, opener, notify);
        Harness { player, recorded, events }
    }

    fn stream(url: &str) -> YtStream {
        YtStream { url: url.into(), ..Default::default() }
    }

    impl Harness {
        /// Events up to and including the first that `stop` accepts.
        fn until(&self, stop: impl Fn(&(u64, YtEvent)) -> bool) -> Vec<(u64, YtEvent)> {
            let mut seen = Vec::new();
            loop {
                let event = self.events.recv_timeout(Duration::from_secs(10)).expect("an event in time");
                let done = stop(&event);
                seen.push(event);
                if done {
                    return seen;
                }
            }
        }
    }

    #[test]
    fn plays_a_track_to_the_end() {
        let h = harness(false);
        h.player.send(YtCommand::Load { seq: 1, stream: Box::new(stream("tone_44k_stereo.m4a")), position_ms: 0, play: true });
        let events = h.until(|(_, event)| *event == YtEvent::Ended);
        assert_eq!(events[0], (1, YtEvent::Loading));
        assert_eq!(events[1], (1, YtEvent::Playing { position_ms: 0 }));
        assert!(events.iter().any(|(_, event)| matches!(event, YtEvent::Position(_))));
        assert!(events.iter().all(|(seq, _)| *seq == 1));
        let recorded = h.recorded.lock().unwrap();
        assert!((86_000..=91_000).contains(&(recorded.samples.len() / 2)));
        assert_eq!((recorded.starts, recorded.stops), (1, 1));
    }

    #[test]
    fn loads_paused_then_seeks_and_plays() {
        let h = harness(false);
        h.player.send(YtCommand::Load { seq: 7, stream: Box::new(stream("tone_44k_stereo.m4a")), position_ms: 0, play: false });
        let events = h.until(|(_, event)| matches!(event, YtEvent::Paused { .. }));
        assert_eq!(events.last(), Some(&(7, YtEvent::Paused { position_ms: 0 })));
        h.player.send(YtCommand::Seek(1_500));
        let (_, event) = h.until(|_| true).remove(0);
        let YtEvent::Paused { position_ms } = event else { panic!("{event:?}") };
        assert!(position_ms.abs_diff(1_500) <= 30);
        assert!(h.recorded.lock().unwrap().samples.is_empty(), "nothing is written while paused");
        h.player.send(YtCommand::Play);
        h.until(|(_, event)| *event == YtEvent::Ended);
        let frames = h.recorded.lock().unwrap().samples.len() / 2;
        assert!((20_000..=25_000).contains(&frames), "the last half second, got {frames}");
    }

    #[test]
    fn the_last_of_several_seeks_wins_and_play_survives_them() {
        let h = harness(false);
        h.player.send(YtCommand::Load { seq: 4, stream: Box::new(stream("tone_44k_stereo.m4a")), position_ms: 0, play: false });
        h.until(|(_, event)| matches!(event, YtEvent::Paused { .. }));
        h.player.send(YtCommand::Seek(200));
        h.player.send(YtCommand::Seek(1_500));
        // may arrive while a seek still runs
        h.player.send(YtCommand::Play);
        let events = h.until(|(_, event)| *event == YtEvent::Ended);
        assert!(events.iter().any(|(_, event)| matches!(event, YtEvent::Playing { position_ms } if position_ms.abs_diff(1_500) <= 30)), "{events:?}");
        let frames = h.recorded.lock().unwrap().samples.len() / 2;
        assert!((20_000..=25_000).contains(&frames), "the last half second, got {frames}");
    }

    #[test]
    fn a_stop_during_a_seek_stays_stopped() {
        let h = harness(false);
        h.player.send(YtCommand::Load { seq: 5, stream: Box::new(stream("tone_44k_stereo.m4a")), position_ms: 0, play: false });
        h.until(|(_, event)| matches!(event, YtEvent::Paused { .. }));
        h.player.send(YtCommand::Seek(500));
        h.player.send(YtCommand::Stop);
        h.player.send(YtCommand::Play);
        // the seek may come back before or after the stop; either way nothing plays
        std::thread::sleep(Duration::from_millis(400));
        let events: Vec<_> = h.events.try_iter().collect();
        assert!(!events.iter().any(|(_, event)| matches!(event, YtEvent::Playing { .. } | YtEvent::Ended)), "{events:?}");
        assert!(h.recorded.lock().unwrap().samples.is_empty());
    }

    #[test]
    fn a_new_load_supersedes_the_old_one() {
        let h = harness(false);
        h.player.send(YtCommand::Load { seq: 1, stream: Box::new(stream("tone_44k_stereo.m4a")), position_ms: 0, play: true });
        h.player.send(YtCommand::Load { seq: 2, stream: Box::new(stream("tone_48k_mono.m4a")), position_ms: 0, play: true });
        let events = h.until(|(seq, event)| *seq == 2 && *event == YtEvent::Ended);
        assert!(!events.iter().any(|(seq, event)| *seq == 1 && *event == YtEvent::Ended));
        assert!(events.contains(&(2, YtEvent::Playing { position_ms: 0 })));
    }

    #[test]
    fn open_failures_are_reported() {
        let h = harness(false);
        h.player.send(YtCommand::Load { seq: 3, stream: Box::new(stream("broken")), position_ms: 0, play: true });
        let events = h.until(|(_, event)| matches!(event, YtEvent::Error(_)));
        assert_eq!(events.last(), Some(&(3, YtEvent::Error("no such stream".into()))));
    }

    #[test]
    fn normalisation_only_attenuates() {
        assert_eq!(normalisation_factor(None), 1.0);
        assert_eq!(normalisation_factor(Some(-3.0)), 1.0);
        assert!((normalisation_factor(Some(6.0)) - 0.501).abs() < 0.001);
        let loud = harness(true);
        let quiet = harness(false);
        let mut track = stream("tone_44k_stereo.m4a");
        track.loudness_db = Some(6.0);
        for h in [&loud, &quiet] {
            h.player.send(YtCommand::Load { seq: 1, stream: Box::new(track.clone()), position_ms: 0, play: true });
            h.until(|(_, event)| *event == YtEvent::Ended);
        }
        let peak = |h: &Harness| h.recorded.lock().unwrap().samples.iter().fold(0f64, |peak, sample| peak.max(sample.abs()));
        assert!((peak(&loud) / peak(&quiet) - 0.501).abs() < 0.01);
    }

    #[test]
    fn stop_and_pause() {
        let h = harness(false);
        h.player.send(YtCommand::Load { seq: 1, stream: Box::new(stream("tone_44k_stereo.m4a")), position_ms: 0, play: true });
        h.player.send(YtCommand::Pause);
        let events = h.until(|(_, event)| matches!(event, YtEvent::Paused { .. }));
        assert!(!events.iter().any(|(_, event)| matches!(event, YtEvent::Playing { .. })), "paused before it opened");
        h.player.send(YtCommand::Stop);
        h.player.send(YtCommand::Play);
        assert!(h.events.recv_timeout(Duration::from_millis(300)).is_err(), "nothing to play after stop");
    }

    #[test]
    fn local_files_are_read_from_disk() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/youtube/tone_48k_mono.m4a");
        let local = YtStream { url: format!("{}{}", super::super::LOCAL_FILE_URL, path.display()), ..Default::default() };
        assert!(open_local(&local).expect("a local stream").is_ok());
        assert!(open_local(&stream("https://example.invalid/a")).is_none());
        let missing = YtStream { url: format!("{}{}", super::super::LOCAL_FILE_URL, path.with_extension("gone").display()), ..Default::default() };
        assert!(open_local(&missing).expect("a local stream").is_err());

        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let sink = recorded.clone();
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let notify: Notify = Arc::new(move |seq, event| {
            let _ = tx.lock().unwrap().send((seq, event));
        });
        let opener: Opener = Arc::new(|stream: &YtStream| open_local(stream).unwrap_or_else(|| Err("not local".into())));
        let player =
            YtPlayer::with_sink(Box::new(move || Box::new(RecordingSink(sink)) as Box<dyn Sink>), false, opener, notify);
        let h = Harness { player, recorded, events };
        h.player.send(YtCommand::Load { seq: 1, stream: Box::new(local), position_ms: 0, play: true });
        h.until(|(_, event)| *event == YtEvent::Ended);
        let frames = h.recorded.lock().unwrap().samples.len() / 2;
        assert!((42_000..=46_000).contains(&frames), "one second at 44.1 kHz, got {frames}");
    }

    #[test]
    fn local_formats_decode_by_their_mime() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/local");
        for (name, mime) in [("tagged.flac", "audio/flac"), ("tagged.mp3", "audio/mpeg"), ("tagged.ogg", "audio/ogg"), ("untagged.wav", "audio/wav")] {
            let stream = YtStream { url: format!("{}{}", super::super::LOCAL_FILE_URL, dir.join(name).display()), mime: mime.into(), ..Default::default() };
            let source = open_local(&stream).unwrap().unwrap();
            let mut decoder = Decoder::open(source, mime).unwrap_or_else(|error| panic!("{name}: {error}"));
            let mut frames = 0;
            while let Some(chunk) = decoder.next_chunk().unwrap_or_else(|error| panic!("{name}: {error}")) {
                frames += chunk.len() / 2;
            }
            assert!((40_000..=48_000).contains(&frames), "{name}: about one second at 44.1 kHz, got {frames}");
        }
    }
}
