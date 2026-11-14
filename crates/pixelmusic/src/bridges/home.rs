// Home — зеркало HomeService: полки главной ({key, title, kind, items:[…]}).
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use md3_model::{HomeShelf, PlayContext, TrackItem, contexts};
use sf_services::home::{HomeMixes, YouTubeHomeSnapshot};
use sf_services::youtube::BrowseRef;
use sf_services::{HomeSnapshot, ServiceError};

use super::{forward_watch, model_ptr, to_qlist, with_model};
use crate::context::ctx;
use crate::models::{DictListModel, ModelKind, to_rows};
use crate::qt::QVariantList;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
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
        /// A YouTube Music shelf's full page ("see all", `browseYouTube`): the same shelf shape as `shelves`
        #[qproperty(QList_QVariant, browse_shelves, cxx_name = "browseShelves", READ, NOTIFY = browse_changed)]
        #[qproperty(bool, browse_loading, cxx_name = "browseLoading", READ, NOTIFY = browse_changed)]
        #[qproperty(QString, browse_error, cxx_name = "browseError", READ, NOTIFY = browse_changed)]
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
        #[qsignal]
        #[cxx_name = "browseChanged"]
        fn browse_changed(self: Pin<&mut Home>);

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
        /// Load the full page of a YouTube Music shelf (its `browseId` / `browseParams`) into `browseShelves`.
        #[qinvokable]
        #[cxx_name = "browseYouTube"]
        fn browse_youtube(self: Pin<&mut Home>, browse_id: &QString, params: &QString);
        /// The first two covers of a mood or genre page (the fanned covers on its Moods & genres card), read in
        /// the background and remembered; `moodCoversLoaded` brings them.
        #[qinvokable]
        #[cxx_name = "moodCovers"]
        fn mood_covers(self: &Home, browse_id: &QString, params: &QString);
        #[qsignal]
        #[cxx_name = "moodCoversLoaded"]
        fn mood_covers_loaded(self: Pin<&mut Home>, key: QString, covers: QStringList);
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
    browse_shelves: QVariantList,
    browse_loading: bool,
    browse_error: QString,
    /// the latest `browseYouTube` request; older answers are dropped
    browse_generation: u64,
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
            browse_shelves: QVariantList::default(),
            browse_loading: false,
            browse_error: QString::default(),
            browse_generation: 0,
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

    fn browse_youtube(mut self: Pin<&mut Self>, browse_id: &QString, params: &QString) {
        let thread = self.qt_thread();
        let target = BrowseRef {
            browse_id: browse_id.to_string(),
            params: Some(params.to_string()).filter(|params| !params.is_empty()),
        };
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.browse_generation += 1;
            this.browse_shelves = QVariantList::default();
            this.browse_error = QString::default();
            this.browse_loading = !target.browse_id.is_empty();
            this.browse_generation
        };
        self.as_mut().browse_changed();
        if target.browse_id.is_empty() {
            return;
        }
        ctx().rt.spawn(async move {
            let result = ctx().services.home.youtube_browse(&target).await;
            let _ = thread.queue(move |home| home.browse_loaded(generation, result));
        });
    }

    fn mood_covers(&self, browse_id: &QString, params: &QString) {
        let target = BrowseRef {
            browse_id: browse_id.to_string(),
            params: Some(params.to_string()).filter(|params| !params.is_empty()),
        };
        if target.browse_id.is_empty() {
            return;
        }
        let key = format!("{}|{}", target.browse_id, target.params.as_deref().unwrap_or_default());
        let thread = self.qt_thread();
        let send = move |thread: cxx_qt::CxxQtThread<qobject::Home>, key: String, covers: Vec<String>| {
            let _ = thread.queue(move |home| {
                let mut list = QStringList::default();
                for cover in &covers {
                    list.append(QString::from(cover.as_str()));
                }
                home.mood_covers_loaded(QString::from(key.as_str()), list);
            });
        };
        if let Some(covers) = mood_cover_cache().lock().ok().and_then(|cache| cache.get(&key).cloned()) {
            return send(thread, key, covers);
        }
        ctx().rt.spawn(async move {
            // a few pages at a time: a whole Moods & genres grid asks for ~40 of them at once
            let Ok(_permit) = mood_cover_gate().acquire().await else { return };
            let covers: Vec<String> = match ctx().services.home.youtube_browse(&target).await {
                Ok(shelves) => shelves
                    .iter()
                    .flat_map(|shelf| shelf.items.iter())
                    .filter_map(|item| item.get("cover").and_then(|cover| cover.as_str()))
                    .filter(|cover| !cover.is_empty())
                    .take(2)
                    .map(str::to_owned)
                    .collect(),
                Err(error) => return tracing::debug!("mood covers {key}: {error}"),
            };
            if let Ok(mut cache) = mood_cover_cache().lock() {
                cache.insert(key.clone(), covers.clone());
            }
            send(thread, key, covers);
        });
    }

    fn browse_loaded(mut self: Pin<&mut Self>, generation: u64, result: Result<Vec<HomeShelf>, ServiceError>) {
        if generation != self.browse_generation {
            return;
        }
        let mut this = self.as_mut().rust_mut();
        this.browse_loading = false;
        match result {
            Ok(shelves) => this.browse_shelves = to_qlist(&shelves),
            Err(error) => this.browse_error = QString::from(error.to_string().as_str()),
        }
        self.as_mut().browse_changed();
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
        let this = self.as_mut().rust_mut();
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

/// Covers of mood / genre pages already read this run ("browseId|params" -> up to two covers).
fn mood_cover_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, Vec<String>>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn mood_cover_gate() -> &'static tokio::sync::Semaphore {
    static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    &GATE
}
