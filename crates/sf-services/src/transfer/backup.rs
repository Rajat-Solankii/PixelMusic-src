//! Settings backup and restore, ported from soundwave1's desktop `BackupPanel`.
//!
//! One JSON file holds the settings, the local listening history and the listening-stats log. It never holds a
//! sign-in: account tokens, cookies, the proxy password and API keys live in the credential store (sf-auth) and
//! are not read here; the few settings that identify an account or a sign-in ([`is_private`]) are left out on
//! export and ignored on import. Window geometry is left out too: it belongs to one screen setup.
use std::path::Path;

use mf_platform::settings::{SettingKey, SettingValue, SettingsStore};
use serde_json::{Map, Value};

use crate::history::PlayRecord;
use crate::stats::ListenEvent;

/// The `format` field of a backup file.
pub const FORMAT: &str = "materialfy-backup";
/// The newest layout this build writes and reads.
pub const VERSION: u32 = 1;
/// Files bigger than this are not backups.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Settings that never leave the computer: sign-ins and keys, and the window's place on this screen.
pub fn is_private(key: SettingKey) -> bool {
    use SettingKey::*;
    if matches!(
        key,
        PlaybackAuthorized
            | WebClientId
            | ProxyUsername
            | WindowX
            | WindowY
            | WindowWidth
            | WindowHeight
            | WindowMaximized
            | FloatingFreeX
            | FloatingFreeY
    ) {
        return true;
    }
    // A key added later that names a secret stays private without a change here.
    let name = key.name().to_ascii_lowercase();
    ["token", "cookie", "secret", "password", "apikey", "clientid", "session", "credential", "auth"]
        .iter()
        .any(|word| name.contains(word))
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Backup {
    pub format: String,
    pub version: u32,
    /// ISO time
    pub created_at: String,
    pub app_version: String,
    /// setting name → value (string, bool or number)
    pub settings: Map<String, Value>,
    /// local listening history, newest first
    pub history: Vec<PlayRecord>,
    /// listening-stats log, oldest first
    pub stats: Vec<ListenEvent>,
}

fn to_json(value: SettingValue) -> Value {
    match value {
        SettingValue::Str(s) => Value::String(s),
        SettingValue::Bool(b) => Value::Bool(b),
        SettingValue::Int(i) => Value::from(i),
        SettingValue::Float(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
    }
}

fn from_json(value: &Value) -> Option<SettingValue> {
    Some(match value {
        Value::String(s) => SettingValue::Str(s.clone()),
        Value::Bool(b) => SettingValue::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SettingValue::Int(i),
            None => SettingValue::Float(n.as_f64()?),
        },
        _ => return None,
    })
}

impl Backup {
    /// A backup of every setting that is not private, plus the given local data.
    pub fn build(store: &SettingsStore, history: Vec<PlayRecord>, stats: Vec<ListenEvent>) -> Self {
        let settings = SettingKey::ALL
            .into_iter()
            .filter(|key| !is_private(*key))
            .map(|key| (key.name().to_owned(), to_json(store.get(key))))
            .filter(|(_, value)| !value.is_null())
            .collect();
        Self {
            format: FORMAT.into(),
            version: VERSION,
            created_at: jiff::Timestamp::now().to_string(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            settings,
            history,
            stats,
        }
    }

    /// Read and check a backup file.
    pub fn read(path: &Path) -> Result<Self, String> {
        let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if size > MAX_BYTES {
            return Err(mf_i18n::t("This file is too big to be a backup"));
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let backup: Self = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| mf_i18n::t("This file is not a Materialfy backup"))?;
        if backup.format != FORMAT {
            return Err(mf_i18n::t("This file is not a Materialfy backup"));
        }
        if backup.version > VERSION {
            return Err(mf_i18n::t("This backup was made by a newer version of Materialfy"));
        }
        Ok(backup)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Write the backup atomically.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        crate::data::fsio::write_atomic(path, self.to_json().as_bytes()).map_err(|e| format!("{e:#}"))
    }

    /// Put the settings back. Private and unknown names, and values of the wrong kind, are skipped.
    pub fn apply_settings(&self, store: &SettingsStore) -> RestoreReport {
        let mut report = RestoreReport::default();
        for (name, value) in &self.settings {
            let Some(key) = SettingKey::from_name(name).filter(|key| !is_private(*key)) else {
                report.skipped += 1;
                continue;
            };
            match from_json(value).and_then(|value| value.coerce(key.value_type())) {
                Some(value) => {
                    store.set(key, value);
                    report.settings += 1;
                }
                None => report.skipped += 1,
            }
        }
        report.history = self.history.len();
        report.stats = self.stats.len();
        report
    }
}

/// A file name for today's backup: "materialfy-backup-2026-11-12.json".
pub fn file_name() -> String {
    format!("materialfy-backup-{}.json", jiff::Zoned::now().date())
}

/// What a restore put back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub settings: usize,
    pub skipped: usize,
    pub history: usize,
    pub stats: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_settings_are_never_exported() {
        let store = SettingsStore::in_memory();
        store.set(SettingKey::WebClientId, SettingValue::Str("my-client-id".into()));
        store.set(SettingKey::ProxyUsername, SettingValue::Str("me".into()));
        store.set(SettingKey::PlaybackAuthorized, SettingValue::Bool(true));
        store.set(SettingKey::EqEnabled, SettingValue::Bool(true));
        store.set(SettingKey::EqPreamp, SettingValue::Float(-3.5));
        store.set(SettingKey::Crossfade, SettingValue::Int(6));
        let backup = Backup::build(&store, Vec::new(), Vec::new());
        let json = backup.to_json();
        assert!(!json.contains("my-client-id"));
        assert!(!backup.settings.contains_key("webClientId"));
        assert!(!backup.settings.contains_key("proxyUsername"));
        assert!(!backup.settings.contains_key("playbackAuthorized"));
        assert!(!backup.settings.contains_key("windowX"));
        assert_eq!(backup.settings["eqEnabled"], Value::Bool(true));
        assert_eq!(backup.settings["eqPreamp"], Value::from(-3.5));
        assert_eq!(backup.settings["crossfade"], Value::from(6));
        for key in SettingKey::ALL {
            assert_eq!(backup.settings.contains_key(key.name()), !is_private(key), "{}", key.name());
        }
    }

    #[test]
    fn a_backup_round_trips_and_skips_what_it_must() {
        let source = SettingsStore::in_memory();
        source.set(SettingKey::EqBands, SettingValue::Str("1,2,3".into()));
        source.set(SettingKey::Volume, SettingValue::Float(0.25));
        source.set(SettingKey::ThemeMode, SettingValue::Str("light".into()));
        let history = vec![PlayRecord { played_at: "2026-11-12T08:00:00Z".into(), ..Default::default() }];
        let stats = vec![ListenEvent { uri: "youtube:track:abc".into(), at: 5, seconds: 60, ..Default::default() }];
        let backup = Backup::build(&source, history.clone(), stats.clone());
        let mut parsed = Backup::parse(&backup.to_json()).unwrap();
        assert_eq!(parsed, backup);

        // a hand-edited file trying to set a private key or garbage is skipped
        parsed.settings.insert("webClientId".into(), Value::String("evil".into()));
        parsed.settings.insert("noSuchKey".into(), Value::Bool(true));
        parsed.settings.insert("crossfade".into(), Value::Array(Vec::new()));
        let target = SettingsStore::in_memory();
        let report = parsed.apply_settings(&target);
        assert_eq!(report.skipped, 3);
        assert_eq!(report.settings, parsed.settings.len() - 3);
        assert_eq!((report.history, report.stats), (1, 1));
        assert_eq!(target.get_str(SettingKey::WebClientId), "");
        assert_eq!(target.get_str(SettingKey::ThemeMode), "light");
        assert_eq!(target.get_f64(SettingKey::Volume), 0.25);
        assert_eq!(target.get_str(SettingKey::EqBands), "1.0,2.0,3.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0");
    }

    #[test]
    fn foreign_and_newer_files_are_refused() {
        assert!(Backup::parse("{}").is_err());
        assert!(Backup::parse("not json").is_err());
        assert!(Backup::parse(r#"{"format":"materialfy-backup","version":99}"#).is_err());
        let ok = Backup::parse("\u{feff}{\"format\":\"materialfy-backup\",\"version\":1}").unwrap();
        assert!(ok.settings.is_empty() && ok.history.is_empty());
    }

    #[test]
    fn write_and_read_a_file() {
        let dir = std::env::temp_dir().join(format!("mf-backup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("backup.json");
        let backup = Backup::build(&SettingsStore::in_memory(), Vec::new(), Vec::new());
        backup.write(&path).unwrap();
        assert_eq!(Backup::read(&path).unwrap(), backup);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
