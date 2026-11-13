// Lyrics (lyrics.py) — зеркало LyricsService: одно changed() на каждое изменение снимка.
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use sf_services::LyricsSnapshot;

use super::{forward_watch, local_path, to_qlist};
use crate::context::ctx;
use crate::qt::{QVariantList, json_to_qvariantlist};

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
        #[qproperty(QList_QVariant, lines, READ, NOTIFY = changed)]
        #[qproperty(QString, track_id, cxx_name = "trackId", READ, NOTIFY = changed)]
        #[qproperty(QString, status, READ, NOTIFY = changed)]
        #[qproperty(QString, source, READ, NOTIFY = changed)]
        #[qproperty(QString, kind, READ, NOTIFY = changed)]
        /// Search Lyrics dialog: [{id, title, artist, album, duration, synced, plain, instrumental}]
        #[qproperty(QList_QVariant, search_results, cxx_name = "searchResults", READ, NOTIFY = search_changed)]
        #[qproperty(bool, searching, READ, NOTIFY = search_changed)]
        #[qproperty(QString, search_error, cxx_name = "searchError", READ, NOTIFY = search_changed)]
        type Lyrics = super::LyricsRust;

        #[qsignal]
        fn changed(self: Pin<&mut Lyrics>);

        #[qsignal]
        #[cxx_name = "searchChanged"]
        fn search_changed(self: Pin<&mut Lyrics>);

        /// Ask the sources again for the playing song (past a cached "not found").
        #[qinvokable]
        fn retry(self: &Lyrics);

        /// LRCLIB search by the user's query; the answer lands in searchResults.
        #[qinvokable]
        fn search(self: Pin<&mut Lyrics>, query: QString);

        /// Use searchResults[index] for the playing song. "" or an error text.
        #[qinvokable]
        #[cxx_name = "applyResult"]
        fn apply_result(self: &Lyrics, index: i32) -> QString;

        /// Use an .lrc/.txt file (file:// URL or path) for the playing song. "" or an error text.
        #[qinvokable]
        #[cxx_name = "importFile"]
        fn import_file(self: &Lyrics, url: QString) -> QString;

        /// Write the shown lyrics as .lrc to a file (file:// URL or path). "" or an error text.
        #[qinvokable]
        #[cxx_name = "saveFile"]
        fn save_file(self: &Lyrics, url: QString) -> QString;
    }

    impl cxx_qt::Threading for Lyrics {}
    impl cxx_qt::Initialize for Lyrics {}
}

pub struct LyricsRust {
    snapshot: LyricsSnapshot,
    lines: QVariantList,
    track_id: QString,
    status: QString,
    source: QString,
    kind: QString,
    search_results: QVariantList,
    searching: bool,
    search_error: QString,
    hits: Vec<mf_lyrics::sources::SearchHit>,
    search_generation: u64,
}

impl Default for LyricsRust {
    fn default() -> Self {
        let snapshot = LyricsSnapshot::default();
        Self {
            status: QString::from(snapshot.status.as_str()),
            snapshot,
            lines: QVariantList::default(),
            track_id: QString::default(),
            source: QString::default(),
            kind: QString::default(),
            search_results: QVariantList::default(),
            searching: false,
            search_error: QString::default(),
            hits: Vec::new(),
            search_generation: 0,
        }
    }
}

impl qobject::Lyrics {
    fn retry(&self) {
        ctx().services.lyrics.retry();
    }

    fn search(mut self: Pin<&mut Self>, query: QString) {
        let query = query.to_string().trim().to_owned();
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.search_generation += 1;
            this.searching = !query.is_empty();
            this.search_error = QString::default();
            this.search_generation
        };
        self.as_mut().search_changed();
        if query.is_empty() {
            return;
        }
        let thread = self.qt_thread();
        ctx().rt.spawn(async move {
            let result = ctx().services.lyrics.search(&query).await;
            let _ = thread.queue(move |lyrics| lyrics.search_done(generation, result));
        });
    }

    fn search_done(mut self: Pin<&mut Self>, generation: u64, result: Result<Vec<mf_lyrics::sources::SearchHit>, String>) {
        let mut this = self.as_mut().rust_mut();
        if this.search_generation != generation {
            return;
        }
        this.searching = false;
        match result {
            Ok(hits) => {
                this.search_results = to_qlist(&hits);
                this.search_error = if hits.is_empty() { QString::from(mf_i18n::t("Nothing found").as_str()) } else { QString::default() };
                this.hits = hits;
            }
            Err(e) => {
                tracing::warn!("lyrics search: {e}");
                this.search_error = QString::from(mf_i18n::t("Search failed. Check the connection and try again").as_str());
            }
        }
        self.search_changed();
    }

    fn apply_result(&self, index: i32) -> QString {
        let Some(hit) = usize::try_from(index).ok().and_then(|i| self.hits.get(i)) else { return QString::default() };
        let text = if hit.synced.trim().is_empty() { &hit.plain } else { &hit.synced };
        error_text(ctx().services.lyrics.apply_text("LRCLIB", text))
    }

    fn import_file(&self, url: QString) -> QString {
        let path = local_path(&url);
        let text = match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => {
                tracing::warn!("lyrics import {path}: {e}");
                return QString::from(mf_i18n::t("Couldn't read the file").as_str());
            }
        };
        error_text(ctx().services.lyrics.apply_text(&mf_i18n::t("File"), &text))
    }

    fn save_file(&self, url: QString) -> QString {
        let path = local_path(&url);
        let Some(lrc) = ctx().services.lyrics.lrc() else {
            return QString::from(mf_i18n::t("There are no lyrics to save").as_str());
        };
        match std::fs::write(&path, lrc) {
            Ok(()) => QString::default(),
            Err(e) => {
                tracing::warn!("lyrics save {path}: {e}");
                QString::from(mf_i18n::t("Couldn't save the file").as_str())
            }
        }
    }

    fn apply(mut self: Pin<&mut Self>, next: LyricsSnapshot) {
        if self.snapshot == next {
            return;
        }
        let mut this = self.as_mut().rust_mut();
        if !Arc::ptr_eq(&this.snapshot.lines, &next.lines) {
            this.lines = match serde_json::to_value(&*next.lines) {
                Ok(serde_json::Value::Array(mut items)) => {
                    with_romanized(&mut items, &next.lines);
                    json_to_qvariantlist(&items)
                }
                _ => QVariantList::default(),
            };
        }
        this.track_id = QString::from(next.track_id.as_str());
        this.status = QString::from(next.status.as_str());
        this.source = QString::from(next.source.as_str());
        this.kind = QString::from(next.kind.as_str());
        this.snapshot = next;
        self.changed();
    }
}

fn error_text(result: Result<(), String>) -> QString {
    QString::from(result.err().unwrap_or_default().as_str())
}

/// "r": the line in Latin letters for lines in Devanagari, Gurmukhi, Hangul, kana, Cyrillic or Greek (shown under the
/// line when Settings.lyricsRomanize is on).
fn with_romanized(items: &mut [serde_json::Value], lines: &[mf_lyrics::Line]) {
    for (item, line) in items.iter_mut().zip(lines) {
        if let serde_json::Value::Object(map) = item
            && let Some(latin) = mf_lyrics::romanize::romanize(line.text().trim())
        {
            map.insert("r".into(), serde_json::Value::String(latin));
        }
    }
}

impl cxx_qt::Initialize for qobject::Lyrics {
    fn initialize(mut self: Pin<&mut Self>) {
        let mut watch = ctx().services.lyrics.watch();
        let snapshot = watch.borrow_and_update().clone();
        self.as_mut().apply(snapshot);
        forward_watch(self.qt_thread(), watch, Self::apply);
    }
}
