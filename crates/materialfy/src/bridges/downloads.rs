// Downloads — mirror of sf_services::downloads::DownloadsService: downloaded YouTube Music songs (tracks: a Track
// model, context "downloads", rows with their saved covers), the queue's state and the storage use. QML asks
// isDownloaded / isPending / progressOf with `revision` in the binding, so badges follow every change.
use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use md3_model::{PlayContext, PlaylistId};
use mf_platform::{SettingKey, SettingValue};
use sf_services::downloads::{DownloadsSnapshot, JobState};

use super::{forward_watch, local_path, model_ptr, track_from_map, with_model};
use crate::context::ctx;
use crate::models::{DictListModel, ModelKind, to_rows};
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
        #[qproperty(*mut QObject, tracks, READ = tracks_ptr, CONSTANT)]
        /// bumped on every change (bind it next to the is…() calls)
        #[qproperty(i32, revision, READ, NOTIFY = changed)]
        #[qproperty(i32, queued, READ, NOTIFY = changed)]
        #[qproperty(i32, running, READ, NOTIFY = changed)]
        #[qproperty(i32, failed, READ, NOTIFY = changed)]
        /// 0..1 over the songs downloading now
        #[qproperty(f64, progress, READ, NOTIFY = changed)]
        /// "1.2 GB of 4 GB" / "1.2 GB"
        #[qproperty(QString, usage, READ, NOTIFY = changed)]
        #[qproperty(QString, folder, READ, NOTIFY = changed)]
        /// the first failure's reason ("" — none)
        #[qproperty(QString, error_text, cxx_name = "errorText", READ, NOTIFY = changed)]
        type Downloads = super::DownloadsRust;

        fn tracks_ptr(self: &Downloads) -> *mut QObject;

        #[qsignal]
        fn changed(self: Pin<&mut Downloads>);

        /// A track dict from a list or menu (YouTube songs only).
        #[qinvokable]
        fn download(self: &Downloads, track: &QMap_QString_QVariant);
        /// The loaded songs of a library playlist (`youtube:playlist:` URI or LibraryEntry id).
        #[qinvokable]
        #[cxx_name = "downloadPlaylist"]
        fn download_playlist(self: &Downloads, playlist: &QString);
        #[qinvokable]
        #[cxx_name = "removePlaylist"]
        fn remove_playlist(self: &Downloads, playlist: &QString);
        /// Deletes the file (or cancels the download).
        #[qinvokable]
        fn remove(self: &Downloads, uri: &QString);
        #[qinvokable]
        fn cancel(self: &Downloads, uri: &QString);
        #[qinvokable]
        #[cxx_name = "cancelAll"]
        fn cancel_all(self: &Downloads);
        #[qinvokable]
        fn retry(self: &Downloads, uri: &QString);
        #[qinvokable]
        #[cxx_name = "retryFailed"]
        fn retry_failed(self: &Downloads);
        #[qinvokable]
        #[cxx_name = "removeAll"]
        fn remove_all(self: &Downloads);
        /// Deletes songs not played for `days` days; returns the notice text.
        #[qinvokable]
        #[cxx_name = "removeForgotten"]
        fn remove_forgotten(self: &Downloads, days: i32) -> QString;
        /// A folder path or a file:// URL from FolderDialog ("" — the default folder).
        #[qinvokable]
        #[cxx_name = "setFolder"]
        fn set_folder(self: &Downloads, folder: &QString);
        #[qinvokable]
        #[cxx_name = "isDownloaded"]
        fn is_downloaded(self: &Downloads, uri: &QString) -> bool;
        /// Queued or downloading.
        #[qinvokable]
        #[cxx_name = "isPending"]
        fn is_pending(self: &Downloads, uri: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "isFailed"]
        fn is_failed(self: &Downloads, uri: &QString) -> bool;
        /// 0..1 while it downloads, -1 otherwise.
        #[qinvokable]
        #[cxx_name = "progressOf"]
        fn progress_of(self: &Downloads, uri: &QString) -> f64;
        #[qinvokable]
        #[cxx_name = "hasPlaylist"]
        fn has_playlist(self: &Downloads, playlist: &QString) -> bool;
        /// "3 of 12 downloaded" for a downloaded playlist, "" otherwise.
        #[qinvokable]
        #[cxx_name = "playlistStatus"]
        fn playlist_status(self: &Downloads, playlist: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "canDownload"]
        fn can_download(self: &Downloads, uri: &QString) -> bool;
    }

    impl cxx_qt::Threading for Downloads {}
    impl cxx_qt::Initialize for Downloads {}
}

pub struct DownloadsRust {
    snapshot: DownloadsSnapshot,
    tracks: UniquePtr<DictListModel>,
    revision: i32,
    queued: i32,
    running: i32,
    failed: i32,
    progress: f64,
    usage: QString,
    folder: QString,
    error_text: QString,
}

fn downloads_context() -> PlayContext {
    PlayContext::new("downloads").titled(mf_i18n::t("Downloads"))
}

impl Default for DownloadsRust {
    fn default() -> Self {
        let mut tracks = DictListModel::create(ModelKind::Track);
        with_model(&mut tracks, |m| m.set_context(downloads_context()));
        Self {
            snapshot: DownloadsSnapshot::default(),
            tracks,
            revision: 0,
            queued: 0,
            running: 0,
            failed: 0,
            progress: 0.0,
            usage: QString::default(),
            folder: QString::default(),
            error_text: QString::default(),
        }
    }
}

/// 1536 → "1.5 KB", in powers of 1024.
fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", units[unit]) }
}

fn playlist_uri(playlist: &QString) -> String {
    PlaylistId::from_uri_or_id(&playlist.to_string()).uri()
}

impl qobject::Downloads {
    fn tracks_ptr(&self) -> *mut cxx_qt::QObject {
        model_ptr(&self.tracks)
    }

    fn download(&self, track: &QVariantMap) {
        if let Some(track) = track_from_map(track) {
            ctx().services.downloads.download(track);
        }
    }

    fn download_playlist(&self, playlist: &QString) {
        let services = &ctx().services;
        let id = PlaylistId::from_uri_or_id(&playlist.to_string());
        let tracks = services.library.playlist_tracks(&id).unwrap_or_default();
        let info = services.library.playlist_info(&id);
        let (title, cover) = info.map(|info| (info.item.title.clone(), info.item.cover.clone())).unwrap_or_default();
        services.downloads.download_playlist(&id.uri(), &title, &cover, tracks);
    }

    fn remove_playlist(&self, playlist: &QString) {
        ctx().services.downloads.remove_playlist(&playlist_uri(playlist));
    }

    fn remove(&self, uri: &QString) {
        ctx().services.downloads.remove(&uri.to_string());
    }

    fn cancel(&self, uri: &QString) {
        ctx().services.downloads.cancel(&uri.to_string());
    }

    fn cancel_all(&self) {
        ctx().services.downloads.cancel_all();
    }

    fn retry(&self, uri: &QString) {
        ctx().services.downloads.retry(&uri.to_string());
    }

    fn retry_failed(&self) {
        ctx().services.downloads.retry_failed();
    }

    fn remove_all(&self) {
        ctx().services.downloads.remove_all();
    }

    fn remove_forgotten(&self, days: i32) -> QString {
        let (songs, bytes) = ctx().services.downloads.remove_forgotten(days.max(0) as u32);
        let text = if songs == 0 {
            mf_i18n::t("No songs to clean up")
        } else {
            mf_i18n::tf("Removed {0} songs, freed {1}", &[&songs, &format_bytes(bytes)])
        };
        QString::from(text.as_str())
    }

    fn set_folder(&self, folder: &QString) {
        let path = if folder.is_empty() { String::new() } else { local_path(folder) };
        ctx().services.settings.set(SettingKey::DownloadsFolder, SettingValue::Str(path));
    }

    fn is_downloaded(&self, uri: &QString) -> bool {
        self.snapshot.is_downloaded(&uri.to_string())
    }

    fn is_pending(&self, uri: &QString) -> bool {
        self.snapshot.is_pending(&uri.to_string())
    }

    fn is_failed(&self, uri: &QString) -> bool {
        self.snapshot.job(&uri.to_string()).is_some_and(|job| matches!(job.state, JobState::Failed(_)))
    }

    fn progress_of(&self, uri: &QString) -> f64 {
        match self.snapshot.job(&uri.to_string()) {
            Some(job) if job.state == JobState::Running => job.progress(),
            Some(job) if job.state == JobState::Queued => 0.0,
            _ => -1.0,
        }
    }

    fn has_playlist(&self, playlist: &QString) -> bool {
        self.snapshot.has_playlist(&playlist_uri(playlist))
    }

    fn playlist_status(&self, playlist: &QString) -> QString {
        let uri = playlist_uri(playlist);
        if !self.snapshot.has_playlist(&uri) {
            return QString::default();
        }
        let (done, all) = self.snapshot.playlist_progress(&uri);
        let text = mf_i18n::tf("{0} of {1} downloaded", &[&done, &all]);
        QString::from(text.as_str())
    }

    fn can_download(&self, uri: &QString) -> bool {
        sf_player::engine::youtube::is_youtube_track(&uri.to_string())
    }

    fn apply(mut self: Pin<&mut Self>, next: DownloadsSnapshot) {
        let prev = std::mem::replace(&mut self.as_mut().rust_mut().snapshot, next.clone());
        let mut this = self.as_mut().rust_mut();
        if prev.tracks != next.tracks || this.revision == 0 {
            let items: Vec<_> = next.tracks.iter().map(|track| track.item()).collect();
            with_model(&mut this.tracks, |mut m| {
                m.as_mut().reset_rows(to_rows(&items));
                m.set_loading(false);
            });
        }
        let count = |wanted: fn(&JobState) -> bool| next.jobs.iter().filter(|job| wanted(&job.state)).count() as i32;
        this.queued = count(|state| *state == JobState::Queued);
        this.running = count(|state| *state == JobState::Running);
        this.failed = count(|state| matches!(state, JobState::Failed(_)));
        let active: Vec<_> = next.jobs.iter().filter(|job| job.state == JobState::Running).collect();
        this.progress =
            if active.is_empty() { 0.0 } else { active.iter().map(|job| job.progress()).sum::<f64>() / active.len() as f64 };
        let used = format_bytes(next.bytes);
        let usage = if next.limit_bytes > 0 {
            mf_i18n::tf("{0} of {1}", &[&used, &format_bytes(next.limit_bytes)])
        } else {
            used
        };
        this.usage = QString::from(usage.as_str());
        this.folder = QString::from(next.folder.as_str());
        let error = next.jobs.iter().find_map(|job| match &job.state {
            JobState::Failed(error) => Some(error.clone()),
            _ => None,
        });
        this.error_text = QString::from(error.unwrap_or_default().as_str());
        this.revision = this.revision.wrapping_add(1).max(1);
        self.as_mut().changed();
    }
}

impl cxx_qt::Initialize for qobject::Downloads {
    fn initialize(mut self: Pin<&mut Self>) {
        let mut watch = ctx().services.downloads.watch();
        let snapshot = watch.borrow_and_update().clone();
        self.as_mut().apply(snapshot);
        forward_watch(self.qt_thread(), watch, Self::apply);
    }
}
