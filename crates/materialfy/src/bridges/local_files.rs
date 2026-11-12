// LocalFiles — mirror of sf_services::local::LocalFilesService: music files from the folders chosen in
// Settings (tracks: a Track model with `local:track:` URIs that play through the YouTube lane), the folder
// list and the scan's progress. Folders live in the LocalFolders setting; changing them rescans.
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use md3_model::PlayContext;
use mf_platform::{SettingKey, SettingValue};
use sf_services::local::{LocalSnapshot, folders_from_setting, folders_to_setting, to_item};

use super::{forward_watch, local_path, model_ptr, with_model};
use crate::context::ctx;
use crate::models::{DictListModel, ModelKind, to_rows};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qproperty(*mut QObject, tracks, READ = tracks_ptr, CONSTANT)]
        #[qproperty(QStringList, folders, READ, NOTIFY = folders_changed)]
        #[qproperty(bool, scanning, READ, NOTIFY = scan_changed)]
        #[qproperty(i32, found, READ, NOTIFY = scan_changed)]
        #[qproperty(i32, read, READ, NOTIFY = scan_changed)]
        #[qproperty(QString, error_text, cxx_name = "errorText", READ, NOTIFY = scan_changed)]
        type LocalFiles = super::LocalFilesRust;

        fn tracks_ptr(self: &LocalFiles) -> *mut QObject;

        #[qsignal]
        #[cxx_name = "foldersChanged"]
        fn folders_changed(self: Pin<&mut LocalFiles>);
        #[qsignal]
        #[cxx_name = "scanChanged"]
        fn scan_changed(self: Pin<&mut LocalFiles>);

        /// Adds a folder (a path or a file:// URL from FolderDialog) and scans it.
        #[qinvokable]
        #[cxx_name = "addFolder"]
        fn add_folder(self: &LocalFiles, folder: &QString);
        #[qinvokable]
        #[cxx_name = "removeFolder"]
        fn remove_folder(self: &LocalFiles, folder: &QString);
        /// Reads every file again.
        #[qinvokable]
        fn rescan(self: &LocalFiles);
    }

    impl cxx_qt::Threading for LocalFiles {}
    impl cxx_qt::Initialize for LocalFiles {}
}

pub struct LocalFilesRust {
    snapshot: LocalSnapshot,
    tracks: UniquePtr<DictListModel>,
    folders: QStringList,
    scanning: bool,
    found: i32,
    read: i32,
    error_text: QString,
}

fn local_context() -> PlayContext {
    PlayContext::new("local").titled(mf_i18n::t("Local files"))
}

impl Default for LocalFilesRust {
    fn default() -> Self {
        let mut tracks = DictListModel::create(ModelKind::Track);
        with_model(&mut tracks, |m| m.set_context(local_context()));
        Self {
            snapshot: LocalSnapshot::default(),
            tracks,
            folders: QStringList::default(),
            scanning: false,
            found: 0,
            read: 0,
            error_text: QString::default(),
        }
    }
}

fn set_folders(folders: Vec<String>) {
    let value = SettingValue::Str(folders_to_setting(&folders));
    ctx().services.settings.set(SettingKey::LocalFolders, value);
}

impl qobject::LocalFiles {
    fn tracks_ptr(&self) -> *mut cxx_qt::QObject {
        model_ptr(&self.tracks)
    }

    fn add_folder(&self, folder: &QString) {
        let path = local_path(folder);
        let mut folders = folders_from_setting(&ctx().services.settings.get_str(SettingKey::LocalFolders));
        folders.push(path);
        set_folders(folders);
    }

    fn remove_folder(&self, folder: &QString) {
        let path = local_path(folder);
        let mut folders = folders_from_setting(&ctx().services.settings.get_str(SettingKey::LocalFolders));
        folders.retain(|kept| *kept != path);
        set_folders(folders);
    }

    fn rescan(&self) {
        ctx().services.local.rescan();
    }

    fn apply(mut self: Pin<&mut Self>, next: LocalSnapshot) {
        let prev = std::mem::replace(&mut self.as_mut().rust_mut().snapshot, next.clone());
        let tracks = !Arc::ptr_eq(&prev.tracks, &next.tracks) || prev.loaded != next.loaded;
        let folders = prev.folders != next.folders;
        let scan = prev.scanning != next.scanning || prev.found != next.found || prev.read != next.read
            || prev.error != next.error;
        let mut this = self.as_mut().rust_mut();
        if tracks {
            let items: Vec<_> = next.tracks.iter().map(to_item).collect();
            with_model(&mut this.tracks, |mut m| {
                m.as_mut().reset_rows(to_rows(&items));
                m.set_loading(!next.loaded);
            });
        }
        if folders {
            this.folders = next.folders.iter().map(|folder| QString::from(folder.as_str())).collect();
        }
        this.scanning = next.scanning;
        this.found = next.found as i32;
        this.read = next.read as i32;
        this.error_text = QString::from(next.error.as_str());
        if folders {
            self.as_mut().folders_changed();
        }
        if scan {
            self.as_mut().scan_changed();
        }
    }
}

impl cxx_qt::Initialize for qobject::LocalFiles {
    fn initialize(mut self: Pin<&mut Self>) {
        let mut watch = ctx().services.local.subscribe();
        let snapshot = watch.borrow_and_update().clone();
        // the first snapshot in full (a diff against the default would skip an empty one)
        self.as_mut().rust_mut().snapshot = LocalSnapshot { loaded: !snapshot.loaded, ..Default::default() };
        self.as_mut().apply(snapshot);
        forward_watch(self.qt_thread(), watch, Self::apply);
    }
}
