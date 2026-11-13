//! Сервисы без входа и без сети: история, закрепления, стоп-лист, каталог отвечает «Войдите в аккаунт».
use std::path::PathBuf;
use std::sync::Arc;

use md3_model::{PlayContext, TrackItem};
use sf_api::{ApiGateway, NetActivity};
use sf_player::{PlayerConfig, PlayerHandle};
use sf_services::{
    AuthService, AuthState, CatalogService, HistoryService, LibraryEvent, LibraryKind, LibraryService, ProxyConfig,
    ProxyInput, SearchKind, ServiceError,
};
use mf_platform::{SettingKey, SettingsStore};

struct Fixture {
    dir: PathBuf,
    settings: Arc<SettingsStore>,
    player: PlayerHandle,
    auth: Arc<AuthService>,
}

fn fixture() -> Fixture {
    let rt = tokio::runtime::Handle::current();
    let dir = std::env::temp_dir().join(format!("sf-services-offline-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let settings = SettingsStore::in_memory();
    let http = sf_api::Http::default();
    let api = Arc::new(ApiGateway::new(http.clone(), Arc::new(NetActivity::default())));
    let config = PlayerConfig { cover_dir: dir.join("covers"), session_file: None, volume: 0.5, http: http.clone() };
    let player = PlayerHandle::start(&rt, config, api.clone()).unwrap();
    let store = sf_auth::Store::in_memory(dir.join("state"));
    let auth = AuthService::new(rt, settings.clone(), api, http, player.clone(), store);
    Fixture { dir, settings, player, auth }
}

fn track(id: &str, duration_ms: i64) -> TrackItem {
    TrackItem {
        track_id: id.to_owned(),
        uri: format!("spotify:track:{id}"),
        title: format!("Песня {id}"),
        duration_ms,
        available: true,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_counts_only_real_listening_and_survives_a_restart() {
    let f = fixture();
    let file = f.dir.join("history.json");
    let history = HistoryService::with_file(tokio::runtime::Handle::current(), f.auth.clone(), f.player.clone(), file.clone());
    let context = PlayContext::with_uri("album", "spotify:album:a1").titled("Альбом");
    history.note_played(track("skipped", 240_000), context.clone(), 5.0);
    history.note_played(track("heard", 240_000), context.clone(), 31.0);
    history.note_played(track("short", 20_000), PlayContext::new("tracks"), 10.0);
    let recent = history.recent(10);
    let uris: Vec<&str> = recent.iter().map(|play| play.track.uri.as_str()).collect();
    assert_eq!(uris, ["spotify:track:short", "spotify:track:heard"]);
    assert_eq!(history.recent_contexts(5), vec![context]);
    assert!(history.context_times().contains_key("spotify:album:a1"));
    history.flush();
    let reread = HistoryService::with_file(tokio::runtime::Handle::current(), f.auth.clone(), f.player.clone(), file);
    assert_eq!(reread.recent(10).len(), 2);
    reread.clear();
    assert!(reread.recent(10).is_empty());
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pins_and_dislikes_are_local_settings() {
    let f = fixture();
    let rt = tokio::runtime::Handle::current();
    let history =
        HistoryService::with_file(rt.clone(), f.auth.clone(), f.player.clone(), f.dir.join("history.json"));
    let library = LibraryService::new(rt, f.auth.clone(), f.player.clone(), history, f.settings.clone());
    library.use_local_playlists_file(f.dir.join("local_playlists.json"));
    let mut events = library.subscribe();

    library.toggle_pin("spotify:playlist:p1".into());
    assert!(library.is_pinned("spotify:playlist:p1"));
    assert_eq!(library.pins_revision(), 1);
    assert!(f.settings.get_str(SettingKey::Pins).contains("spotify:playlist:p1"));
    library.toggle_pin("spotify:playlist:p1".into());
    assert!(!library.is_pinned("spotify:playlist:p1"));
    assert!(f.settings.get_str(SettingKey::Pins).is_empty());

    library.dislike(track("bad", 1000));
    assert!(library.is_hidden("spotify:track:bad"));
    assert!(f.settings.get_str(SettingKey::HiddenTracks).contains("spotify:track:bad"));

    library.toggle_like(track("x", 1000));
    assert!(!library.is_liked("x"), "signed out: nothing changes");
    let mut notice = None;
    while let Ok(event) = events.try_recv() {
        if let LibraryEvent::Notice(text) = event {
            notice = Some(text);
        }
    }
    assert_eq!(notice.as_deref(), Some("Sign in to your account"));
    assert!(library.entries(LibraryKind::Playlists, "all", "spotify").is_empty());
    assert_eq!(library.loading_pages(), (0, 0));
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_playlists_route_edit_and_persist() {
    use md3_model::{ListChange, PlaylistId};
    let f = fixture();
    let rt = tokio::runtime::Handle::current();
    let history = HistoryService::with_file(rt.clone(), f.auth.clone(), f.player.clone(), f.dir.join("history.json"));
    let library = LibraryService::new(rt.clone(), f.auth.clone(), f.player.clone(), history.clone(), f.settings.clone());
    let file = f.dir.join("local_playlists.json");
    library.use_local_playlists_file(file.clone());
    let mut events = library.subscribe();

    // no Spotify account: "New playlist" makes a local one
    let song = |id: &str| TrackItem { uri: format!("youtube:track:{id}"), title: id.to_uppercase(), available: true, ..Default::default() };
    library.create_playlist("Road trip".into(), Some(song("a")));
    let entries = library.entries(LibraryKind::Playlists, "all", "spotify");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert!(entry.uri.starts_with("local:playlist:") && entry.can_edit && !entry.read_only);
    assert_eq!((entry.title.as_str(), entry.track_count), ("Road trip", 1));
    assert!(library.entries(LibraryKind::Playlists, "followed", "spotify").is_empty());

    let id = PlaylistId::from_uri_or_id(&entry.id);
    assert!(id.is_local() && library.can_edit(&id));
    library.add_tracks(id.clone(), vec!["youtube:track:b".into(), "local:track:00ff".into()]);
    library.insert_tracks(id.clone(), vec!["youtube:track:c".into()], 0);
    library.move_tracks(id.clone(), 0, 3);
    library.remove_tracks(id.clone(), vec![("youtube:track:b".into(), 1)]);
    library.rename_playlist(id.clone(), "  Drive  ".into());
    let uris = |library: &LibraryService| -> Vec<String> {
        library.playlist_tracks(&id).unwrap().into_iter().map(|track| track.uri).collect()
    };
    assert_eq!(uris(&library), ["youtube:track:a", "local:track:00ff", "youtube:track:c"]);
    let info = library.playlist_info(&id).unwrap();
    assert_eq!((info.item.title.as_str(), info.item.track_count, info.item.can_edit), ("Drive", 3, true));

    library.load_playlist(id.clone());
    let mut reset = None;
    while let Ok(event) = events.try_recv() {
        if let LibraryEvent::PlaylistTracks { id: of, change: ListChange::Reset(rows) } = event
            && of == id
        {
            reset = Some(rows.len());
        }
    }
    assert_eq!(reset, Some(3), "opening sends the rows at once");

    // saved to disk: a new service reads the same playlist
    let again = LibraryService::new(rt, f.auth.clone(), f.player.clone(), history, f.settings.clone());
    again.use_local_playlists_file(file.clone());
    assert_eq!(uris(&again), ["youtube:track:a", "local:track:00ff", "youtube:track:c"]);
    let saved = again.local_playlists();
    assert!(saved[0].dirty && saved[0].youtube_id.is_none(), "signed out: waits for a sync");

    library.unfollow_playlist(id.clone());
    assert!(library.playlist_info(&id).is_none());
    assert!(library.entries(LibraryKind::Playlists, "all", "spotify").is_empty());
    library.follow_playlist(id.clone());
    assert_eq!(uris(&library).len(), 3, "Undo brings it back");
    library.unfollow_playlist(id.clone());
    again.use_local_playlists_file(file);
    assert!(again.local_playlists().is_empty());
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_asks_to_sign_in_and_an_empty_search_is_empty() {
    let f = fixture();
    assert_eq!(f.auth.snapshot().state, AuthState::SignedOut);
    assert!(!f.auth.is_signed_in());
    let catalog = CatalogService::new(f.auth.clone(), f.player.clone());
    assert!(catalog.search("   ", SearchKind::All).await.unwrap().is_empty());
    match catalog.album("a1").await {
        Err(error @ ServiceError::NotSignedIn) => assert_eq!(error.to_string(), "Sign in to your account"),
        other => panic!("expected NotSignedIn, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&f.dir);
}

/// --offline: start() не зовут — «Входим…» не висит; start() без сохранённого входа приводит к signedOut.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_in_without_saved_grants_ends_signed_out() {
    let f = fixture();
    assert_eq!(f.auth.snapshot().state, AuthState::SignedOut);
    let mut watch = f.auth.watch();
    f.auth.start();
    let settled = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if watch.borrow_and_update().state == AuthState::SignedOut {
                return;
            }
            watch.changed().await.unwrap();
        }
    })
    .await;
    assert!(settled.is_ok(), "sign-in got stuck in {:?}", f.auth.snapshot().state);
    let _ = std::fs::remove_dir_all(&f.dir);
}

/// Оболочка берёт прокси для картинок QML из proxy()/watch_proxy(): с паролем и после каждого применения.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn applied_proxy_is_published_for_the_shell() {
    let f = fixture();
    assert_eq!(f.auth.proxy(), ProxyConfig::System);
    let mut proxy = f.auth.watch_proxy();
    f.auth.apply_proxy(ProxyInput {
        mode: "socks".into(),
        host: "127.0.0.1".into(),
        port: "1080".into(),
        username: "user".into(),
        password: "secret".into(),
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), proxy.changed()).await.unwrap().unwrap();
    let ProxyConfig::Socks(manual) = f.auth.proxy() else { panic!("expected SOCKS, got {:?}", f.auth.proxy()) };
    assert_eq!((manual.host(), manual.port(), manual.username(), manual.password()), ("127.0.0.1", 1080, "user", "secret"));
    assert_eq!(f.settings.get_str(SettingKey::ProxyMode), "socks");
    let _ = std::fs::remove_dir_all(&f.dir);
}

/// Вход в Spotify убран: кнопки моста не открывают браузер и не меняют состояние.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spotify_sign_in_entry_points_do_nothing() {
    let f = fixture();
    let before = f.auth.snapshot();
    assert_eq!(before.state, AuthState::SignedOut);
    f.auth.start_login();
    f.auth.start_web_login();
    f.auth.authorize_playback();
    f.auth.set_personal_client_id(Some("dummy-personal-client".into()));
    f.auth.retry();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let after = f.auth.snapshot();
    assert_eq!(after, before);
    assert!(after.auth_url.is_empty());
    assert_eq!(after.auth_port, 0);
    assert!(!f.auth.is_signed_in());
    assert!(f.settings.get_str(SettingKey::WebClientId).is_empty());
    let _ = std::fs::remove_dir_all(&f.dir);
}
