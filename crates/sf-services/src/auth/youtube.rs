//! YouTube Music account on `AuthService`: browser sign-in (`sf_auth::youtube::sign_in`) or pasted cookies,
//! the `Slot::YouTube` credential slot, restore at startup, account card (`account/account_menu`), sign-out.
//! Independent of the Spotify sign-in: signing out of one keeps the other. Its own snapshot and events
//! (`watch_youtube`, `subscribe_youtube`), so the Spotify `AuthEvent` matches in the bridges stay exhaustive.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sf_auth::YouTubeAccount;
use sf_auth::credentials::{Grant as StoredGrant, Slot};
use tokio::sync::{broadcast, watch};

use super::AuthService;
use crate::lock;
use crate::youtube::{AccountInfo, YouTubeError, YouTubeService};

/// YouTube Music sign-in state. `as_str` → the name QML sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YouTubeAuthState {
    #[default]
    SignedOut,
    /// reading the saved account at startup
    Checking,
    /// the sign-in browser window is open
    AwaitingBrowser,
    /// cookies are in, reading the account card
    Verifying,
    SignedIn,
}

impl YouTubeAuthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SignedOut => "signedOut",
            Self::Checking => "checking",
            Self::AwaitingBrowser => "awaitingBrowser",
            Self::Verifying => "verifying",
            Self::SignedIn => "signedIn",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct YouTubeAuthSnapshot {
    pub state: YouTubeAuthState,
    /// name / email / avatar; `None` while signed out
    pub account: Option<AccountInfo>,
    /// last failure, already translated; "" when none
    pub error_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum YouTubeAuthEvent {
    SignedIn(AccountInfo),
    SignedOut,
}

pub(super) struct YtAuth {
    pub(super) service: Arc<YouTubeService>,
    /// the sign-in browser's private profile (`state_dir/youtube-signin-profile`)
    profile_dir: PathBuf,
    snapshot: watch::Sender<YouTubeAuthSnapshot>,
    events: broadcast::Sender<YouTubeAuthEvent>,
    /// bumped by every sign-in attempt, cancel and sign-out: late results of older attempts are dropped
    generation: AtomicU64,
    login: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
}

impl YtAuth {
    pub(super) fn new(service: Arc<YouTubeService>, profile_dir: PathBuf) -> Self {
        Self {
            service,
            profile_dir,
            snapshot: watch::channel(YouTubeAuthSnapshot::default()).0,
            events: broadcast::channel(16).0,
            generation: AtomicU64::new(0),
            login: Default::default(),
        }
    }
}

/// The card from what the saved account already knows (shown before `account_menu` answers).
fn card_of(account: &YouTubeAccount) -> AccountInfo {
    AccountInfo {
        name: account.name.clone().unwrap_or_default(),
        email: account.email.clone().unwrap_or_default(),
        avatar: account.avatar.clone().unwrap_or_default(),
    }
}

/// Default folder of the sign-in browser profile.
pub(super) fn default_profile_dir() -> PathBuf {
    mf_platform::dirs::state_dir().join("youtube-signin-profile")
}

impl AuthService {
    /// The YouTube Music service every other service routes `youtube:` items to.
    pub fn youtube(&self) -> Arc<YouTubeService> {
        self.yt.service.clone()
    }

    pub fn is_youtube_signed_in(&self) -> bool {
        self.yt.snapshot.borrow().state == YouTubeAuthState::SignedIn
    }

    pub fn youtube_snapshot(&self) -> YouTubeAuthSnapshot {
        self.yt.snapshot.borrow().clone()
    }

    pub fn watch_youtube(&self) -> watch::Receiver<YouTubeAuthSnapshot> {
        self.yt.snapshot.subscribe()
    }

    pub fn subscribe_youtube(&self) -> broadcast::Receiver<YouTubeAuthEvent> {
        self.yt.events.subscribe()
    }

    fn yt_update(&self, change: impl FnOnce(&mut YouTubeAuthSnapshot)) {
        self.yt.snapshot.send_if_modified(|snapshot| {
            let before = snapshot.clone();
            change(snapshot);
            *snapshot != before
        });
    }

    fn yt_next_generation(&self) -> u64 {
        if let Some(login) = lock(&self.yt.login).take() {
            login.abort();
        }
        self.yt.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn yt_current(&self, generation: u64) -> bool {
        self.yt.generation.load(Ordering::SeqCst) == generation
    }

    /// Restore the saved YouTube Music account (called by `start`; safe to call again).
    pub fn restore_youtube(&self) {
        let generation = self.yt_next_generation();
        self.yt_update(|s| {
            s.state = YouTubeAuthState::Checking;
            s.error_text.clear();
        });
        self.spawn(move |service| async move {
            let loaded = service.store.lease(Slot::YouTube).load().await;
            if !service.yt_current(generation) {
                return;
            }
            let account = match loaded {
                Ok(loaded) => {
                    if let Some(warning) = loaded.warning {
                        tracing::warn!("saved YouTube sign-in: {warning}");
                    }
                    match loaded.grant {
                        Some(StoredGrant::YouTube(account)) => Some(account),
                        _ => None,
                    }
                }
                Err(error) => {
                    tracing::warn!("saved YouTube sign-in not read: {error:#}");
                    service.yt_update(|s| s.error_text = error.to_string());
                    None
                }
            };
            let Some(account) = account else {
                service.yt.service.set_account(None);
                service.yt_update(|s| s.state = YouTubeAuthState::SignedOut);
                return;
            };
            // signed in at once from the saved card; the fresh card follows when YouTube answers
            let card = card_of(&account);
            service.yt.service.set_account(Some(account.clone()));
            service.yt_update(|s| {
                s.state = YouTubeAuthState::SignedIn;
                s.account = Some(card.clone());
            });
            let _ = service.yt.events.send(YouTubeAuthEvent::SignedIn(card));
            service.refresh_youtube_card(generation, account, false).await;
        });
    }

    /// Open the sign-in browser window (Edge or Chrome) and wait for the user to sign in.
    pub fn start_youtube_login(&self) {
        let generation = self.yt_next_generation();
        self.yt_update(|s| {
            s.state = YouTubeAuthState::AwaitingBrowser;
            s.error_text.clear();
        });
        let profile_dir = self.yt.profile_dir.clone();
        let me = self.me.clone();
        let task = self.rt.spawn(async move {
            let result = sf_auth::youtube::sign_in(&profile_dir).await;
            let Some(service) = me.upgrade() else { return };
            if !service.yt_current(generation) {
                return;
            }
            match result {
                Ok(account) => service.finish_youtube_login(generation, account).await,
                Err(error) => service.yt_failed(error.to_string()),
            }
        });
        *lock(&self.yt.login) = Some(task.abort_handle());
    }

    /// Close the sign-in window without signing in.
    pub fn cancel_youtube_login(&self) {
        let waiting = matches!(
            self.yt.snapshot.borrow().state,
            YouTubeAuthState::AwaitingBrowser | YouTubeAuthState::Verifying
        );
        if waiting {
            self.yt_next_generation();
            self.yt.service.set_account(None);
            self.yt_update(|s| {
                s.state = YouTubeAuthState::SignedOut;
                s.account = None;
            });
        }
    }

    /// Sign in with a `Cookie:` header pasted from the browser (machines without Edge/Chrome). A header
    /// without a signed-in session is refused at once (the text is also put in the snapshot).
    pub fn youtube_sign_in_with_cookies(&self, cookies: &str) -> Result<(), String> {
        let account = match YouTubeAccount::from_cookie_string(cookies) {
            Ok(account) => account,
            Err(error) => {
                let text = error.to_string();
                self.yt_update(|s| s.error_text = text.clone());
                return Err(text);
            }
        };
        let generation = self.yt_next_generation();
        self.yt_update(|s| {
            s.state = YouTubeAuthState::Verifying;
            s.error_text.clear();
        });
        self.spawn(move |service| async move { service.finish_youtube_login(generation, account).await });
        Ok(())
    }

    /// Sign out of YouTube Music: forget the saved cookies and the sign-in browser profile. Spotify stays.
    pub fn youtube_sign_out(&self) {
        self.yt_next_generation();
        if let Err(error) = self.store.revoke(Slot::YouTube) {
            tracing::warn!("YouTube sign-in revocation: {error:#}");
        }
        self.delete_slot(Slot::YouTube);
        self.yt.service.set_account(None);
        let profile_dir = self.yt.profile_dir.clone();
        self.rt.spawn_blocking(move || {
            if let Err(error) = sf_auth::youtube::remove_profile(&profile_dir) {
                tracing::warn!("YouTube sign-in profile not removed: {error}");
            }
        });
        let was_signed_in = self.is_youtube_signed_in();
        self.yt_update(|s| {
            s.state = YouTubeAuthState::SignedOut;
            s.account = None;
            s.error_text.clear();
        });
        if was_signed_in {
            let _ = self.yt.events.send(YouTubeAuthEvent::SignedOut);
        }
    }

    /// Re-read the account card (name, email, avatar) from YouTube Music.
    pub fn refresh_youtube_account(&self) {
        let Some(account) = self.yt.service.client().account() else { return };
        let generation = self.yt.generation.load(Ordering::SeqCst);
        self.spawn(move |service| async move { service.refresh_youtube_card(generation, account, false).await });
    }

    fn yt_failed(&self, text: String) {
        self.yt.service.set_account(None);
        let was_signed_in = self.is_youtube_signed_in();
        self.yt_update(|s| {
            s.state = YouTubeAuthState::SignedOut;
            s.account = None;
            s.error_text = text;
        });
        if was_signed_in {
            let _ = self.yt.events.send(YouTubeAuthEvent::SignedOut);
        }
    }

    /// Fresh cookies: check them with `account_menu`, save, then signed in.
    async fn finish_youtube_login(self: Arc<Self>, generation: u64, account: YouTubeAccount) {
        self.yt_update(|s| s.state = YouTubeAuthState::Verifying);
        self.yt.service.set_account(Some(account.clone()));
        self.refresh_youtube_card(generation, account, true).await;
    }

    /// `account_menu` → card. `fresh`: the cookies came from a sign-in now, so a refusal is a failed sign-in
    /// and the account is saved after the check. A restored account stays signed in through network errors
    /// and is dropped only when YouTube says the session is over.
    async fn refresh_youtube_card(self: &Arc<Self>, generation: u64, mut account: YouTubeAccount, fresh: bool) {
        let answer = self.yt.service.account_info().await;
        if !self.yt_current(generation) {
            return;
        }
        let card = match answer {
            Ok(card) => card,
            Err(error @ (YouTubeError::SessionExpired | YouTubeError::NotSignedIn)) => {
                if !fresh {
                    let _ = self.store.revoke(Slot::YouTube);
                    self.delete_slot(Slot::YouTube);
                }
                self.yt_failed(error.to_string());
                return;
            }
            Err(error) if fresh => {
                self.yt_failed(error.to_string());
                return;
            }
            Err(error) => {
                tracing::warn!("YouTube account card: {error:?}");
                return;
            }
        };
        let changed = account.name.as_deref() != Some(card.name.as_str())
            || account.email.as_deref() != Some(card.email.as_str())
            || account.avatar.as_deref() != Some(card.avatar.as_str());
        account.name = Some(card.name.clone());
        account.email = Some(card.email.clone());
        account.avatar = Some(card.avatar.clone());
        if fresh || changed {
            self.yt.service.client().set_account(Some(account.clone()));
            if let Err(error) = self.store.lease(Slot::YouTube).save(StoredGrant::YouTube(account)).await {
                tracing::warn!("YouTube sign-in not saved: {error:#}");
                (self.storage_notice())(error);
            }
            if !self.yt_current(generation) {
                return;
            }
        }
        let newly = self.yt.snapshot.borrow().state != YouTubeAuthState::SignedIn;
        self.yt_update(|s| {
            s.state = YouTubeAuthState::SignedIn;
            s.account = Some(card.clone());
            s.error_text.clear();
        });
        if newly {
            let _ = self.yt.events.send(YouTubeAuthEvent::SignedIn(card));
        }
    }
}
