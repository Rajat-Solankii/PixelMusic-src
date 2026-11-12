//! Last.fm account as kept in the OS credential store (`Slot::LastFm`): the user's own API key and shared secret
//! (last.fm/api/account/create) plus, once signed in, the session key and user name. Never in a plain file.
use serde::{Deserialize, Serialize};

/// No Debug: holds the shared secret and the session key.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LastFmAccount {
    pub api_key: String,
    pub secret: String,
    /// "" until the browser approval finished
    pub session_key: String,
    pub username: String,
}

impl LastFmAccount {
    pub fn new(api_key: &str, secret: &str) -> Self {
        Self { api_key: api_key.trim().to_owned(), secret: secret.trim().to_owned(), ..Default::default() }
    }

    pub fn has_keys(&self) -> bool {
        !self.api_key.is_empty() && !self.secret.is_empty()
    }

    pub fn connected(&self) -> bool {
        self.has_keys() && !self.session_key.is_empty()
    }

    /// The same keys without the session (sign-out, or error 9 "invalid session key").
    pub fn disconnected(&self) -> Self {
        Self { api_key: self.api_key.clone(), secret: self.secret.clone(), ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_trimmed_and_disconnect_keeps_them() {
        let mut account = LastFmAccount::new(" dummy-key ", "dummy-secret\n");
        assert_eq!(account.api_key, "dummy-key");
        assert!(account.has_keys() && !account.connected());
        account.session_key = "dummy-session".into();
        account.username = "someone".into();
        assert!(account.connected());
        let plain = account.disconnected();
        assert!(plain.has_keys() && !plain.connected() && plain.username.is_empty());
        assert!(!LastFmAccount::new("dummy-key", " ").has_keys());
    }
}
