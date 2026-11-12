// Extras — desktop features ported from soundwave1: sleep timer (sf_player::sleep over Services.sleep), equalizer
// settings (eqEnabled/eqPreamp/eqBands/eqPreset/audioBalance/monoAudio; glue applies them to the shared EQ of
// both lanes), listening stats (Services.stats), settings backup/restore and M3U playlist export
// (sf_services::transfer). File work runs on the blocking pool; `transferFinished` reports the outcome.
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use md3_model::PlaylistId;
use mf_platform::settings::{eq_bands_from_str, eq_bands_to_string};
use mf_platform::{SettingKey, SettingValue};
use sf_player::engine::eq::{BANDS, PRESETS, preset};
use sf_player::sleep::SleepStatus;
use sf_services::stats::{StatsEvent, StatsRange, format_time};
use sf_services::transfer::{Backup, export_m3u, m3u};

use super::{Delivery, forward_events, forward_watch, local_path, to_qlist, to_qmap};
use crate::context::ctx;
use crate::qt::{QVariantList, QVariantMap};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
        include!("cxx-qt-lib/qmap.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        // --- sleep timer ---
        #[qproperty(bool, sleep_active, cxx_name = "sleepActive", READ, NOTIFY = sleep_changed)]
        #[qproperty(bool, sleep_end_of_track, cxx_name = "sleepEndOfTrack", READ, NOTIFY = sleep_changed)]
        /// seconds left in the timed mode, 0 otherwise
        #[qproperty(i32, sleep_remaining, cxx_name = "sleepRemaining", READ, NOTIFY = sleep_changed)]
        // --- equalizer ---
        #[qproperty(bool, eq_enabled, cxx_name = "eqEnabled", READ, WRITE = set_eq_enabled, NOTIFY = eq_changed)]
        #[qproperty(f64, eq_preamp, cxx_name = "eqPreamp", READ, WRITE = set_eq_preamp, NOTIFY = eq_changed)]
        /// ten gains in dB
        #[qproperty(QList_QVariant, eq_bands, cxx_name = "eqBands", READ, NOTIFY = eq_changed)]
        /// the preset the bands came from; "" — custom
        #[qproperty(QString, eq_preset, cxx_name = "eqPreset", READ, NOTIFY = eq_changed)]
        #[qproperty(f64, audio_balance, cxx_name = "audioBalance", READ, WRITE = set_audio_balance, NOTIFY = eq_changed)]
        #[qproperty(bool, mono_audio, cxx_name = "monoAudio", READ, WRITE = set_mono_audio, NOTIFY = eq_changed)]
        #[qproperty(QStringList, eq_preset_names, cxx_name = "eqPresetNames", READ, CONSTANT)]
        /// band labels: "60", "170", … "1k", "16k"
        #[qproperty(QStringList, eq_frequencies, cxx_name = "eqFrequencies", READ, CONSTANT)]
        // --- file work ---
        #[qproperty(bool, busy, READ, NOTIFY = busy_changed)]
        type Extras = super::ExtrasRust;

        fn set_eq_enabled(self: Pin<&mut Extras>, value: bool);
        fn set_eq_preamp(self: Pin<&mut Extras>, value: f64);
        fn set_audio_balance(self: Pin<&mut Extras>, value: f64);
        fn set_mono_audio(self: Pin<&mut Extras>, value: bool);

        #[qsignal]
        #[cxx_name = "sleepChanged"]
        fn sleep_changed(self: Pin<&mut Extras>);
        /// the timer paused playback (for a toast)
        #[qsignal]
        #[cxx_name = "sleepFired"]
        fn sleep_fired(self: Pin<&mut Extras>);
        #[qsignal]
        #[cxx_name = "eqChanged"]
        fn eq_changed(self: Pin<&mut Extras>);
        /// a new listen was counted: call stats() again
        #[qsignal]
        #[cxx_name = "statsChanged"]
        fn stats_changed(self: Pin<&mut Extras>);
        #[qsignal]
        #[cxx_name = "busyChanged"]
        fn busy_changed(self: Pin<&mut Extras>);
        /// backup, restore or export finished; `message` is ready for a toast
        #[qsignal]
        #[cxx_name = "transferFinished"]
        fn transfer_finished(self: Pin<&mut Extras>, ok: bool, message: QString);

        /// Pause in `minutes` (0 — off).
        #[qinvokable]
        #[cxx_name = "setSleepMinutes"]
        fn set_sleep_minutes(self: &Extras, minutes: i32);
        /// Pause when the track playing now ends.
        #[qinvokable]
        #[cxx_name = "sleepAtEndOfTrack"]
        fn sleep_at_end_of_track(self: &Extras);
        #[qinvokable]
        #[cxx_name = "cancelSleep"]
        fn cancel_sleep(self: &Extras);

        #[qinvokable]
        #[cxx_name = "setBand"]
        fn set_band(self: &Extras, index: i32, db: f64);
        #[qinvokable]
        #[cxx_name = "applyPreset"]
        fn apply_preset(self: &Extras, name: &QString);
        /// Flat bands, no preamp, centred balance, stereo; the switch stays as it is.
        #[qinvokable]
        #[cxx_name = "resetEq"]
        fn reset_eq(self: &Extras);

        /// {totalSeconds, plays, averagePerDaySeconds, distinctTracks, distinctArtists, spotifySeconds,
        /// youtubeSeconds, topTracks: [{id, title, subtitle, cover, plays, seconds}], topArtists, buckets:
        /// [{label, seconds}]} for "week" | "month" | "year" | "all".
        #[qinvokable]
        fn stats(self: &Extras, range: &QString) -> QMap_QString_QVariant;
        /// "5 h 22 m", "44 m", "38 s"
        #[qinvokable]
        #[cxx_name = "formatTime"]
        fn format_time(self: &Extras, seconds: f64) -> QString;
        #[qinvokable]
        #[cxx_name = "clearStats"]
        fn clear_stats(self: &Extras);

        /// Write a backup to a path or file:// URL (from FileDialog).
        #[qinvokable]
        #[cxx_name = "exportBackup"]
        fn export_backup(self: Pin<&mut Extras>, file: &QString);
        /// Restore a backup.
        #[qinvokable]
        #[cxx_name = "importBackup"]
        fn import_backup(self: Pin<&mut Extras>, file: &QString);
        /// "materialfy-backup-2026-11-12.json"
        #[qinvokable]
        #[cxx_name = "backupFileName"]
        fn backup_file_name(self: &Extras) -> QString;

        /// Export a loaded playlist (Library.playlistTracks was opened for it) as M3U.
        #[qinvokable]
        #[cxx_name = "exportPlaylistM3u"]
        fn export_playlist_m3u(self: Pin<&mut Extras>, playlist_id: &QString, title: &QString, file: &QString);
        /// A safe file name for the playlist: "Road _ Trip.m3u8"
        #[qinvokable]
        #[cxx_name = "m3uFileName"]
        fn m3u_file_name(self: &Extras, title: &QString) -> QString;
    }

    impl cxx_qt::Threading for Extras {}
    impl cxx_qt::Initialize for Extras {}
}

pub struct ExtrasRust {
    sleep_active: bool,
    sleep_end_of_track: bool,
    sleep_remaining: i32,
    eq_enabled: bool,
    eq_preamp: f64,
    eq_bands: QVariantList,
    eq_preset: QString,
    audio_balance: f64,
    mono_audio: bool,
    eq_preset_names: QStringList,
    eq_frequencies: QStringList,
    busy: bool,
}

fn frequency_label(hz: f32) -> String {
    if hz >= 1000.0 { format!("{}k", hz / 1000.0) } else { format!("{hz}") }
}

impl Default for ExtrasRust {
    fn default() -> Self {
        Self {
            sleep_active: false,
            sleep_end_of_track: false,
            sleep_remaining: 0,
            eq_enabled: false,
            eq_preamp: 0.0,
            eq_bands: QVariantList::default(),
            eq_preset: QString::default(),
            audio_balance: 0.0,
            mono_audio: false,
            eq_preset_names: PRESETS.iter().map(|p| QString::from(p.name)).collect(),
            eq_frequencies: BANDS.iter().map(|hz| QString::from(frequency_label(*hz).as_str())).collect(),
            busy: false,
        }
    }
}

fn is_eq_key(key: SettingKey) -> bool {
    matches!(
        key,
        SettingKey::EqEnabled
            | SettingKey::EqPreamp
            | SettingKey::EqBands
            | SettingKey::EqPreset
            | SettingKey::AudioBalance
            | SettingKey::MonoAudio
    )
}

impl qobject::Extras {
    // --- sleep timer ---

    fn set_sleep_minutes(&self, minutes: i32) {
        ctx().services.sleep.set_minutes(minutes.max(0) as u32);
    }

    fn sleep_at_end_of_track(&self) {
        ctx().services.sleep.set_end_of_track();
    }

    fn cancel_sleep(&self) {
        ctx().services.sleep.cancel();
    }

    fn apply_sleep(mut self: Pin<&mut Self>, status: SleepStatus) {
        let remaining = status.remaining_ms.div_ceil(1000).min(i32::MAX as u64) as i32;
        let changed = {
            let this = self.rust();
            this.sleep_active != status.active
                || this.sleep_end_of_track != status.end_of_track
                || this.sleep_remaining != remaining
        };
        if changed {
            let mut this = self.as_mut().rust_mut();
            this.sleep_active = status.active;
            this.sleep_end_of_track = status.end_of_track;
            this.sleep_remaining = remaining;
            self.as_mut().sleep_changed();
        }
        if status.fired {
            self.sleep_fired();
        }
    }

    // --- equalizer ---

    fn set_eq_enabled(self: Pin<&mut Self>, value: bool) {
        ctx().services.settings.set(SettingKey::EqEnabled, SettingValue::Bool(value));
    }

    fn set_eq_preamp(self: Pin<&mut Self>, value: f64) {
        ctx().services.settings.set(SettingKey::EqPreamp, SettingValue::Float((value * 10.0).round() / 10.0));
    }

    fn set_audio_balance(self: Pin<&mut Self>, value: f64) {
        // a little stickiness at the centre
        let value = if value.abs() < 0.03 { 0.0 } else { (value * 100.0).round() / 100.0 };
        ctx().services.settings.set(SettingKey::AudioBalance, SettingValue::Float(value));
    }

    fn set_mono_audio(self: Pin<&mut Self>, value: bool) {
        ctx().services.settings.set(SettingKey::MonoAudio, SettingValue::Bool(value));
    }

    fn set_band(&self, index: i32, db: f64) {
        let settings = &ctx().services.settings;
        let Ok(index) = usize::try_from(index) else { return };
        let mut bands = eq_bands_from_str(&settings.get_str(SettingKey::EqBands));
        let Some(band) = bands.get_mut(index) else { return };
        *band = (db * 10.0).round() / 10.0;
        settings.set(SettingKey::EqBands, SettingValue::Str(eq_bands_to_string(&bands)));
        // moving a slider makes the curve custom
        settings.set(SettingKey::EqPreset, SettingValue::Str(String::new()));
    }

    fn apply_preset(&self, name: &QString) {
        let Some(found) = preset(&name.to_string()) else { return };
        let bands: Vec<f64> = found.bands_db.iter().map(|db| f64::from(*db)).collect();
        let settings = &ctx().services.settings;
        settings.set(SettingKey::EqBands, SettingValue::Str(eq_bands_to_string(&bands)));
        settings.set(SettingKey::EqPreset, SettingValue::Str(found.name.to_owned()));
        // choosing a curve means wanting to hear it
        settings.set(SettingKey::EqEnabled, SettingValue::Bool(true));
    }

    fn reset_eq(&self) {
        let settings = &ctx().services.settings;
        settings.set(SettingKey::EqBands, SettingValue::Str(String::new()));
        settings.set(SettingKey::EqPreset, SettingValue::Str(String::new()));
        settings.set(SettingKey::EqPreamp, SettingValue::Float(0.0));
        settings.set(SettingKey::AudioBalance, SettingValue::Float(0.0));
        settings.set(SettingKey::MonoAudio, SettingValue::Bool(false));
    }

    fn load_eq(mut self: Pin<&mut Self>) {
        let settings = &ctx().services.settings;
        let bands = eq_bands_from_str(&settings.get_str(SettingKey::EqBands));
        let mut this = self.as_mut().rust_mut();
        this.eq_enabled = settings.get_bool(SettingKey::EqEnabled);
        this.eq_preamp = settings.get_f64(SettingKey::EqPreamp);
        this.eq_bands = to_qlist(&bands);
        this.eq_preset = QString::from(settings.get_str(SettingKey::EqPreset).as_str());
        this.audio_balance = settings.get_f64(SettingKey::AudioBalance);
        this.mono_audio = settings.get_bool(SettingKey::MonoAudio);
        self.eq_changed();
    }

    fn on_setting(self: Pin<&mut Self>, delivery: Delivery<SettingKey>) {
        let relevant = match delivery {
            Delivery::Event(key) => is_eq_key(key),
            Delivery::Lagged => true,
        };
        if relevant {
            self.load_eq();
        }
    }

    // --- stats ---

    fn stats(&self, range: &QString) -> QVariantMap {
        to_qmap(&ctx().services.stats.summary(StatsRange::from_name(&range.to_string())))
    }

    fn format_time(&self, seconds: f64) -> QString {
        QString::from(format_time(seconds.max(0.0) as u64).as_str())
    }

    fn clear_stats(&self) {
        ctx().services.stats.clear();
    }

    fn on_stats(self: Pin<&mut Self>, _delivery: Delivery<StatsEvent>) {
        self.stats_changed();
    }

    // --- file work ---

    fn set_busy(mut self: Pin<&mut Self>, value: bool) {
        if self.rust().busy != value {
            self.as_mut().rust_mut().busy = value;
            self.busy_changed();
        }
    }

    /// Run `work` on the blocking pool; its (ok, message) comes back as transferFinished.
    fn run_transfer(mut self: Pin<&mut Self>, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
        if self.rust().busy {
            return;
        }
        self.as_mut().set_busy(true);
        let thread = self.qt_thread();
        ctx().rt.spawn_blocking(move || {
            let (ok, message) = match work() {
                Ok(message) => (true, message),
                Err(message) => (false, message),
            };
            let _ = thread.queue(move |mut object| {
                object.as_mut().set_busy(false);
                object.transfer_finished(ok, QString::from(message.as_str()));
            });
        });
    }

    fn export_backup(self: Pin<&mut Self>, file: &QString) {
        let path = local_path(file);
        self.run_transfer(move || {
            let services = &ctx().services;
            let backup = Backup::build(&services.settings, services.history.local_plays(), services.stats.plays());
            backup.write(std::path::Path::new(&path))?;
            Ok(mf_i18n::t("Backup saved"))
        });
    }

    fn import_backup(self: Pin<&mut Self>, file: &QString) {
        let path = local_path(file);
        self.run_transfer(move || {
            let services = &ctx().services;
            let backup = Backup::read(std::path::Path::new(&path))?;
            let report = backup.apply_settings(&services.settings);
            if !backup.history.is_empty() {
                services.history.replace_local(backup.history);
            }
            if !backup.stats.is_empty() {
                services.stats.replace(backup.stats);
            }
            Ok(mf_i18n::tf("Restored {0} settings", &[&report.settings]))
        });
    }

    fn backup_file_name(&self) -> QString {
        QString::from(sf_services::transfer::backup::file_name().as_str())
    }

    fn export_playlist_m3u(self: Pin<&mut Self>, playlist_id: &QString, title: &QString, file: &QString) {
        let id = PlaylistId::from_uri_or_id(&playlist_id.to_string());
        let (title, path) = (title.to_string(), local_path(file));
        self.run_transfer(move || {
            let services = &ctx().services;
            let tracks = services
                .library
                .playlist_tracks(&id)
                .filter(|tracks| !tracks.is_empty())
                .ok_or_else(|| mf_i18n::t("The playlist hasn’t loaded yet"))?;
            let local = services.local.clone();
            let export = export_m3u(&title, &tracks, |track| local.track(&track.uri).map(|t| t.path));
            m3u::write(std::path::Path::new(&path), &export)?;
            Ok(if export.skipped > 0 {
                mf_i18n::tf("Exported {0} tracks, skipped {1}", &[&export.written, &export.skipped])
            } else {
                mf_i18n::tf("Exported {0} tracks", &[&export.written])
            })
        });
    }

    fn m3u_file_name(&self, title: &QString) -> QString {
        QString::from(m3u::file_name(&title.to_string()).as_str())
    }
}

impl cxx_qt::Initialize for qobject::Extras {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().load_eq();
        let sleep = ctx().services.sleep.subscribe();
        self.as_mut().apply_sleep(sleep.borrow().clone());
        forward_watch(self.qt_thread(), sleep, Self::apply_sleep);
        forward_events(self.qt_thread(), ctx().services.settings.subscribe(), Self::on_setting);
        forward_events(self.qt_thread(), ctx().services.stats.subscribe(), Self::on_stats);
    }
}
