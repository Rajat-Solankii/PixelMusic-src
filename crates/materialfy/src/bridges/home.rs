// Home — зеркало HomeService: полки главной ({key, title, kind, items:[…]}).
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use sf_services::HomeSnapshot;
use sf_services::home::YouTubeHomeSnapshot;

use super::{forward_watch, to_qlist};
use crate::context::ctx;
use crate::qt::QVariantList;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        // Свой сигнал у каждого свойства: смена loading/errorText не должна пересобирать полки в QML
        #[qproperty(QList_QVariant, shelves, READ, NOTIFY = shelves_changed)]
        #[qproperty(bool, loading, READ, NOTIFY = loading_changed)]
        #[qproperty(QString, error_text, cxx_name = "errorText", READ, NOTIFY = error_text_changed)]
        /// YouTube Music home (works without an account): the same shelf shape as `shelves`
        #[qproperty(QList_QVariant, youtube_shelves, cxx_name = "youtubeShelves", READ, NOTIFY = youtube_shelves_changed)]
        #[qproperty(bool, youtube_loading, cxx_name = "youtubeLoading", READ, NOTIFY = youtube_changed)]
        #[qproperty(bool, youtube_has_more, cxx_name = "youtubeHasMore", READ, NOTIFY = youtube_changed)]
        #[qproperty(QString, youtube_error, cxx_name = "youtubeError", READ, NOTIFY = youtube_changed)]
        type Home = super::HomeRust;

        #[qsignal]
        #[cxx_name = "shelvesChanged"]
        fn shelves_changed(self: Pin<&mut Home>);
        #[qsignal]
        #[cxx_name = "loadingChanged"]
        fn loading_changed(self: Pin<&mut Home>);
        #[qsignal]
        #[cxx_name = "errorTextChanged"]
        fn error_text_changed(self: Pin<&mut Home>);
        #[qsignal]
        #[cxx_name = "youtubeShelvesChanged"]
        fn youtube_shelves_changed(self: Pin<&mut Home>);
        #[qsignal]
        #[cxx_name = "youtubeChanged"]
        fn youtube_changed(self: Pin<&mut Home>);

        /// Обновить полки (force — без 10-минутного ограничения).
        #[qinvokable]
        fn refresh(self: &Home, force: bool);
        /// Load the YouTube Music home (cached for a minute unless force).
        #[qinvokable]
        #[cxx_name = "refreshYouTube"]
        fn refresh_youtube(self: &Home, force: bool);
        /// Next YouTube Music home shelves (youtubeHasMore).
        #[qinvokable]
        #[cxx_name = "moreYouTube"]
        fn more_youtube(self: &Home);
    }

    impl cxx_qt::Threading for Home {}
    impl cxx_qt::Initialize for Home {}
}

#[derive(Default)]
pub struct HomeRust {
    snapshot: HomeSnapshot,
    shelves: QVariantList,
    loading: bool,
    error_text: QString,
    youtube: YouTubeHomeSnapshot,
    youtube_shelves: QVariantList,
    youtube_loading: bool,
    youtube_has_more: bool,
    youtube_error: QString,
}

impl qobject::Home {
    fn refresh(&self, force: bool) {
        ctx().services.home.refresh(force);
    }

    fn refresh_youtube(&self, force: bool) {
        ctx().services.home.refresh_youtube(force);
    }

    fn more_youtube(&self) {
        ctx().services.home.more_youtube();
    }

    fn apply_youtube(mut self: Pin<&mut Self>, next: YouTubeHomeSnapshot) {
        let prev = std::mem::replace(&mut self.as_mut().rust_mut().youtube, next.clone());
        if prev == next {
            return;
        }
        let shelves = !Arc::ptr_eq(&prev.shelves, &next.shelves) && prev.shelves != next.shelves;
        let rest = prev.loading != next.loading || prev.has_more != next.has_more || prev.error_text != next.error_text;
        let mut this = self.as_mut().rust_mut();
        if shelves {
            this.youtube_shelves = to_qlist(&next.shelves);
        }
        this.youtube_loading = next.loading;
        this.youtube_has_more = next.has_more;
        this.youtube_error = QString::from(next.error_text.as_str());
        if shelves {
            self.as_mut().youtube_shelves_changed();
        }
        if rest {
            self.as_mut().youtube_changed();
        }
    }

    fn apply(mut self: Pin<&mut Self>, next: HomeSnapshot) {
        let prev = std::mem::replace(&mut self.as_mut().rust_mut().snapshot, next.clone());
        if prev == next {
            return;
        }
        // полки сравниваются по содержимому: сервис может прислать новый Arc с теми же полками
        let shelves = !Arc::ptr_eq(&prev.shelves, &next.shelves) && prev.shelves != next.shelves;
        let loading = prev.loading != next.loading;
        let error = prev.error_text != next.error_text;
        let mut this = self.as_mut().rust_mut();
        if shelves {
            this.shelves = to_qlist(&next.shelves);
        }
        this.loading = next.loading;
        if error {
            this.error_text = QString::from(next.error_text.as_str());
        }
        if shelves {
            self.as_mut().shelves_changed();
        }
        if loading {
            self.as_mut().loading_changed();
        }
        if error {
            self.as_mut().error_text_changed();
        }
    }
}

impl cxx_qt::Initialize for qobject::Home {
    fn initialize(mut self: Pin<&mut Self>) {
        let mut watch = ctx().services.home.watch();
        let snapshot = watch.borrow_and_update().clone();
        self.as_mut().apply(snapshot);
        forward_watch(self.qt_thread(), watch, Self::apply);
        let mut youtube = ctx().services.home.watch_youtube();
        let snapshot = youtube.borrow_and_update().clone();
        self.as_mut().apply_youtube(snapshot);
        forward_watch(self.qt_thread(), youtube, Self::apply_youtube);
    }
}
