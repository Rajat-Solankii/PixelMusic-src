//! Связки без QML (исполнитель «services-extra»): настройки → плеер (autoplay), сборка EngineConfig из настроек,
//! медиаклавиши и системная панель «Сейчас играет» через fastframe-now-playing: Windows — SMTC (свой поток и
//! скрытое окно, HWND Qt не нужен), macOS — Now Playing, Linux — MPRIS org.mpris.MediaPlayer2.materialfy.
use std::sync::Arc;
use std::time::Duration;

use sf_auth::ProxyConfig;
use sf_player::{EngineConfig, PlayerHandle};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use mf_platform::{SettingKey, SettingsStore};

use crate::AppRequest;

/// «Живое» из настроек → плеер: Autoplay, скорость и кроссфейд (последние два вывод звука читает сам на каждом
/// пакете). Громкость в плеер передаёт только мост Player (начальная — PlayerConfig.volume); параметры движка
/// не трогает (флаг перезапуска ведёт AuthService).
pub fn spawn_settings_glue(rt: &Handle, settings: Arc<SettingsStore>, player: PlayerHandle) {
    apply_audio_fx(&settings);
    apply_eq(&settings);
    let mut changes = settings.subscribe();
    rt.spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SettingKey::Autoplay) => player.set_autoplay(settings.get_bool(SettingKey::Autoplay)),
                Ok(SettingKey::PlaybackSpeed | SettingKey::Crossfade | SettingKey::DjVoice) => apply_audio_fx(&settings),
                Ok(
                    SettingKey::EqEnabled
                    | SettingKey::EqPreamp
                    | SettingKey::EqBands
                    | SettingKey::AudioBalance
                    | SettingKey::MonoAudio,
                ) => apply_eq(&settings),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    player.set_autoplay(settings.get_bool(SettingKey::Autoplay));
                    apply_audio_fx(&settings);
                    apply_eq(&settings);
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

/// Скорость и кроссфейд из настроек → вывод звука.
fn apply_audio_fx(settings: &SettingsStore) {
    use sf_player::engine::fx;
    fx::set_speed(settings.get_f64(SettingKey::PlaybackSpeed) as f32);
    let seconds = settings.get_int(SettingKey::Crossfade).clamp(0, mf_platform::settings::CROSSFADE_MAX_S) as u64;
    fx::set_crossfade(Duration::from_secs(seconds));
    fx::set_dj_voice(settings.get_bool(SettingKey::DjVoice));
}

/// The one equalizer of the app: the librespot engine (every restart) and the YouTube lane share it, so the
/// Settings panel changes both at once.
pub fn shared_eq() -> sf_player::engine::eq::SharedEq {
    static EQ: std::sync::LazyLock<sf_player::engine::eq::SharedEq> =
        std::sync::LazyLock::new(sf_player::engine::eq::shared);
    Arc::clone(&EQ)
}

/// The stored equalizer settings, as the engine takes them.
pub fn eq_settings(settings: &SettingsStore) -> sf_player::engine::eq::EqSettings {
    let bands = mf_platform::settings::eq_bands_from_str(&settings.get_str(SettingKey::EqBands));
    sf_player::engine::eq::EqSettings::from_stored(
        settings.get_bool(SettingKey::EqEnabled),
        settings.get_f64(SettingKey::EqPreamp),
        &bands,
        settings.get_f64(SettingKey::AudioBalance),
        settings.get_bool(SettingKey::MonoAudio),
    )
}

/// Equalizer settings → the shared equalizer (the audio threads pick the change up on their next packet).
fn apply_eq(settings: &SettingsStore) {
    let next = eq_settings(settings);
    let eq = shared_eq();
    if *eq.lock() != next {
        *eq.lock() = next;
    }
}

/// EngineConfig из настроек: имя устройства (DeviceName или DevicesService::default_device_name), битрейт,
/// нормализация, autoplay, gapless, устройство вывода, кэш (AudioCache/AudioCacheMb → dirs::audio_cache_dir),
/// громкость, прокси. tap — новый AudioTap (PlayerHandle подменяет своим), eq — по умолчанию (EQ-UI нет).
pub fn engine_config(settings: &SettingsStore, proxy: ProxyConfig) -> EngineConfig {
    let volume = settings.get_f64(SettingKey::Volume).clamp(0.0, 1.0);
    let name = settings.get_str(SettingKey::DeviceName);
    let device_name = if name.trim().is_empty() { crate::devices::DevicesService::default_device_name() } else { name };
    let cache_mb = settings.get_int(SettingKey::AudioCacheMb).max(0) as u64;
    let device = settings.get_str(SettingKey::AudioDevice);
    EngineConfig {
        device_name,
        bitrate_kbps: match settings.get_int(SettingKey::Bitrate) {
            96 => 96,
            160 => 160,
            _ => 320,
        },
        normalisation: settings.get_bool(SettingKey::Normalisation),
        autoplay: settings.get_bool(SettingKey::Autoplay),
        gapless: settings.get_bool(SettingKey::Gapless),
        backend: None,
        audio_device: (!device.is_empty()).then_some(device),
        initial_volume: sf_player::volume_to_u16(volume),
        volume_dir: mf_platform::dirs::volume_dir(),
        audio_cache_dir: settings.get_bool(SettingKey::AudioCache).then(mf_platform::dirs::audio_cache_dir),
        audio_cache_limit: Some(cache_mb * 1024 * 1024),
        buffer_ms: sf_player::engine::sink::DEFAULT_BUFFER_MS,
        tap: sf_player::engine::vis::AudioTap::new(),
        eq: shared_eq(),
        proxy,
    }
}

/// Остановка медиаинтеграции при выходе.
pub struct MediaStop {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    media: crate::extra::media::Media,
}

impl MediaStop {
    /// Снять панель/шину и подождать поток не дольше `timeout`.
    pub fn stop(self, timeout: Duration) {
        #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
        self.media.stop(timeout);
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        let _ = timeout;
    }
}

/// Медиаклавиши/SMTC (Windows), MPRIS (Linux), Now Playing (macOS): состояние из PlayerSnapshot,
/// команды → PlayerHandle; Raise/Quit → AppRequest. None — не поддерживается/не запустилось.
pub fn start_media_controls(
    rt: &Handle,
    settings: Arc<SettingsStore>,
    player: PlayerHandle,
    requests: broadcast::Sender<AppRequest>,
) -> Option<MediaStop> {
    // громкость из панели идёт в плеер; в настройки её пишет мост Player по volumeChanged
    let _ = settings;
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        crate::extra::media::start(rt, player, requests).map(|media| MediaStop { media })
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (rt, player, requests);
        None
    }
}
