//! CoverTones: the colours of one cover for a small tinted surface (Home's recently-played pills). `tones(url, dark)`
//! answers at once from a cache: {container, onContainer, onContainerVariant} (QColor), or an empty map while the
//! cover is still being read or when it could not be read (QML keeps its theme tones then). A miss starts the work
//! in the background: the picture comes from the player's cover cache (sf_player::engine::cover, cache_subdir
//! "covers": one download per URL, kept on disk), the tones from mf_theme::cover_tones_from_image; `revision` goes
//! up when an answer lands so QML bindings that read it ask again.
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QColor, QString, QVariant};
use mf_theme::CoverTones as Tones;

use super::local_path;
use crate::context::ctx;
use crate::qt::QVariantMap;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qmap.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qproperty(i32, revision, READ, NOTIFY = revision_changed)]
        type CoverTones = super::CoverTonesRust;

        #[qsignal]
        #[cxx_name = "revisionChanged"]
        fn revision_changed(self: Pin<&mut CoverTones>);

        /// http(s) URL, file:// URL or path of a cover; `dark` — tones for the dark scheme.
        #[qinvokable]
        fn tones(self: Pin<&mut CoverTones>, url: &QString, dark: bool) -> QMap_QString_QVariant;
    }

    impl cxx_qt::Threading for CoverTones {}
}

/// Covers remembered at most (the map is dropped whole past this; it fills again from the disk cache).
const MAX_ENTRIES: usize = 512;

enum Entry {
    Pending,
    Ready(Tones, Tones),
    Failed,
}

#[derive(Default)]
pub struct CoverTonesRust {
    revision: i32,
    entries: HashMap<String, Entry>,
}

fn color(argb: u32) -> QVariant {
    let [a, r, g, b] = argb.to_be_bytes();
    QVariant::from(&QColor::from_rgba(r as i32, g as i32, b as i32, a as i32))
}

fn to_map(tones: &Tones) -> QVariantMap {
    let mut map = QVariantMap::default();
    map.insert(QString::from("container"), color(tones.container));
    map.insert(QString::from("onContainer"), color(tones.on_container));
    map.insert(QString::from("onContainerVariant"), color(tones.on_container_variant));
    map
}

impl qobject::CoverTones {
    fn tones(mut self: Pin<&mut Self>, url: &QString, dark: bool) -> QVariantMap {
        let key = url.to_string();
        if key.trim().is_empty() {
            return QVariantMap::default();
        }
        match self.entries.get(&key) {
            Some(Entry::Ready(light, dark_tones)) => return to_map(if dark { dark_tones } else { light }),
            Some(_) => return QVariantMap::default(),
            None => {}
        }
        let thread = self.qt_thread();
        let mut this = self.as_mut().rust_mut();
        if this.entries.len() >= MAX_ENTRIES {
            this.entries.retain(|_, entry| matches!(entry, Entry::Pending));
        }
        this.entries.insert(key.clone(), Entry::Pending);
        let local = local_path(url);
        let remote = local.starts_with("http://") || local.starts_with("https://");
        let http = ctx().services.api.http();
        let rt = ctx().rt.clone();
        ctx().rt.spawn(async move {
            let file = if remote {
                let dir = mf_platform::dirs::cache_subdir("covers");
                sf_player::engine::cover::fetch_cover(&http, &dir, &local).await
            } else {
                Ok(PathBuf::from(&local))
            };
            let result = match file {
                Ok(path) => rt
                    .spawn_blocking(move || mf_theme::cover_tones_from_image(&path).map_err(|e| e.to_string()))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string())),
                Err(error) => Err(error),
            };
            if let Err(error) = &result {
                tracing::debug!("cover tones: {error}");
            }
            let _ = thread.queue(move |tones| tones.landed(key, result.ok()));
        });
        QVariantMap::default()
    }

    fn landed(mut self: Pin<&mut Self>, key: String, result: Option<(Tones, Tones)>) {
        let entry = match result {
            Some((light, dark)) => Entry::Ready(light, dark),
            None => Entry::Failed,
        };
        let mut this = self.as_mut().rust_mut();
        this.entries.insert(key, entry);
        this.revision = this.revision.wrapping_add(1);
        self.as_mut().revision_changed();
    }
}
