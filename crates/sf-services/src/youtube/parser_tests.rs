//! Parser tests against the recorded responses in `tests/fixtures/youtube/` and small inline shapes.
use md3_model::ArtistRef;
use serde_json::{Value, json};

use super::error::YouTubeError;
use super::models::*;
use super::parsers::*;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/youtube/");

pub(crate) fn fixture(name: &str) -> Value {
    let path = format!("{FIXTURES}{name}");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path}: {error}"))
}

#[test]
fn the_first_playlist_page_has_its_header_one_track_and_a_continuation() {
    let page = playlist_page(&fixture("playlist_tracks_page1.json"), "VLPL1234567890abcdef").expect("page");
    assert_eq!(page.info.playlist_id, "PL1234567890abcdef");
    assert_eq!(page.info.uri, "youtube:playlist:PL1234567890abcdef");
    assert_eq!(page.info.title, "Chill Lofi Beats");
    assert_eq!(page.info.owner, "Curator Alex");
    assert_eq!(page.info.track_count, 100);
    assert!(!page.info.can_edit);
    assert_eq!(page.tracks.len(), 1);
    let row = &page.tracks[0];
    assert_eq!(row.set_video_id, "set_001");
    let track = &row.track;
    assert_eq!(track.track_id, "vid_track_001");
    assert_eq!(track.uri, "youtube:track:vid_track_001");
    assert_eq!(track.title, "Midnight Rain");
    assert_eq!(track.artists, "Kavinsky");
    assert_eq!(track.artist_refs, vec![ArtistRef { id: "UCkavinsky".into(), name: "Kavinsky".into() }]);
    assert_eq!(track.album, "Nightcall EP");
    assert_eq!(track.album_id, "MPREb_album01");
    assert_eq!(track.duration_ms, 222_000);
    assert_eq!(track.cover, "https://lh3.googleusercontent.com/track001.jpg=w300-h300-l90-rj");
    assert!(track.available);
    let token = page.continuation.expect("continuation");
    assert!(token.contains("ctok_page2_token"), "{token}");
}

#[test]
fn the_second_playlist_page_ends_the_list() {
    let page = playlist_continuation(&fixture("playlist_tracks_page2.json"));
    assert_eq!(page.tracks.len(), 1);
    let row = &page.tracks[0];
    assert_eq!(row.track.track_id, "vid_track_051");
    assert_eq!(row.track.title, "Sunset Horizon");
    assert_eq!(row.track.artists, "Tycho");
    assert_eq!(row.track.duration_ms, 255_000);
    assert_eq!(row.set_video_id, "set_051");
    assert_eq!(page.continuation, None);
}

#[test]
fn search_results_yield_a_track_an_album_an_artist_and_a_playlist() {
    let page = search_page(&fixture("search_response.json"));
    assert_eq!(page.items.len(), 4, "{:#?}", page.items);
    let YtItem::Track(track) = &page.items[0] else { panic!("{:?}", page.items[0]) };
    assert_eq!(track.track_id, "fJ9rUzIMcZQ");
    assert_eq!(track.title, "Bohemian Rhapsody");
    assert_eq!(track.artists, "Queen");
    assert_eq!(track.album, "A Night at the Opera");
    assert_eq!(track.album_id, "MPREb_opera");
    assert_eq!(track.duration_ms, 355_000);
    let YtItem::Album(album) = &page.items[1] else { panic!("{:?}", page.items[1]) };
    assert_eq!(album.album_id, "MPREb_opera");
    assert_eq!(album.uri, "youtube:album:MPREb_opera");
    assert_eq!(album.title, "A Night at the Opera");
    assert_eq!(album.artists, "Queen");
    assert_eq!(album.year, 1975);
    let YtItem::Artist(artist) = &page.items[2] else { panic!("{:?}", page.items[2]) };
    assert_eq!(artist.artist_id, "UCiMhD4jzUqG-IgPzUmmytRQ");
    assert_eq!(artist.uri, "youtube:artist:UCiMhD4jzUqG-IgPzUmmytRQ");
    assert_eq!(artist.name, "Queen");
    assert_eq!(artist.followers, 38_000_000);
    let YtItem::Playlist(playlist) = &page.items[3] else { panic!("{:?}", page.items[3]) };
    assert_eq!(playlist.playlist_id, "PLqueen_greatest");
    assert_eq!(playlist.uri, "youtube:playlist:PLqueen_greatest");
    assert_eq!(playlist.title, "Queen: Greatest Hits");
    assert_eq!(playlist.track_count, 50);
    assert_eq!(page.continuation, None);
}

#[test]
fn the_search_summary_keeps_one_shelf_per_kind() {
    let shelves = search_summary(&fixture("search_response.json"));
    let titles: Vec<&str> = shelves.iter().map(|shelf| shelf.title.as_str()).collect();
    assert_eq!(titles, ["Songs", "Albums", "Artists", "Community playlists"]);
    assert!(shelves.iter().all(|shelf| shelf.items.len() == 1));
}

#[test]
fn the_library_lists_both_playlists_without_the_vl_prefix() {
    let page = library_page(&fixture("browse_library.json"));
    let playlists: Vec<(String, i64)> = page
        .items
        .iter()
        .map(|item| match item {
            YtItem::Playlist(playlist) => (playlist.playlist_id.clone(), playlist.track_count),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(playlists, [("PL1234567890abcdef".to_owned(), 45), ("PLsynthwave999xyz".to_owned(), 80)]);
    let YtItem::Playlist(first) = &page.items[0] else { unreachable!() };
    assert_eq!(first.title, "Chill Lofi Beats");
    // the largest thumbnail - size params added by sized()
    assert_eq!(first.cover, "https://lh3.googleusercontent.com/cover1_large.jpg=w300-h300-l90-rj");
    assert_eq!(page.continuation, None);
}

#[test]
fn a_page_without_a_header_is_an_unavailable_playlist() {
    let errors = fixture("error_responses.json");
    assert_eq!(playlist_page(&errors["playlist_private_or_deleted"], "PLgone"), Err(YouTubeError::PlaylistUnavailable));
}

#[test]
fn liked_music_uses_the_detail_header_layout() {
    let response = json!({
        "header": { "musicDetailHeaderRenderer": {
            "title": { "runs": [{ "text": "Liked Music" }] },
            "secondSubtitle": { "runs": [{ "text": "1,234 songs" }] }
        }},
        "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
            "sectionListRenderer": { "contents": [{ "musicShelfRenderer": {
                "contents": [{ "musicResponsiveListItemRenderer": {
                    "playlistItemData": { "videoId": "abcdefghijk" },
                    "flexColumns": [
                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Song" }] } } },
                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Singer" }] } } }
                    ],
                    "fixedColumns": [{ "musicResponsiveListItemFlexColumnRenderer": { "text": { "simpleText": "1:02:03" } } }]
                }}],
                "continuations": [{ "nextContinuationData": { "continuation": "next-liked" } }]
            }}]}
        }}}]}}
    });
    let page = playlist_page(&response, "LM").expect("page");
    assert_eq!(page.info.title, "Liked Music");
    assert_eq!(page.info.track_count, 1234);
    assert_eq!(page.tracks[0].track.artists, "Singer");
    // the old spelling of the fixed column is still read
    assert_eq!(page.tracks[0].track.duration_ms, 3_723_000);
    assert_eq!(page.continuation.as_deref(), Some("next-liked"));
}

#[test]
fn playlist_continuations_are_found_in_appended_items_too() {
    let response = json!({ "onResponseReceivedActions": [{ "appendContinuationItemsAction": { "continuationItems": [
        { "musicResponsiveListItemRenderer": {
            "playlistItemData": { "videoId": "v1", "playlistSetVideoId": "s1" },
            "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "One" }] } } }]
        }},
        { "continuationItemRenderer": { "continuationEndpoint": { "continuationCommand": { "token": "tok-3" } } } }
    ]}}]});
    let page = playlist_continuation(&response);
    assert_eq!(page.tracks.len(), 1);
    assert_eq!(page.tracks[0].set_video_id, "s1");
    assert_eq!(page.continuation.as_deref(), Some("tok-3"));
    assert_eq!(playlist_continuation(&json!({})), TracksPage::default());
}

#[test]
fn helpers_read_times_counts_and_image_sizes() {
    assert_eq!(parse_time("3:42"), Some(222));
    assert_eq!(parse_time("1:02:03"), Some(3723));
    assert_eq!(parse_time("Queen"), None);
    assert_eq!(parse_abbreviated("38M subscribers"), 38_000_000);
    assert_eq!(parse_abbreviated("1.2K"), 1200);
    assert_eq!(parse_abbreviated("12,345 views"), 12_345);
    assert_eq!(parse_abbreviated("none"), 0);
    assert_eq!(
        sized("https://lh3.googleusercontent.com/x=w120-h120-l90-rj", 300),
        "https://lh3.googleusercontent.com/x=w300-h300-l90-rj"
    );
    assert_eq!(sized("https://yt3.ggpht.com/y=s88-c-k", 640), "https://yt3.ggpht.com/y=s640");
    // ytimg.com URLs should get size params added
    assert_eq!(sized("https://i.ytimg.com/vi/z/hq.jpg", 300), "https://i.ytimg.com/vi/z/hq.jpg=w300-h300-l90-rj");
    
    // Test URL size clamping (min 48, max 1920)
    assert!(sized("https://lh3.googleusercontent.com/test", 10).contains("=w48-h48"));
    assert!(sized("https://lh3.googleusercontent.com/test", 5000).contains("=w1920-h1920"));
    
    // Test protocol-relative URL normalization via sized
    assert!(sized("//lh3.googleusercontent.com/test", 300).starts_with("https://"));
    
    let subtitle = json!({ "runs": [{ "text": "Album • Queen • 1975" }] });
    let split = runs(&subtitle);
    let texts: Vec<&str> = split.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, ["Album", " • ", "Queen", " • ", "1975"]);
}

#[test]
fn small_responses_are_read() {
    let account = json!({ "actions": [{ "openPopupAction": { "popup": { "multiPageMenuRenderer": { "header": {
        "activeAccountHeaderRenderer": {
            "accountName": { "runs": [{ "text": "Alex" }] },
            "email": { "runs": [{ "text": "alex@example.com" }] },
            "accountPhoto": { "thumbnails": [{ "url": "//lh3.googleusercontent.com/a=s48" }] }
        }}}}}}]});
    assert_eq!(
        account_info(&account),
        Some(AccountInfo {
            name: "Alex".into(),
            email: "alex@example.com".into(),
            avatar: "https://lh3.googleusercontent.com/a=s300".into()
        })
    );
    assert_eq!(account_info(&json!({})), None);
    assert_eq!(created_playlist_id(&json!({ "playlistId": "PLnew" })).as_deref(), Some("PLnew"));
    let edited = json!({ "status": "STATUS_SUCCEEDED", "playlistEditResults": [
        { "playlistEditVideoAddedResultData": { "setVideoId": "set9" } }
    ]});
    assert_eq!(edit_result(&edited), (true, vec!["set9".to_owned()]));
    let cues = json!({ "actions": [{ "updateEngagementPanelAction": { "content": { "transcriptRenderer": { "body": {
        "transcriptBodyRenderer": { "cueGroups": [{ "transcriptCueGroupRenderer": { "cues": [{ "transcriptCueRenderer": {
            "startOffsetMs": "61500", "cue": { "simpleText": "♪ Hello ♪" }
        }}]}}]}
    }}}}}]});
    assert_eq!(transcript(&cues).as_deref(), Some("[01:01.500]Hello"));
}

/// `music/get_queue` rows, plain and wrapped (a song that also has a music video), in the shape YouTube sends.
pub(super) fn get_queue_response() -> Value {
    let row = |id: &str, length: &str| {
        json!({ "playlistPanelVideoRenderer": {
            "videoId": id,
            "title": { "runs": [{ "text": "Song" }] },
            "lengthText": { "runs": [{ "text": length }], "accessibility": {} },
        } })
    };
    json!({ "queueDatas": [
        { "content": row("plain", "3:16") },
        { "content": { "playlistPanelVideoWrapperRenderer": {
            "primaryRenderer": row("wrapped", "1:02:03"),
            "counterpart": [{ "counterpartRenderer": row("video", "4:00") }],
        } } },
        { "content": { "playlistPanelVideoRenderer": { "videoId": "nolength" } } },
    ] })
}

#[test]
fn get_queue_gives_each_songs_length() {
    assert_eq!(
        queue_lengths(&get_queue_response()),
        [("plain".to_owned(), 196), ("wrapped".to_owned(), 3723)]
    );
    assert!(queue_lengths(&json!({})).is_empty());
}

#[test]
fn the_home_feed_reads_carousels_immersive_shelves_and_its_continuation() {
    let page = shelves_page(&fixture("home_page1.json"));
    let titles: Vec<&str> = page.shelves.iter().map(|shelf| shelf.title.as_str()).collect();
    assert_eq!(titles, ["Quick picks", "Your mixes"], "the taste builder is no shelf");
    assert_eq!(page.shelves[0].strapline, "Start radio from a song");
    assert!(matches!(&page.shelves[0].items[0], YtItem::Track(track) if track.track_id == "vid_around01"));
    let mixes = &page.shelves[1];
    assert_eq!(mixes.strapline, "Made for you");
    let more = mixes.more.as_ref().expect("see all");
    assert_eq!((more.browse_id.as_str(), more.params.as_deref()), ("FEmusic_mixed_for_you", Some("mixes_params")));
    let YtItem::Playlist(mix) = &mixes.items[0] else { panic!("{:?}", mixes.items[0]) };
    assert_eq!(mix.playlist_id, "RDTMAK5uy_supermix");
    assert_eq!(page.continuation.as_deref(), Some("home_ctoken_page2"));
}

#[test]
fn a_home_continuation_brings_similar_to_and_community_playlists() {
    let page = shelves_page(&fixture("home_continuation.json"));
    assert_eq!(page.shelves.len(), 2, "the empty shelf is skipped: {:#?}", page.shelves);
    let similar = &page.shelves[0];
    assert_eq!((similar.strapline.as_str(), similar.title.as_str()), ("Similar to", "Daft Punk"));
    assert_eq!(similar.thumbnail, "https://lh3.googleusercontent.com/daftpunk=w240-h240-l90-rj");
    assert_eq!(similar.more, None, "the title links to the artist, not to a full shelf");
    let YtItem::Album(album) = &similar.items[0] else { panic!("{:?}", similar.items[0]) };
    assert_eq!((album.album_id.as_str(), album.artists.as_str(), album.year), ("MPREb_cross", "Justice", 2007));
    let YtItem::Artist(artist) = &similar.items[1] else { panic!("{:?}", similar.items[1]) };
    assert_eq!((artist.artist_id.as_str(), artist.followers), ("UCair", 2_100_000));

    let community = &page.shelves[1];
    assert_eq!(community.title, "Community playlists");
    assert_eq!(community.more.as_ref().map(|more| more.browse_id.as_str()), Some("FEmusic_community_playlists"));
    let YtItem::Playlist(playlist) = &community.items[0] else { panic!("{:?}", community.items[0]) };
    assert_eq!((playlist.playlist_id.as_str(), playlist.owner.as_str()), ("PLfrenchtouch", "Night Owl"));
    assert_eq!(page.continuation, None, "the last page has no token");
}

#[test]
fn appended_continuation_items_are_read_as_shelves() {
    let carousel = fixture("home_continuation.json")["continuationContents"]["sectionListContinuation"]["contents"][1].clone();
    let response = json!({
        "onResponseReceivedActions": [{ "appendContinuationItemsAction": { "continuationItems": [
            carousel,
            { "continuationItemRenderer": { "continuationEndpoint": { "continuationCommand": { "token": "next_tok" } } } }
        ] } }]
    });
    let page = shelves_page(&response);
    assert_eq!(page.shelves.len(), 1);
    assert_eq!(page.shelves[0].title, "Community playlists");
    assert_eq!(page.continuation.as_deref(), Some("next_tok"));
}

#[test]
fn an_artist_page_carousel_title_with_params_stays_its_see_all() {
    let response = json!({ "contents": { "sectionListRenderer": { "contents": [{ "musicCarouselShelfRenderer": {
        "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [{ "text": "Albums",
            "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdaftpunk", "params": "discography",
                "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": {
                    "pageType": "MUSIC_PAGE_TYPE_ARTIST" } } } } }] } } },
        "contents": fixture("home_continuation.json")["continuationContents"]["sectionListContinuation"]["contents"][0]
            ["musicCarouselShelfRenderer"]["contents"].clone()
    } }] } } });
    let page = shelves_page(&response);
    let more = page.shelves[0].more.as_ref().expect("see all");
    assert_eq!((more.browse_id.as_str(), more.params.as_deref()), ("UCdaftpunk", Some("discography")));
}
