// Home — зеркало HomeService: полки главной ({key, title, kind, items:[…]}).
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use md3_model::{PlayContext, TrackItem, contexts};
use sf_services::HomeSnapshot;
use sf_services::home::{HomeMixes, YouTubeHomeSnapshot};

use super::{forward_watch, model_ptr, to_qlist, with_model};
use crate::context::ctx;
use crate::models::{DictListModel, ModelKind, to_rows};
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
        /// Mixes from the local listening history (empty until something was played): track dictionaries for
        /// the cards on Home, and the same tracks as models (Player.playFrom / shuffleFrom, the "see all" page)
        #[qproperty(QList_QVariant, recently_played, cxx_name = "recentlyPlayed", READ, NOTIFY = mixes_changed)]
        #[qproperty(QList_QVariant, daily_mix, cxx_name = "dailyMix", READ, NOTIFY = mixes_changed)]
        #[qproperty(QList_QVariant, your_mix, cxx_name = "yourMix", READ, NOTIFY = mixes_changed)]
        #[qproperty(*mut QObject, recent_tracks, cxx_name = "recentTracks", READ = recent_ptr, CONSTANT)]
        #[qproperty(*mut QObject, daily_mix_tracks, cxx_name = "dailyMixTracks", READ = daily_ptr, CONSTANT)]
        #[qproperty(*mut QObject, your_mix_tracks, cxx_name = "yourMixTracks", READ = your_ptr, CONSTANT)]
        type Home = super::HomeRust;

        fn recent_ptr(self: &Home) -> *mut QObject;
        fn daily_ptr(self: &Home) -> *mut QObject;
        fn your_ptr(self: &Home) -> *mut QObject;

        #[qsignal]
        #[cxx_name = "mixesChanged"]
        fn mixes_changed(self: Pin<&mut Home>);

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
    mixes: HomeMixes,
    recently_played: QVariantList,
    daily_mix: QVariantList,
    your_mix: QVariantList,
    recent_tracks: UniquePtr<DictListModel>,
    daily_mix_tracks: UniquePtr<DictListModel>,
    your_mix_tracks: UniquePtr<DictListModel>,
}

/// A track model that plays as a list of its own ("Playing from Daily Mix").
fn mix_model(title: &str) -> UniquePtr<DictListModel> {
    let mut model = DictListModel::create(ModelKind::Track);
    with_model(&mut model, |m| m.set_context(PlayContext::new(contexts::TRACKS).titled(mf_i18n::t(title))));
    model
}

impl Default for HomeRust {
    fn default() -> Self {
        Self {
            snapshot: HomeSnapshot::default(),
            shelves: QVariantList::default(),
            loading: false,
            error_text: QString::default(),
            youtube: YouTubeHomeSnapshot::default(),
            youtube_shelves: QVariantList::default(),
            youtube_loading: false,
            youtube_has_more: false,
            youtube_error: QString::default(),
            mixes: HomeMixes::default(),
            recently_played: QVariantList::default(),
            daily_mix: QVariantList::default(),
            your_mix: QVariantList::default(),
            recent_tracks: mix_model("Recently played"),
            daily_mix_tracks: mix_model("Daily Mix"),
            your_mix_tracks: mix_model("Your Mix"),
        }
    }
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

    fn recent_ptr(&self) -> *mut cxx_qt::QObject {
        model_ptr(&self.recent_tracks)
    }

    fn daily_ptr(&self) -> *mut cxx_qt::QObject {
        model_ptr(&self.daily_mix_tracks)
    }

    fn your_ptr(&self) -> *mut cxx_qt::QObject {
        model_ptr(&self.your_mix_tracks)
    }

    fn apply_mixes(mut self: Pin<&mut Self>, next: HomeMixes) {
        let prev = std::mem::replace(&mut self.as_mut().rust_mut().mixes, next.clone());
        if prev == next {
            return;
        }
        let changed = |prev: &Arc<Vec<TrackItem>>, next: &Arc<Vec<TrackItem>>| !Arc::ptr_eq(prev, next) && prev != next;
        let mut this = self.as_mut().rust_mut();
        let this = this.get_mut();
        if changed(&prev.recent, &next.recent) {
            this.recently_played = to_qlist(&next.recent);
            with_model(&mut this.recent_tracks, |m| m.reset_rows(to_rows(&next.recent)));
        }
        if changed(&prev.daily, &next.daily) {
            this.daily_mix = to_qlist(&next.daily);
            with_model(&mut this.daily_mix_tracks, |m| m.reset_rows(to_rows(&next.daily)));
        }
        if changed(&prev.your, &next.your) {
            this.your_mix = to_qlist(&next.your);
            with_model(&mut this.your_mix_tracks, |m| m.reset_rows(to_rows(&next.your)));
        }
        self.as_mut().mixes_changed();
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
        let mut mixes = ctx().services.home.watch_mixes();
        let snapshot = mixes.borrow_and_update().clone();
        self.as_mut().apply_mixes(snapshot);
        forward_watch(self.qt_thread(), mixes, Self::apply_mixes);
    }
}
