// SmartMix — mirror of LastFmFeedService: the Last.fm feed (recent: RecentTrack keys title/artist/album/cover/
// dateText; friends: name/displayName/avatar; topAlbums: title/artist/cover/playCount) and the generated Smart Mix
// (tracks: a track model, title, cover). The account is the one connected on the Settings card (LastFm).
// covers: art found on YouTube Music for feed tiles Last.fm has none for, keyed "track|artist|title" /
// "album|artist|title" in lower case; a tile asks with requestCover when it is created.
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use md3_model::{PlayContext, TrackItem, contexts};
use sf_services::lastfm_feed::{CoverKind, LastFmFeedSnapshot, MixMode, MixRequest, MixTarget};

use super::{forward_watch, model_ptr, to_qlist, to_qmap, with_model};
use crate::context::ctx;
use crate::models::{DictListModel, ModelKind, to_rows};
use crate::qt::{QVariantList, QVariantMap};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
        include!("cxx-qt-lib/qmap.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qproperty(bool, feed_loading, cxx_name = "feedLoading", READ, NOTIFY = feed_changed)]
        #[qproperty(QList_QVariant, recent, READ, NOTIFY = feed_changed)]
        #[qproperty(QList_QVariant, friends, READ, NOTIFY = feed_changed)]
        #[qproperty(QList_QVariant, top_albums, cxx_name = "topAlbums", READ, NOTIFY = feed_changed)]
        #[qproperty(QString, feed_error, cxx_name = "feedError", READ, NOTIFY = feed_changed)]
        #[qproperty(bool, generating, READ, NOTIFY = mix_changed)]
        #[qproperty(QString, progress, READ, NOTIFY = mix_changed)]
        #[qproperty(QString, mix_title, cxx_name = "mixTitle", READ, NOTIFY = mix_changed)]
        #[qproperty(QString, mix_mode, cxx_name = "mixMode", READ, NOTIFY = mix_changed)]
        #[qproperty(QString, mix_cover, cxx_name = "mixCover", READ, NOTIFY = mix_changed)]
        #[qproperty(i32, track_count, cxx_name = "trackCount", READ, NOTIFY = mix_changed)]
        #[qproperty(QString, mix_error, cxx_name = "mixError", READ, NOTIFY = mix_changed)]
        #[qproperty(QMap_QString_QVariant, covers, READ, NOTIFY = covers_changed)]
        #[qproperty(*mut QObject, tracks, READ = tracks_ptr, CONSTANT)]
        type SmartMix = super::SmartMixRust;

        fn tracks_ptr(self: &SmartMix) -> *mut QObject;

        #[qsignal]
        #[cxx_name = "feedChanged"]
        fn feed_changed(self: Pin<&mut SmartMix>);
        #[qsignal]
        #[cxx_name = "mixChanged"]
        fn mix_changed(self: Pin<&mut SmartMix>);
        #[qsignal]
        #[cxx_name = "coversChanged"]
        fn covers_changed(self: Pin<&mut SmartMix>);

        /// Recent tracks, friends and top albums of the connected Last.fm user.
        #[qinvokable]
        #[cxx_name = "refreshFeed"]
        fn refresh_feed(self: &SmartMix);
        /// mode: top | recent | similar-tracks | similar-artists | tag | mix | recommendations | library;
        /// period: overall | 12month | 6month | 3month | 1month | 7day; target: youtube | spotify.
        #[qinvokable]
        fn generate(
            self: &SmartMix,
            mode: &QString,
            count: i32,
            period: &QString,
            seed_track: &QString,
            seed_artist: &QString,
            seed_artist_input: &QString,
            tag: &QString,
            target: &QString,
        );
        #[qinvokable]
        fn cancel(self: &SmartMix);
        /// Save the generated mix as a playlist (YouTube Music or Spotify, after its tracks).
        #[qinvokable]
        fn save(self: &SmartMix);
        /// A feed row: find the track in the catalog and play it.
        #[qinvokable]
        #[cxx_name = "playTrack"]
        fn play_track(self: &SmartMix, title: &QString, artist: &QString, target: &QString);
        /// A feed tile without art: look its cover up on YouTube Music (kind: track | album). Once per tile and
        /// run; a few at a time. The result shows up in `covers`.
        #[qinvokable]
        #[cxx_name = "requestCover"]
        fn request_cover(self: &SmartMix, kind: &QString, title: &QString, artist: &QString);
    }

    impl cxx_qt::Threading for SmartMix {}
    impl cxx_qt::Initialize for SmartMix {}
}

pub struct SmartMixRust {
    snapshot: LastFmFeedSnapshot,
    feed_loading: bool,
    recent: QVariantList,
    friends: QVariantList,
    top_albums: QVariantList,
    feed_error: QString,
    generating: bool,
    progress: QString,
    mix_title: QString,
    mix_mode: QString,
    mix_cover: QString,
    track_count: i32,
    mix_error: QString,
    covers: QVariantMap,
    tracks: UniquePtr<DictListModel>,
}

impl Default for SmartMixRust {
    fn default() -> Self {
        let mut tracks = DictListModel::create(ModelKind::Track);
        with_model(&mut tracks, |m| m.set_context(PlayContext::new(contexts::TRACKS)));
        Self {
            snapshot: LastFmFeedSnapshot::default(),
            feed_loading: false,
            recent: QVariantList::default(),
            friends: QVariantList::default(),
            top_albums: QVariantList::default(),
            feed_error: QString::default(),
            generating: false,
            progress: QString::default(),
            mix_title: QString::default(),
            mix_mode: QString::default(),
            mix_cover: QString::default(),
            track_count: 0,
            mix_error: QString::default(),
            covers: QVariantMap::default(),
            tracks,
        }
    }
}

impl qobject::SmartMix {
    fn tracks_ptr(&self) -> *mut cxx_qt::QObject {
        model_ptr(&self.tracks)
    }

    fn refresh_feed(&self) {
        ctx().services.lastfm_feed.refresh_feed();
    }

    #[allow(clippy::too_many_arguments)]
    fn generate(
        &self,
        mode: &QString,
        count: i32,
        period: &QString,
        seed_track: &QString,
        seed_artist: &QString,
        seed_artist_input: &QString,
        tag: &QString,
        target: &QString,
    ) {
        let request = MixRequest {
            mode: MixMode::from_key(&mode.to_string()).unwrap_or_default(),
            count: count.clamp(5, 100) as usize,
            period: period.to_string(),
            seed_track: seed_track.to_string(),
            seed_artist: seed_artist.to_string(),
            seed_artist_input: seed_artist_input.to_string(),
            tag: tag.to_string(),
        };
        let services = &ctx().services;
        let existing: Vec<String> = services.library.playlists().into_iter().map(|playlist| playlist.title).collect();
        services.lastfm_feed.generate(request, MixTarget::from_key(&target.to_string()), existing);
    }

    fn cancel(&self) {
        ctx().services.lastfm_feed.cancel();
    }

    fn save(&self) {
        let snapshot = ctx().services.lastfm_feed.snapshot();
        let uris: Vec<String> = snapshot.mix_tracks.iter().map(TrackItem::play_uri).collect();
        if !uris.is_empty() {
            ctx().services.library.save_smart_mix(snapshot.mix_title, uris);
        }
    }

    fn play_track(&self, title: &QString, artist: &QString, target: &QString) {
        let target = MixTarget::from_key(&target.to_string());
        ctx().services.lastfm_feed.play_track(title.to_string(), artist.to_string(), target);
    }

    fn request_cover(&self, kind: &QString, title: &QString, artist: &QString) {
        let kind = CoverKind::from_key(&kind.to_string());
        ctx().services.lastfm_feed.request_cover(kind, &title.to_string(), &artist.to_string());
    }

    fn apply(mut self: Pin<&mut Self>, next: LastFmFeedSnapshot) {
        let prev = std::mem::replace(&mut self.as_mut().rust_mut().snapshot, next.clone());
        if prev == next {
            return;
        }
        let feed = prev.feed_loading != next.feed_loading
            || prev.feed_error != next.feed_error
            || !Arc::ptr_eq(&prev.recent, &next.recent)
            || !Arc::ptr_eq(&prev.friends, &next.friends)
            || !Arc::ptr_eq(&prev.top_albums, &next.top_albums);
        if feed {
            let mut this = self.as_mut().rust_mut();
            this.feed_loading = next.feed_loading;
            this.recent = to_qlist(&next.recent);
            this.friends = to_qlist(&next.friends);
            this.top_albums = to_qlist(&next.top_albums);
            this.feed_error = QString::from(next.feed_error.as_str());
            self.as_mut().feed_changed();
        }
        if !Arc::ptr_eq(&prev.covers, &next.covers) {
            self.as_mut().rust_mut().covers = to_qmap(&*next.covers);
            self.as_mut().covers_changed();
        }
        if !Arc::ptr_eq(&prev.mix_tracks, &next.mix_tracks) && prev.mix_tracks != next.mix_tracks {
            let title = next.mix_title.clone();
            with_model(&mut self.as_mut().rust_mut().get_mut().tracks, |mut m| {
                m.as_mut().set_context(PlayContext::new(contexts::TRACKS).titled(title));
                m.reset_rows(to_rows(&next.mix_tracks));
            });
        }
        let mix = prev.generating != next.generating
            || prev.progress != next.progress
            || prev.mix_title != next.mix_title
            || prev.mix_mode != next.mix_mode
            || prev.mix_cover != next.mix_cover
            || prev.mix_error != next.mix_error
            || prev.mix_tracks.len() != next.mix_tracks.len();
        if mix {
            let mut this = self.as_mut().rust_mut();
            this.generating = next.generating;
            this.progress = QString::from(next.progress.as_str());
            this.mix_title = QString::from(next.mix_title.as_str());
            this.mix_mode = QString::from(next.mix_mode.as_str());
            this.mix_cover = QString::from(next.mix_cover.as_str());
            this.track_count = i32::try_from(next.mix_tracks.len()).unwrap_or(i32::MAX);
            this.mix_error = QString::from(next.mix_error.as_str());
            self.as_mut().mix_changed();
        }
    }
}

impl cxx_qt::Initialize for qobject::SmartMix {
    fn initialize(mut self: Pin<&mut Self>) {
        let mut watch = ctx().services.lastfm_feed.watch();
        let snapshot = watch.borrow_and_update().clone();
        // the first snapshot is published whole (prev == default would drop an empty one)
        self.as_mut().rust_mut().snapshot = LastFmFeedSnapshot { generating: !snapshot.generating, ..Default::default() };
        self.as_mut().apply(snapshot);
        forward_watch(self.qt_thread(), watch, Self::apply);
    }
}
