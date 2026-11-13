//! Сервисы приложения (без Qt) — фасад той же формы, что сервисы yamusic-md3 + devices, history, home.
//! Каждый сервис — Arc; состояние — watch (снимок) и/или broadcast (события). Мосты QML подписываются в
//! cxx_qt::Initialize и переносят изменения в GUI-поток через CxxQtThread::queue.
//!
//! Один tokio-рантайм на всё (создаёт оболочка, engine.rs): сервисы, PlayerHandle-актор, librespot
//! (Session/Spirc спавнят задачи на текущем рантайме) и задачи мостов. Блокирующее (keyring, zeroconf,
//! размеры кэша) — spawn_blocking/свои потоки. Вход НЕ начинается в start — оболочка зовёт auth.start()
//! после загрузки QML (если не --offline).

pub mod auth;
pub mod catalog;
pub mod cover_fallback;
/// внутреннее «services-core»
pub mod data;
pub mod devices;
/// Downloaded YouTube Music songs and playlists (queue, files, index), played from disk by the lane
pub mod downloads;
/// внутреннее «services-extra»
pub mod extra;
pub mod glue;
pub mod history;
pub mod home;
/// Same-Wi-Fi Jam (no account): HTTP server + UDP beacon, wire-compatible with the Kotlin apps
pub mod lanjam;
/// Last.fm scrobbling and "now playing" (session in the credential store, slot LastFm)
pub mod lastfm;
/// Last.fm feed (recent tracks, friends, top albums) and Smart Mix playlists
pub mod lastfm_feed;
pub mod library;
/// Music files on this computer: folder scan, tags and covers, played through the YouTube lane
pub mod local;
pub mod lyrics;
/// Saved past queues ("Replay queue")
pub mod queue_history;
/// «Новинки»: «Радар новинок» и свежие релизы исполнителей
pub mod releases;
/// Friends (friend activity) and Jam through the session
pub mod search_history;
pub mod social;
pub mod sources;
/// Listening statistics (top tracks, artists, minutes per period) from a local listen log
pub mod stats;
/// Desktop extras: settings backup/restore and M3U playlist export
pub mod transfer;
/// auto-update from GitHub Releases
pub mod update;
pub mod wave;
/// YouTube Music: InnerTube client, parsers, services, playlist import
pub mod youtube;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sf_api::ApiGateway;
use sf_player::{PlayerConfig, PlayerHandle};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use mf_platform::{SettingKey, SettingsStore};

pub use auth::{
    AuthEvent, AuthService, AuthSnapshot, AuthState, PlaybackAuthState, ProxyInput, YouTubeAuthEvent, YouTubeAuthSnapshot,
    YouTubeAuthState,
};
pub use sf_auth::{Grant, ManualProxy, ProxyConfig, ProxyMode};
pub use catalog::{AlbumPage, ArtistPage, CatalogService, EpisodePage, SearchKind, SearchResults, ShowPage};
pub use devices::{DevicesEvent, DevicesService, DevicesSnapshot};
pub use history::{HistoryEvent, HistoryService, PlayRecord};
pub use lastfm::{LastFmEvent, LastFmService, LastFmSnapshot};
pub use home::{HomeService, HomeSnapshot};
pub use library::{FeedbackKind, LibraryEvent, LibraryKind, LibraryService, PlaylistInfo};
pub use lyrics::{LyricsService, LyricsSnapshot, LyricsStatus};
pub use md3_model::{ListChange, PlaylistId};
pub use releases::{NewRelease, ReleasesService, ReleasesSnapshot};
pub use social::{FriendActivity, Jam, JamMember, SocialService, SocialSnapshot};
pub use update::{UpdateService, UpdateSnapshot, UpdateState};
pub use wave::{WaveService, WaveSnapshot};
pub use youtube::{YouTubeError, YouTubeService};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{}", mf_i18n::t("Sign in to your account"))]
    NotSignedIn,
    #[error("{}", data::route::api_error_text(.0))]
    Api(#[from] sf_api::ApiError),
    #[error("{0}")]
    Other(String),
}

/// Просьбы к оболочке Qt (медиаклавиши/MPRIS Raise/Quit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRequest {
    RaiseWindow,
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct ServicesOptions {
    /// сохранять/восстанавливать ручную очередь и точку возобновления (false в режиме снимков)
    pub persist_session: bool,
    /// медиаклавиши/SMTC/MPRIS (false в режиме снимков)
    pub enable_media: bool,
}

/// Все сервисы. Clone — дешёвый (Arc).
#[derive(Clone)]
pub struct Services {
    pub settings: Arc<SettingsStore>,
    pub api: Arc<ApiGateway>,
    pub auth: Arc<AuthService>,
    pub library: Arc<LibraryService>,
    pub catalog: Arc<CatalogService>,
    /// Радио Spotify (бывшая «Моя волна»)
    pub wave: Arc<WaveService>,
    pub home: Arc<HomeService>,
    /// «Новинки»
    pub releases: Arc<ReleasesService>,
    pub lyrics: Arc<LyricsService>,
    pub history: Arc<HistoryService>,
    pub devices: Arc<DevicesService>,
    /// friend activity and Jam
    pub social: Arc<SocialService>,
    /// same-Wi-Fi Jam (no account), beside Spotify's Jam in `social`
    pub lanjam: Arc<lanjam::LanJamService>,
    /// auto-update (checks start from the Updater bridge, not here)
    pub update: Arc<update::UpdateService>,
    /// YouTube Music (anonymous until signed in through `auth`); the same Arc as `auth.youtube()`, which
    /// library, catalog, home, wave, history and lyrics route `youtube:` items to
    pub youtube: Arc<YouTubeService>,
    /// music files on this computer (folders from the LocalFolders setting)
    pub local: Arc<local::LocalFilesService>,
    /// downloaded YouTube Music songs and playlists (folder and limit from Settings)
    pub downloads: Arc<downloads::DownloadsService>,
    /// the lane's stream chain (Monochrome → JioSaavn → YouTube): Settings' Monochrome check and JioSaavn search
    pub streams: sources::SourceChain,
    /// Last.fm scrobbling (the Settings card's bridge drives sign-in)
    pub lastfm: Arc<LastFmService>,
    /// Last.fm feed and Smart Mix (read calls with the account in `lastfm`)
    pub lastfm_feed: Arc<lastfm_feed::LastFmFeedService>,
    /// listening statistics (its own listen log, fed by the player)
    pub stats: Arc<stats::StatsService>,
    /// sleep timer over the player (both lanes)
    pub sleep: sf_player::sleep::SleepTimerHandle,
    pub player: PlayerHandle,
    app_requests: broadcast::Sender<AppRequest>,
    media: Arc<Mutex<Option<glue::MediaStop>>>,
}

/// Настройка «Обложки мимо прокси» → флаг клиента обложек (меняется сразу, без перезапуска).
fn follow_covers_bypass(rt: &Handle, settings: &Arc<SettingsStore>, direct: Arc<std::sync::atomic::AtomicBool>) {
    let settings = settings.clone();
    let mut changes = settings.subscribe();
    rt.spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SettingKey::CoversBypassProxy) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    direct.store(settings.get_bool(SettingKey::CoversBypassProxy), std::sync::atomic::Ordering::Relaxed);
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Settings "YouTube Music data saver", "lossless streams" and "JioSaavn streams" → `YouTubeService` and the
/// source chain (applies to the next resolved stream).
fn follow_youtube_data_saver(
    rt: &Handle,
    settings: &Arc<SettingsStore>,
    youtube: Arc<YouTubeService>,
    chain: sources::SourceChain,
) {
    let apply = move |settings: &SettingsStore| {
        let data_saver = settings.get_bool(SettingKey::YouTubeDataSaver);
        youtube.set_data_saver(data_saver);
        chain.set_data_saver(data_saver);
        chain.set_lossless(settings.get_bool(SettingKey::LosslessStreams));
        chain.set_saavn(settings.get_bool(SettingKey::JioSaavnStreams));
        chain.clear();
    };
    apply(settings.as_ref());
    let settings = settings.clone();
    let mut changes = settings.subscribe();
    rt.spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SettingKey::YouTubeDataSaver | SettingKey::LosslessStreams | SettingKey::JioSaavnStreams)
                | Err(broadcast::error::RecvError::Lagged(_)) => apply(settings.as_ref()),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Settings "Sync Playlists & Liked Songs", "Personalized Explore Feed" and "Personalized Queue & Radios" →
/// the library's YouTube lists, the home feed (read again when shown) and the radio requests; the Smart Mix
/// playlist settings (shown in Library, retention) → the library (library/smart_mixes.rs); "Sync local playlists
/// to YouTube Music" → the library (library/local.rs).
fn follow_youtube_account_use(
    rt: &Handle,
    settings: &Arc<SettingsStore>,
    youtube: Arc<YouTubeService>,
    home: Arc<HomeService>,
    library: Arc<LibraryService>,
) {
    youtube.set_personal_home(settings.get_bool(SettingKey::YouTubePersonalHome));
    youtube.set_personal_radio(settings.get_bool(SettingKey::YouTubePersonalRadio));
    let settings = settings.clone();
    let mut changes = settings.subscribe();
    rt.spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SettingKey::YouTubePersonalHome) => {
                    youtube.set_personal_home(settings.get_bool(SettingKey::YouTubePersonalHome));
                    home.reload_youtube_if_shown();
                }
                Ok(SettingKey::YouTubePersonalRadio) => {
                    youtube.set_personal_radio(settings.get_bool(SettingKey::YouTubePersonalRadio))
                }
                Ok(SettingKey::YouTubeLibrarySync) => library.youtube_sync_changed(),
                Ok(SettingKey::LastFmShowMixes) => library.smart_mix_visibility_changed(),
                Ok(SettingKey::LastFmMixRetentionDays) => library.sweep_smart_mixes(),
                Ok(SettingKey::LocalPlaylistSync) => library.local_sync_changed(),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    youtube.set_personal_home(settings.get_bool(SettingKey::YouTubePersonalHome));
                    youtube.set_personal_radio(settings.get_bool(SettingKey::YouTubePersonalRadio));
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Setting "Last.fm Smart Mix": Home's Daily Mix (Last.fm "mix" blend, 30 songs) and Your Mix (Last.fm
/// recommendations, 60 songs), found on YouTube Music, made once a day (checked hourly) and when the setting or
/// the Last.fm account changes; off or not connected — the mixes from the local history.
fn follow_lastfm_home_mixes(
    rt: &Handle,
    settings: &Arc<SettingsStore>,
    home: Arc<HomeService>,
    feed: Arc<lastfm_feed::LastFmFeedService>,
    lastfm: Arc<lastfm::LastFmService>,
) {
    let settings = settings.clone();
    let mut changes = settings.subscribe();
    let mut account = lastfm.subscribe();
    rt.spawn(async move {
        // the day the shown Last.fm mixes were made for (None — none shown)
        let mut made: Option<u64> = None;
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            let on = settings.get_bool(SettingKey::LastFmSmartMix) && lastfm.snapshot().connected;
            let today = home::mixes::day_seed(&jiff::Zoned::now());
            if !on && made.is_some() {
                made = None;
                home.set_lastfm_mixes(None);
            } else if on && made != Some(today) {
                let (daily, your) = tokio::join!(
                    feed.home_mix(lastfm_feed::MixMode::Mix, home::mixes::DAILY_LIMIT),
                    feed.home_mix(lastfm_feed::MixMode::Recommendations, home::mixes::YOUR_LIMIT),
                );
                // the setting may have been turned off meanwhile
                if settings.get_bool(SettingKey::LastFmSmartMix) {
                    match (daily, your) {
                        (Ok(daily), Ok(your)) => {
                            made = Some(today);
                            home.set_lastfm_mixes(Some((daily, your)));
                        }
                        (daily, your) => {
                            let error = daily.err().or(your.err()).unwrap_or_default();
                            tracing::warn!("Last.fm Smart Mix for Home: {error}");
                        }
                    }
                }
            }
            tokio::select! {
                change = changes.recv() => match change {
                    Ok(SettingKey::LastFmSmartMix) | Err(broadcast::error::RecvError::Lagged(_)) => made = None,
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                event = account.recv() => match event {
                    // a sign-in, sign-out or another account: made again (or dropped) above
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        let on = settings.get_bool(SettingKey::LastFmSmartMix) && lastfm.snapshot().connected;
                        if on != made.is_some() {
                            made = None;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => {}
            }
        }
    });
}

impl Services {
    /// Порядок: HTTP (прокси из настроек) → ApiGateway → PlayerHandle → Auth → History → Library → Catalog →
    /// Wave → Releases → Home → Lyrics → Devices → glue (настройки → плеер) → медиаклавиши.
    pub fn start(rt: &Handle, settings: Arc<SettingsStore>, options: ServicesOptions) -> anyhow::Result<Services> {
        // применённый прокси из настроек; пароль из хранилища подставит auth.start()
        let proxy = auth::settings_proxy(&settings, None);
        let http = sf_api::Http::from_proxy(&proxy).unwrap_or_else(|error| {
            tracing::warn!("invalid proxy settings: {error:#}");
            sf_api::Http::unavailable(error)
        });
        let api = Arc::new(ApiGateway::new(http.clone(), Arc::new(sf_api::NetActivity::default())));
        let covers_direct = Arc::new(std::sync::atomic::AtomicBool::new(settings.get_bool(SettingKey::CoversBypassProxy)));
        follow_covers_bypass(rt, &settings, covers_direct.clone());
        let config = PlayerConfig {
            cover_dir: mf_platform::dirs::cache_subdir("covers"),
            session_file: options.persist_session.then(mf_platform::dirs::session_file),
            volume: settings.get_f64(SettingKey::Volume).clamp(0.0, 1.0),
            // клиент только для обложек: по настройке «Обложки мимо прокси» — напрямую
            http: http.bypassing(covers_direct),
        };
        let player = PlayerHandle::start(rt, config, api.clone()).map_err(|e| anyhow::anyhow!("player: {e}"))?;
        let store = sf_auth::Store::with_runtime(mf_platform::dirs::state_dir(), rt.clone());
        // Last.fm keeps its session in the same credential store (slot LastFm); local playback is reported
        let lastfm = LastFmService::new(rt.clone(), settings.clone(), http.clone(), store.clone());
        lastfm.attach_player(player.clone());
        let auth = AuthService::new(rt.clone(), settings.clone(), api.clone(), http.clone(), player.clone(), store);
        let history = HistoryService::new(rt.clone(), auth.clone(), player.clone());
        let library =
            LibraryService::new(rt.clone(), auth.clone(), player.clone(), history.clone(), settings.clone());
        let catalog = CatalogService::new(auth.clone(), player.clone());
        let wave = WaveService::new(
            rt.clone(),
            auth.clone(),
            player.clone(),
            library.clone(),
            history.clone(),
            settings.clone(),
        );
        let releases =
            ReleasesService::new(rt.clone(), auth.clone(), library.clone(), history.clone(), player.clone(), http.clone());
        let home = HomeService::new(
            rt.clone(),
            auth.clone(),
            player.clone(),
            library.clone(),
            history.clone(),
            releases.clone(),
            settings.clone(),
        );
        // тексты — через общий клиент: тот же прокси, что у Web API, смена применяется сразу
        let fetcher = {
            let http = http.clone();
            mf_lyrics::LyricsFetcher::with_source(mf_platform::dirs::cache_subdir("lyrics"), move || http.client())
        };
        let lyrics = LyricsService::new(rt.clone(), player.clone(), fetcher);
        lyrics.set_youtube(auth.youtube());
        let devices = DevicesService::new(rt.clone(), auth.clone(), player.clone(), settings.clone());
        let social = SocialService::new(rt.clone(), auth.clone(), player.clone());
        let lanjam = lanjam::LanJamService::new(rt.clone(), Arc::new(player.clone()), lanjam::NetConfig::system());
        // owned by auth (built over the shared client: the same proxy as the Web API, changes apply at once);
        // the account is restored into it by auth.start()
        let youtube = auth.youtube();
        // YouTube tracks play on this computer through their own output (settings read once at start)
        let audio_device = settings.get_str(SettingKey::AudioDevice);
        let output = sf_player::engine::youtube::YtOutput {
            audio_device: (!audio_device.is_empty()).then_some(audio_device),
            normalisation: settings.get_bool(SettingKey::Normalisation),
            eq: glue::shared_eq(),
            ..Default::default()
        };
        // YouTube tracks may play from another catalog's copy (Monochrome FLAC, JioSaavn AAC) when Settings allow
        let streams = sources::SourceChain::new(youtube.clone(), http.clone());
        // local files play through the same lane: their URIs are answered with the file, the rest by `streams`
        let local = local::LocalFilesService::new(rt.clone());
        let lane_source = local::LaneSource::new(local.clone(), Some(Arc::new(streams.clone())));
        // downloaded songs play from their files (no network); downloads resolve through `streams`
        let downloads = downloads::DownloadsService::new(
            rt.clone(),
            http.clone(),
            Arc::new(streams.clone()),
            mf_platform::dirs::app_data_dir().join("downloads.json"),
            downloads::config_from_settings(&settings),
        );
        downloads::follow_settings(rt, &settings, downloads.clone());
        let lane_source = downloads::OfflineSource::new(downloads.clone(), lane_source);
        player.enable_youtube(lane_source, output, http.clone());
        local::follow_folders_setting(rt, &settings, local.clone());
        follow_youtube_data_saver(rt, &settings, youtube.clone(), streams.clone());
        follow_youtube_account_use(rt, &settings, youtube.clone(), home.clone(), library.clone());
        glue::spawn_settings_glue(rt, settings.clone(), player.clone());

        let (app_requests, _) = broadcast::channel(16);
        let update = update::UpdateService::new(rt.clone(), settings.clone(), http.clone(), app_requests.clone());
        let media = if options.enable_media {
            glue::start_media_controls(rt, settings.clone(), player.clone(), app_requests.clone())
        } else {
            None
        };

        let stats = stats::StatsService::new(rt.clone(), &player);
        let sleep = sf_player::sleep::SleepTimerHandle::spawn(rt, player.clone());
        let lastfm_feed = {
            let lastfm = lastfm.clone();
            let account: lastfm_feed::AccountSource = Arc::new(move || {
                let snapshot = lastfm.snapshot();
                if snapshot.connected { (snapshot.api_key, snapshot.username) } else { Default::default() }
            });
            let (yt_search, spotify): (Arc<dyn lastfm_feed::TrackSearch>, Arc<dyn lastfm_feed::TrackSearch>) =
                (youtube.clone(), catalog.clone());
            // feed tiles without Last.fm art get their cover from YouTube Music
            let covers: Arc<dyn lastfm_feed::CoverSearch> = youtube.clone();
            lastfm_feed::LastFmFeedService::new(
                rt.clone(),
                http.clone(),
                account,
                Some(yt_search),
                Some(spotify),
                Some(player.clone()),
                Some(covers),
                Default::default(),
            )
        };

        follow_lastfm_home_mixes(rt, &settings, home.clone(), lastfm_feed.clone(), lastfm.clone());

        Ok(Services {
            settings,
            api,
            auth,
            library,
            catalog,
            wave,
            home,
            releases,
            lyrics,
            history,
            devices,
            social,
            lanjam,
            update,
            youtube,
            local,
            downloads,
            streams,
            lastfm,
            lastfm_feed,
            stats,
            sleep,
            player,
            app_requests,
            media: Arc::new(Mutex::new(media)),
        })
    }

    pub fn app_requests(&self) -> broadcast::Receiver<AppRequest> {
        self.app_requests.subscribe()
    }

    /// При выходе (из GUI-потока после exec): сохранить сессию и историю, остановить движок и медиаклавиши.
    /// Блокирует ≤ 2 с.
    pub fn shutdown(&self) {
        self.lanjam.shutdown();
        self.player.shutdown();
        self.history.flush();
        self.stats.flush();
        if let Some(stop) = lock(&self.media).take() {
            stop.stop(Duration::from_millis(500));
        }
    }
}

/// Mutex без паники на «отравлении».
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
