//! Вход (исполнитель «services-core»). Вход в Spotify из приложения убран: ни браузерного PKCE, ни loopback,
//! ни восстановления сохранённых грантов Spotify (слоты Playback/Shared/Personal при запуске не читаются и не
//! удаляются). Здесь остались прокси (пароль из слота Proxy), аккаунт YouTube Music (auth/youtube.rs) и снимок
//! для моста Auth, который для Spotify всегда «signedOut».
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use md3_model::Account;
use sf_api::{ApiError, ApiGateway, ApiSource};
use sf_auth::credentials::{Error as StoreError, Grant as StoredGrant, ProxyPassword, Slot};
use sf_auth::{ProxyConfig, ProxyMode};
use sf_player::PlayerHandle;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};
use mf_platform::{SettingKey, SettingValue, SettingsStore};

use crate::lock;

mod youtube;
pub use youtube::{YouTubeAuthEvent, YouTubeAuthSnapshot, YouTubeAuthState};

/// Состояние входа (Web API). as_str → Auth.state в QML.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthState {
    /// восстановление сохранённого входа
    #[default]
    Checking,
    SignedOut,
    /// браузер открыт, ждём редирект на 127.0.0.1:8989
    AwaitingBrowser,
    /// код получен, обмен/проверка аккаунта
    Connecting,
    SignedIn,
    Error,
}

impl AuthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::SignedOut => "signedOut",
            Self::AwaitingBrowser => "awaitingBrowser",
            Self::Connecting => "connecting",
            Self::SignedIn => "signedIn",
            Self::Error => "error",
        }
    }
}

/// Воспроизведение на этом ПК (второй грант + librespot). as_str → Auth.playbackState.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackAuthState {
    /// не включено (Premium есть, разрешения нет)
    #[default]
    None,
    /// браузер открыт, ждём редирект на 127.0.0.1:8898
    Authorizing,
    /// поднимается Engine
    Connecting,
    Ready,
    Error,
    /// аккаунт Free — движок не запускается (librespot 0.8 делает exit(1))
    PremiumRequired,
}

impl PlaybackAuthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Authorizing => "authorizing",
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::Error => "error",
            Self::PremiumRequired => "premiumRequired",
        }
    }
}

/// Снимок для моста Auth.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuthSnapshot {
    pub state: AuthState,
    /// по-русски: «Вход отменён в браузере», «Порт 8989 занят другой программой», …
    pub error_text: String,
    /// URL accounts.spotify.com/authorize текущего ожидания (Web API или playback), "" — нет
    pub auth_url: String,
    /// порт loopback текущего ожидания (8989 | 8898), 0 — нет
    pub auth_port: u16,
    pub playback: PlaybackAuthState,
    pub playback_error: String,
    /// стабильный код error_text для подсказок в QML (текст переведён): "" | "free" | "cancelled" | "portBusy" |
    /// "noResponse" | "accountMismatch" | "otherAccount" | "audioKey"
    pub error_code: String,
    /// то же для playback_error
    pub playback_error_code: String,
    pub account: Option<Account>,
    /// "keyring" | ""
    pub token_storage: String,
    /// личный Client ID ("" — нет) и его состояние: "none" | "authorizing" | "ready" | "error"
    pub personal_client_id: String,
    pub personal_state: String,
    /// токен общего Web API (необязателен при входе через сессию): "none" | "authorizing" | "ready" | "error"
    pub web_state: String,
    /// применение прокси: "" | "applying" | "ok" | "error"
    pub proxy_state: String,
    pub proxy_error: String,
    /// параметры движка в настройках (glue::engine_config) отличаются от запущенного; сброс — в restart_engine
    pub engine_restart_pending: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AuthEvent {
    SignedIn(Account),
    SignedOut,
    /// локальное воспроизведение готово (device_id)
    PlaybackReady(String),
    /// вход есть, но аккаунт Free
    PremiumRequired,
}

/// Поля формы прокси (Settings → «Сеть»); пароль не сохраняется в настройках.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxyInput {
    /// "off" | "system" | "http" | "socks"
    pub mode: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
}

struct Inner {
    started: bool,
    proxy: ProxyConfig,
}

pub struct AuthService {
    rt: Handle,
    settings: Arc<SettingsStore>,
    api: Arc<ApiGateway>,
    http: sf_api::Http,
    player: PlayerHandle,
    store: sf_auth::Store,
    snapshot: watch::Sender<AuthSnapshot>,
    events: broadcast::Sender<AuthEvent>,
    /// действующий прокси — для оболочки (QNetworkAccessManager картинок QML)
    proxy: watch::Sender<ProxyConfig>,
    inner: Mutex<Inner>,
    /// YouTube Music account (auth/youtube.rs)
    yt: youtube::YtAuth,
    me: Weak<AuthService>,
}

impl AuthService {
    /// The settings store (services that only hold the auth service read their switches here).
    pub fn settings(&self) -> Arc<SettingsStore> {
        self.settings.clone()
    }
}

/// Прокси из применённых настроек (+ пароль из слота).
pub(crate) fn settings_proxy(settings: &SettingsStore, password: Option<&ProxyPassword>) -> ProxyConfig {
    let mode = ProxyMode::from_setting(&settings.get_str(SettingKey::ProxyMode));
    let host = settings.get_str(SettingKey::ProxyHost);
    let port = settings.get_str(SettingKey::ProxyPort);
    let username = settings.get_str(SettingKey::ProxyUsername);
    match ProxyConfig::from_parts(mode, &host, &port, &username, "") {
        Ok(mut config) => {
            if let Some(password) = password {
                config.restore_password(password);
            }
            config
        }
        Err(error) => ProxyConfig::Invalid(error),
    }
}

impl AuthService {
    pub fn new(
        rt: Handle,
        settings: Arc<SettingsStore>,
        api: Arc<ApiGateway>,
        http: sf_api::Http,
        player: PlayerHandle,
        store: sf_auth::Store,
    ) -> Arc<Self> {
        let youtube = Arc::new(crate::youtube::YouTubeService::new(http.clone()));
        Self::with_youtube(rt, settings, api, http, player, store, youtube, youtube::default_profile_dir())
    }

    /// As `new`, with a prepared YouTube Music service and sign-in browser profile folder (tests).
    #[allow(clippy::too_many_arguments)]
    pub fn with_youtube(
        rt: Handle,
        settings: Arc<SettingsStore>,
        api: Arc<ApiGateway>,
        http: sf_api::Http,
        player: PlayerHandle,
        store: sf_auth::Store,
        youtube: Arc<crate::youtube::YouTubeService>,
        youtube_profile_dir: PathBuf,
    ) -> Arc<Self> {
        // до start() входа нет: с --offline оболочка start() не зовёт, «Входим…» висеть не должно
        let snapshot = AuthSnapshot {
            state: AuthState::SignedOut,
            token_storage: store.backend_name().to_owned(),
            personal_state: "none".to_owned(),
            web_state: "none".to_owned(),
            ..Default::default()
        };
        // тот же прокси, что у HTTP-клиента из Services::start; пароль подставит restore()
        let proxy = settings_proxy(&settings, None);
        Arc::new_cyclic(|me| Self {
            rt,
            settings,
            api,
            http,
            player,
            store,
            snapshot: watch::channel(snapshot).0,
            events: broadcast::channel(64).0,
            proxy: watch::channel(proxy.clone()).0,
            inner: Mutex::new(Inner { started: false, proxy }),
            yt: youtube::YtAuth::new(youtube, youtube_profile_dir),
            me: me.clone(),
        })
    }

    fn update(&self, change: impl FnOnce(&mut AuthSnapshot)) {
        self.snapshot.send_if_modified(|snapshot| {
            let before = snapshot.clone();
            change(snapshot);
            *snapshot != before
        });
    }

    fn spawn<F>(&self, task: impl FnOnce(Arc<Self>) -> F + Send + 'static)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let me = self.me.clone();
        self.rt.spawn(async move {
            if let Some(service) = me.upgrade() {
                task(service).await;
            }
        });
    }

    /// Ошибка хранилища секретов: вход не запомнится (без Debug секретов).
    fn storage_notice(&self) -> Arc<dyn Fn(StoreError) + Send + Sync> {
        let me = self.me.clone();
        Arc::new(move |error| {
            if error == StoreError::Stale {
                return;
            }
            tracing::warn!("credential store: {error:#}");
            if let Some(service) = me.upgrade()
                && matches!(error, StoreError::Unavailable | StoreError::Locked | StoreError::Verification)
            {
                service.update(|s| s.token_storage.clear());
            }
        })
    }

    fn delete_slot(&self, slot: Slot) {
        let pending = self.store.lease(slot).delete();
        let notice = self.storage_notice();
        self.rt.spawn(async move {
            if let Err(error) = pending.await {
                notice(error);
            }
        });
    }

    // --- запуск ---------------------------------------------------------------------------------

    /// Прокси (пароль из слота Proxy) и аккаунт YouTube Music. Сохранённые гранты Spotify не читаются и не
    /// удаляются: вход в Spotify убран, состояние сразу signedOut, локальный движок Spotify не ждём.
    pub fn start(&self) {
        {
            let mut inner = lock(&self.inner);
            if inner.started {
                return;
            }
            inner.started = true;
        }
        self.update(|s| {
            s.state = AuthState::Checking;
            s.error_text.clear();
        });
        self.spawn(|service| async move { service.restore().await });
        self.restore_youtube();
    }

    async fn restore(self: Arc<Self>) {
        let password = match self.store.lease(Slot::Proxy).load().await {
            Ok(loaded) => {
                if let Some(warning) = loaded.warning {
                    tracing::warn!("proxy password: {warning}");
                }
                match loaded.grant {
                    Some(StoredGrant::Proxy(password)) => Some(password),
                    _ => None,
                }
            }
            Err(error) => {
                tracing::warn!("proxy password not read: {error:#}");
                None
            }
        };
        self.install_proxy(settings_proxy(&self.settings, password.as_ref()));
        self.api.clear(ApiSource::Shared);
        self.api.clear(ApiSource::Personal);
        self.player.expect_local(false);
        self.update(|s| {
            if s.state == AuthState::Checking {
                s.state = AuthState::SignedOut;
            }
        });
    }

    // --- вход в Spotify убран: точки входа моста ничего не делают --------------------------------

    /// Был «Войти через Spotify»: браузер не открывается, loopback не слушается.
    pub fn start_login(&self) {
        tracing::info!("Spotify sign-in is not available");
    }

    /// Был «Войти без Premium» (Web API).
    pub fn start_web_login(&self) {
        tracing::info!("Spotify sign-in is not available");
    }

    /// Был «Разрешить воспроизведение» (librespot).
    pub fn authorize_playback(&self) {
        tracing::info!("Spotify sign-in is not available");
    }

    /// Был вход личного приложения Web API.
    pub fn set_personal_client_id(&self, _client_id: Option<String>) {
        tracing::info!("Spotify sign-in is not available");
    }

    /// Ожидания браузера не бывает.
    pub fn reopen_browser(&self) {}

    /// Ожидания браузера не бывает.
    pub fn cancel_login(&self) {}

    /// Повторять нечего: вход в Spotify не начинается.
    pub fn retry(&self) {}

    /// Профиля Spotify нет.
    pub fn refresh_profile(&self) {}

    /// Ошибки Web API Spotify не влияют на вход: грантов нет.
    pub fn note_api_error(&self, _error: &ApiError) {}

    /// Движок Spotify не запускается — перезапускать нечего.
    pub fn restart_engine(&self) {
        self.update(|s| s.engine_restart_pending = false);
    }

    /// Воспроизведение Spotify не включается: только сбросить настройку.
    pub fn disable_playback(&self) {
        self.settings.set(SettingKey::PlaybackAuthorized, SettingValue::Bool(false));
        self.update(|s| {
            s.playback = PlaybackAuthState::None;
            s.playback_error.clear();
        });
    }

    /// Выход из Spotify: забыть оставшиеся с прошлых версий гранты Spotify (YouTube Music, Last.fm и пароль
    /// прокси не трогаются); снимок — signedOut.
    pub fn sign_out(&self) {
        if let Err(error) = self.store.revoke_spotify() {
            tracing::warn!("sign-in revocation: {error:#}");
        }
        self.api.clear_all();
        for slot in Slot::SPOTIFY {
            self.delete_slot(slot);
        }
        self.settings.set(SettingKey::PlaybackAuthorized, SettingValue::Bool(false));
        self.update(|s| {
            s.state = AuthState::SignedOut;
            s.error_text.clear();
            s.error_code.clear();
            s.auth_url.clear();
            s.auth_port = 0;
            s.account = None;
            s.playback = PlaybackAuthState::None;
            s.playback_error.clear();
            s.playback_error_code.clear();
            s.personal_state = "none".to_owned();
            s.web_state = "none".to_owned();
            s.engine_restart_pending = false;
        });
    }

    // --- прокси ----------------------------------------------------------------------------------

    fn install_proxy(&self, config: ProxyConfig) {
        match sf_api::http::build_client(&config) {
            Ok(client) => {
                self.http.replace(client);
                lock(&self.inner).proxy = config.clone();
                self.publish_proxy(config);
            }
            Err(error) => {
                tracing::warn!("proxy settings are invalid: {error:#}");
                self.http.block(error.clone());
                lock(&self.inner).proxy = ProxyConfig::Invalid(error.clone());
                self.publish_proxy(ProxyConfig::Invalid(error.clone()));
                self.update(|s| {
                    s.proxy_state = "error".to_owned();
                    s.proxy_error = error;
                });
            }
        }
    }

    /// Применить прокси: проверить поля; только после успешной проверки записать ProxyMode/Host/Port/Username
    /// в настройки (мост их только читает) и пароль в слот Proxy, заменить HTTP-клиент.
    pub fn apply_proxy(&self, input: ProxyInput) {
        self.update(|s| {
            s.proxy_state = "applying".to_owned();
            s.proxy_error.clear();
        });
        self.spawn(move |service| async move {
            let fail = |text: String| {
                service.update(|s| {
                    s.proxy_state = "error".to_owned();
                    s.proxy_error = text;
                })
            };
            let mode_name = input.mode.trim().to_lowercase();
            if !ProxyMode::ALL.iter().any(|mode| mode.as_str() == mode_name) {
                return fail(mf_i18n::t("Unknown proxy mode"));
            }
            let mode = ProxyMode::from_setting(&mode_name);
            let (host, port, username) =
                (input.host.trim().to_owned(), input.port.trim().to_owned(), input.username.trim().to_owned());
            let mut password = input.password.clone();
            if mode.is_manual() && password.is_empty() && !username.is_empty()
                && let Ok(loaded) = service.store.lease(Slot::Proxy).load().await
                && let Some(StoredGrant::Proxy(saved)) = loaded.grant
                && saved.host == host
                && saved.port.to_string() == port
                && saved.username == username
            {
                password = saved.password;
            }
            let config = match ProxyConfig::from_parts(mode, &host, &port, &username, &password) {
                Ok(config) => config,
                Err(text) => return fail(text),
            };
            let client = match sf_api::http::build_client(&config) {
                Ok(client) => client,
                Err(text) => return fail(mf_i18n::tf("Proxy rejected: {0}", &[&text])),
            };
            let settings = &service.settings;
            settings.set(SettingKey::ProxyMode, SettingValue::Str(mode.as_str().to_owned()));
            settings.set(SettingKey::ProxyHost, SettingValue::Str(if mode.is_manual() { host } else { String::new() }));
            settings.set(SettingKey::ProxyPort, SettingValue::Str(if mode.is_manual() { port } else { String::new() }));
            settings.set(
                SettingKey::ProxyUsername,
                SettingValue::Str(if mode.is_manual() { username } else { String::new() }),
            );
            if mode.is_manual() {
                if let Some(record) = config.password_record() {
                    service.store.invalidate(Slot::Proxy);
                    if let Err(error) = service.store.lease(Slot::Proxy).save(StoredGrant::Proxy(record)).await {
                        tracing::warn!("proxy password not saved: {error:#}");
                    }
                } else {
                    let _ = service.store.revoke(Slot::Proxy);
                    service.delete_slot(Slot::Proxy);
                }
            }
            service.http.replace(client);
            lock(&service.inner).proxy = config.clone();
            service.publish_proxy(config);
            service.update(|s| {
                s.proxy_state = "ok".to_owned();
                s.proxy_error.clear();
            });
        });
    }

    pub fn is_signed_in(&self) -> bool {
        self.snapshot.borrow().state == AuthState::SignedIn
    }

    pub fn is_premium(&self) -> bool {
        self.snapshot.borrow().account.as_ref().is_some_and(|a| a.premium)
    }

    pub fn account(&self) -> Option<Account> {
        self.snapshot.borrow().account.clone()
    }

    pub fn account_id(&self) -> Option<md3_model::AccountId> {
        self.account().map(|a| md3_model::AccountId::new(a.id))
    }

    /// Шлюз Web API (общий для всех сервисов).
    pub fn api(&self) -> Arc<ApiGateway> {
        self.api.clone()
    }

    pub fn snapshot(&self) -> AuthSnapshot {
        self.snapshot.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<AuthSnapshot> {
        self.snapshot.subscribe()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AuthEvent> {
        self.events.subscribe()
    }

    /// Действующий прокси (с паролем) — тот же, через который идут запросы к Web API.
    pub fn proxy(&self) -> ProxyConfig {
        self.proxy.borrow().clone()
    }

    /// Изменения действующего прокси: после restore() при входе и после каждого успешного apply_proxy.
    pub fn watch_proxy(&self) -> watch::Receiver<ProxyConfig> {
        self.proxy.subscribe()
    }

    fn publish_proxy(&self, config: ProxyConfig) {
        self.proxy.send_if_modified(|current| {
            let changed = *current != config;
            *current = config;
            changed
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_name_like_qml() {
        assert_eq!(AuthState::AwaitingBrowser.as_str(), "awaitingBrowser");
        assert_eq!(PlaybackAuthState::PremiumRequired.as_str(), "premiumRequired");
    }
}
