// LastFm — mirror of LastFmService for the Settings card: the user's own API key and secret, browser sign-in,
// connection state. The Scrobble / Now playing switches are Settings.lastfmScrobble / Settings.lastfmNowPlaying.
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use sf_services::{LastFmEvent, LastFmSnapshot};

use super::{Delivery, forward_events};
use crate::context::ctx;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qproperty(bool, loaded, READ, NOTIFY = state_changed)]
        #[qproperty(bool, has_keys, cxx_name = "hasKeys", READ, NOTIFY = state_changed)]
        #[qproperty(bool, connected, READ, NOTIFY = state_changed)]
        #[qproperty(QString, username, READ, NOTIFY = state_changed)]
        #[qproperty(QString, api_key, cxx_name = "apiKey", READ, NOTIFY = state_changed)]
        #[qproperty(bool, signing_in, cxx_name = "signingIn", READ, NOTIFY = state_changed)]
        #[qproperty(QString, auth_url, cxx_name = "authUrl", READ, NOTIFY = state_changed)]
        #[qproperty(QString, error_text, cxx_name = "errorText", READ, NOTIFY = state_changed)]
        #[qproperty(QString, create_key_url, cxx_name = "createKeyUrl", READ, CONSTANT)]
        type LastFm = super::LastFmRust;

        #[qsignal]
        #[cxx_name = "stateChanged"]
        fn state_changed(self: Pin<&mut LastFm>);
        /// signed in (for a toast)
        #[qsignal]
        #[cxx_name = "connectedAs"]
        fn connected_as(self: Pin<&mut LastFm>, username: QString);

        /// The user's API key and shared secret; an empty secret keeps the saved one.
        #[qinvokable]
        #[cxx_name = "setKeys"]
        fn set_keys(self: &LastFm, api_key: &QString, secret: &QString);
        /// Opens the approval page in the browser and waits for it (up to 3 minutes).
        #[qinvokable]
        #[cxx_name = "signIn"]
        fn sign_in(self: &LastFm);
        #[qinvokable]
        #[cxx_name = "cancelSignIn"]
        fn cancel_sign_in(self: &LastFm);
        /// Forget the session, keep the keys.
        #[qinvokable]
        fn disconnect(self: &LastFm);
        /// Remove the keys and the session from the credential store.
        #[qinvokable]
        fn forget(self: &LastFm);
    }

    impl cxx_qt::Threading for LastFm {}
    impl cxx_qt::Initialize for LastFm {}
}

pub struct LastFmRust {
    loaded: bool,
    has_keys: bool,
    connected: bool,
    username: QString,
    api_key: QString,
    signing_in: bool,
    auth_url: QString,
    error_text: QString,
    create_key_url: QString,
}

impl Default for LastFmRust {
    fn default() -> Self {
        Self {
            loaded: false,
            has_keys: false,
            connected: false,
            username: QString::default(),
            api_key: QString::default(),
            signing_in: false,
            auth_url: QString::default(),
            error_text: QString::default(),
            create_key_url: QString::from(sf_services::lastfm::CREATE_KEY_URL),
        }
    }
}

impl qobject::LastFm {
    fn set_keys(&self, api_key: &QString, secret: &QString) {
        ctx().services.lastfm.set_keys(&api_key.to_string(), &secret.to_string());
    }

    fn sign_in(&self) {
        ctx().services.lastfm.begin_sign_in();
    }

    fn cancel_sign_in(&self) {
        ctx().services.lastfm.cancel_sign_in();
    }

    fn disconnect(&self) {
        ctx().services.lastfm.disconnect();
    }

    fn forget(&self) {
        ctx().services.lastfm.forget();
    }

    fn publish(mut self: Pin<&mut Self>, snapshot: LastFmSnapshot) {
        let mut this = self.as_mut().rust_mut();
        this.loaded = snapshot.loaded;
        this.has_keys = snapshot.has_keys;
        this.connected = snapshot.connected;
        this.username = QString::from(snapshot.username.as_str());
        this.api_key = QString::from(snapshot.api_key.as_str());
        this.signing_in = snapshot.signing_in;
        this.auth_url = QString::from(snapshot.auth_url.as_str());
        this.error_text = QString::from(snapshot.error_text.as_str());
        self.as_mut().state_changed();
    }

    fn apply(mut self: Pin<&mut Self>, delivery: Delivery<LastFmEvent>) {
        match delivery {
            Delivery::Event(LastFmEvent::Connected(username)) => {
                self.as_mut().connected_as(QString::from(username.as_str()));
            }
            // scrobbles and "now playing" don't change the card
            Delivery::Event(LastFmEvent::NowPlaying { .. } | LastFmEvent::Scrobbled { .. }) => return,
            Delivery::Event(LastFmEvent::Changed) | Delivery::Lagged => {}
        }
        let snapshot = ctx().services.lastfm.snapshot();
        self.publish(snapshot);
    }
}

impl cxx_qt::Initialize for qobject::LastFm {
    fn initialize(mut self: Pin<&mut Self>) {
        let events = ctx().services.lastfm.subscribe();
        let snapshot = ctx().services.lastfm.snapshot();
        self.as_mut().publish(snapshot);
        forward_events(self.qt_thread(), events, Self::apply);
    }
}
