// QueueHistory: saved past queues (PixelPlayer's "Queue history" / "Replay queue"). The Player bridge feeds every
// queue snapshot to the service (`service().observe`), this object lists the saved queues for QML.
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use cxx_qt::{CxxQtType, Threading};
use md3_model::{PlayContext, contexts};
use serde_json::{Value, json};
use sf_services::queue_history::{QueueHistoryEvent, QueueHistoryService, SavedQueue};

use super::{Delivery, forward_events};
use crate::context::ctx;
use crate::qt::{QVariantList, json_to_qvariantlist};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
    }

    unsafe extern "RustQt" {
        /// [{id, savedAt (unix ms), title, count, first: {title, artists, cover}, covers: [≤ 3 distinct covers],
        ///   durationMs}], newest first
        #[qobject]
        #[qproperty(QList_QVariant, queues, READ, NOTIFY = changed)]
        #[qproperty(i32, count, READ, NOTIFY = changed)]
        type QueueHistory = super::QueueHistoryRust;

        #[qsignal]
        fn changed(self: Pin<&mut QueueHistory>);

        /// Play the saved queue from its first song.
        #[qinvokable]
        fn replay(self: &QueueHistory, id: f64);

        #[qinvokable]
        fn remove(self: &QueueHistory, id: f64);

        #[qinvokable]
        fn clear(self: &QueueHistory);
    }

    impl cxx_qt::Threading for QueueHistory {}
    impl cxx_qt::Initialize for QueueHistory {}
}

#[derive(Default)]
pub struct QueueHistoryRust {
    queues: QVariantList,
    count: i32,
}

/// The service (one per process; the Player bridge reports queue snapshots to it).
pub(crate) fn service() -> &'static Arc<QueueHistoryService> {
    static SERVICE: OnceLock<Arc<QueueHistoryService>> = OnceLock::new();
    SERVICE.get_or_init(|| QueueHistoryService::new(ctx().rt.clone()))
}

/// The song's cover; a YouTube song saved without one (a queue filled by ids, an older file) gets its video
/// thumbnail (`mqdefault`: 16:9 without the letterbox bars of `hqdefault`, so a square crop looks clean).
fn cover_of(track: &md3_model::TrackItem) -> String {
    match track.uri.strip_prefix("youtube:track:") {
        Some(id) if track.cover.is_empty() && !id.is_empty() => format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"),
        _ => track.cover.clone(),
    }
}

fn entry(queue: &SavedQueue) -> Value {
    let first = queue.tracks.first().cloned().unwrap_or_default();
    let mut covers: Vec<String> = Vec::new();
    for track in &queue.tracks {
        let cover = cover_of(track);
        if !cover.is_empty() && !covers.contains(&cover) {
            covers.push(cover);
            if covers.len() == 3 {
                break;
            }
        }
    }
    json!({
        "id": queue.id as f64,
        "savedAt": queue.saved_at as f64,
        "title": queue.title,
        "count": queue.tracks.len(),
        "first": {"title": first.title, "artists": first.artists, "cover": cover_of(&first)},
        "covers": covers,
        "durationMs": queue.tracks.iter().map(|t| t.duration_ms.max(0)).sum::<i64>() as f64,
    })
}

impl qobject::QueueHistory {
    fn replay(&self, id: f64) {
        let Some(queue) = service().get(id as u64) else { return };
        let context = PlayContext { title: queue.title.clone(), ..PlayContext::new(contexts::TRACKS) };
        ctx().services.player.play_items(queue.tracks, 0, context);
    }

    fn remove(&self, id: f64) {
        service().remove(id as u64);
    }

    fn clear(&self) {
        service().clear();
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let saved = service().saved();
        let items: Vec<Value> = saved.iter().map(entry).collect();
        let mut this = self.as_mut().rust_mut();
        this.queues = json_to_qvariantlist(&items);
        this.count = i32::try_from(saved.len()).unwrap_or(i32::MAX);
        self.changed();
    }

    fn on_event(self: Pin<&mut Self>, _: Delivery<QueueHistoryEvent>) {
        self.refresh();
    }
}

impl cxx_qt::Initialize for qobject::QueueHistory {
    fn initialize(mut self: Pin<&mut Self>) {
        forward_events(self.qt_thread(), service().subscribe(), Self::on_event);
        self.as_mut().refresh();
    }
}
