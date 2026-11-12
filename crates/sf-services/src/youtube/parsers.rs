//! InnerTube response parsers: pure functions over `serde_json::Value` (ports of the soundwave1 `pages/*`
//! and `YouTube.kt` extraction code). A missing key reads as `Null`, so unknown layouts yield empty results
//! instead of errors; only pages that cannot be shown at all return `Err`.
use std::collections::HashSet;
use std::sync::LazyLock;

use md3_model::{AlbumInfo, AlbumItem, ArtistInfo, ArtistItem, ArtistRef, PlaylistItem, TrackItem, album_kind_label};
use regex::Regex;
use serde_json::Value;

use super::error::{Result, YouTubeError};
use super::models::*;

const SEPARATOR: &str = " • ";
static NULL: Value = Value::Null;

static VIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*\d+([.,]\d+)?\s*(K|M|B|views|plays|visualizaciones|vues)?\s*$").expect("regex")
});
static COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(\d[\d,]*)\s+(songs?|tracks?|videos?|episodes?)\b").expect("regex"));
static ABBREVIATED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(\d[\d.,]*)\s*([KMB])?").expect("regex"));
static SIZE_WH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"=w\d+-h\d+[^/=]*$").expect("regex"));
static SIZE_S: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"=s\d+[^/=]*$").expect("regex"));

// ── small helpers ────────────────────────────────────────────────────────────────────────────────

fn arr(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn or<'a>(first: &'a Value, second: &'a Value) -> &'a Value {
    if first.is_null() { second } else { first }
}

fn non_blank(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

/// One text run with its navigation endpoint (`Null` when it is plain text).
#[derive(Clone, Debug)]
pub(crate) struct Run<'a> {
    pub text: String,
    pub nav: &'a Value,
}

/// Runs of a `{ "runs": [...] }` text. A plain run that holds several ` • `-separated parts is split,
/// so `"Album • Queen • 1975"` reads the same whether it arrives as one run or five.
pub(crate) fn runs(value: &Value) -> Vec<Run<'_>> {
    let mut out = Vec::new();
    for run in arr(&value["runs"]) {
        let content = text(&run["text"]);
        let nav = &run["navigationEndpoint"];
        if nav.is_null() && content != SEPARATOR && content.contains(SEPARATOR) {
            for (index, part) in content.split(SEPARATOR).enumerate() {
                if index > 0 {
                    out.push(Run { text: SEPARATOR.to_owned(), nav: &NULL });
                }
                out.push(Run { text: part.to_owned(), nav: &NULL });
            }
        } else {
            out.push(Run { text: content.to_owned(), nav });
        }
    }
    out
}

fn joined(runs: &[Run<'_>]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

fn runs_text(value: &Value) -> String {
    arr(&value["runs"]).iter().map(|run| text(&run["text"])).collect()
}

/// `splitBySeparator`: runs between ` • ` separators.
fn groups<'a>(runs: &[Run<'a>]) -> Vec<Vec<Run<'a>>> {
    let mut out = vec![Vec::new()];
    for run in runs {
        if run.text == SEPARATOR {
            out.push(Vec::new());
        } else if let Some(last) = out.last_mut() {
            last.push(run.clone());
        }
    }
    out
}

fn browse_id(nav: &Value) -> &str {
    text(&nav["browseEndpoint"]["browseId"])
}

fn page_type(nav: &Value) -> &str {
    text(
        &nav["browseEndpoint"]["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]
            ["pageType"],
    )
}

fn watch(nav: &Value) -> &Value {
    or(&nav["watchEndpoint"], &nav["watchPlaylistEndpoint"])
}

fn browse_ref(endpoint: &Value) -> Option<BrowseRef> {
    let browse_id = non_blank(text(&endpoint["browseId"]).to_owned())?;
    Some(BrowseRef { browse_id, params: endpoint["params"].as_str().map(str::to_owned) })
}

fn is_artist_run(run: &Run<'_>) -> bool {
    !run.nav["browseEndpoint"].is_null()
        && (browse_id(run.nav).starts_with("UC") || page_type(run.nav).contains("ARTIST"))
}

fn is_album_run(run: &Run<'_>) -> bool {
    !run.nav["browseEndpoint"].is_null()
        && (browse_id(run.nav).starts_with("MPREb_") || page_type(run.nav).contains("ALBUM"))
}

/// `"3:42"` / `"1:02:03"` in seconds.
pub(crate) fn parse_time(value: &str) -> Option<i64> {
    let parts: Vec<i64> = value.trim().split(':').map(|part| part.parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [minutes, seconds] => Some(minutes * 60 + seconds),
        [hours, minutes, seconds] => Some(hours * 3600 + minutes * 60 + seconds),
        _ => None,
    }
}

/// Durations, view counts, bare separators: never an album or an artist.
fn is_non_album_metadata(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty() || trimmed == "•" || parse_time(trimmed).is_some() || VIEWS.is_match(trimmed)
}

fn is_type_marker(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "song" | "songs" | "video" | "videos" | "single" | "singles" | "album" | "albums" | "ep" | "playlist"
            | "artist" | "episode" | "podcast" | "profile" | "station" | "canción" | "chanson" | "lied"
            | "canzone" | "música" | "music"
    )
}

fn alnum(value: &str) -> String {
    value.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// `oddElements`: names sit at even positions, joiners (`" & "`, `", "`) between them.
fn to_artists(group: &[Run<'_>]) -> Vec<ArtistRef> {
    group
        .iter()
        .step_by(2)
        .filter_map(|run| {
            let name = run.text.trim();
            (!matches!(name, "" | "•" | "&" | ","))
                .then(|| ArtistRef { id: browse_id(run.nav).to_owned(), name: name.to_owned() })
        })
        .collect()
}

/// `PageHelper.extractArtists`: the group with an artist link, else the first group that is not the
/// title, an album, a duration, a view count or a type marker.
fn extract_artists(groups: &[Vec<Run<'_>>], title: &str) -> Vec<ArtistRef> {
    for group in groups {
        if group.iter().any(is_artist_run) {
            let artists = to_artists(group);
            if !artists.is_empty() {
                return artists;
            }
        }
    }
    let title_key = alnum(title);
    for group in groups {
        let Some(first) = group.first() else { continue };
        let value = first.text.trim();
        let is_title = !title_key.is_empty() && (alnum(value) == title_key || value.eq_ignore_ascii_case(title.trim()));
        if is_title || group.iter().any(is_album_run) || is_non_album_metadata(value) || is_type_marker(value) {
            continue;
        }
        return to_artists(group);
    }
    Vec::new()
}

fn album_of(run: &Run<'_>) -> (String, String) {
    (run.text.trim().to_owned(), browse_id(run.nav).to_owned())
}

fn album_candidate(run: Option<&Run<'_>>) -> Option<(String, String)> {
    run.filter(|run| !is_artist_run(run) && !is_non_album_metadata(&run.text)).map(album_of)
}

/// `PageHelper.extractAlbumFromRuns`: (name, browse id) from one subtitle line.
fn extract_album_runs(runs: &[Run<'_>]) -> Option<(String, String)> {
    if let Some(run) = runs.iter().find(|run| is_album_run(run)) {
        return Some(album_of(run));
    }
    let segments = groups(runs);
    if segments.len() >= 3 {
        return album_candidate(segments[1].iter().find(|run| !run.text.trim().is_empty()));
    }
    None
}

/// `PageHelper.extractAlbum`: (name, browse id) from the flex columns of a list row.
fn extract_album_columns(columns: &[Vec<Run<'_>>]) -> Option<(String, String)> {
    if let Some(run) = columns.iter().flatten().find(|run| is_album_run(run)) {
        return Some(album_of(run));
    }
    for column in columns.iter().skip(2) {
        if let Some(album) = album_candidate(column.iter().find(|run| !run.text.trim().is_empty())) {
            return Some(album);
        }
    }
    let segments = groups(columns.get(1)?);
    if segments.len() >= 3 {
        return album_candidate(segments[1].iter().find(|run| !run.text.trim().is_empty()));
    }
    None
}

fn last_time(groups: &[Vec<Run<'_>>]) -> Option<i64> {
    groups.iter().rev().flat_map(|group| group.iter().rev()).find_map(|run| parse_time(&run.text))
}

fn year_of(groups: &[Vec<Run<'_>>]) -> i64 {
    groups
        .iter()
        .flatten()
        .filter_map(|run| run.text.trim().parse::<i64>().ok())
        .find(|year| (1900..2200).contains(year))
        .unwrap_or(0)
}

fn count_of(value: &str) -> Option<i64> {
    COUNT.captures(value)?.get(1)?.as_str().replace(',', "").parse().ok()
}

/// `"38M subscribers"` / `"1.2K"` / `"12,345"` as a number (0 when there is none).
pub(crate) fn parse_abbreviated(value: &str) -> i64 {
    let Some(found) = ABBREVIATED.captures(value) else { return 0 };
    let scale = match found.get(2).map(|suffix| suffix.as_str().to_ascii_uppercase()).as_deref() {
        Some("K") => 1e3,
        Some("M") => 1e6,
        Some("B") => 1e9,
        _ => 1.0,
    };
    let digits = if scale > 1.0 { found[1].replace(',', ".") } else { found[1].replace(',', "") };
    digits.parse::<f64>().map_or(0, |number| (number * scale).round() as i64)
}

/// Normalize a thumbnail URL: fix protocol-relative URLs and malformed size parameters.
fn normalize_thumbnail_url(url: &str) -> String {
    let url = url.trim();
    
    // Fix protocol-relative URLs
    let url = if url.starts_with("//") {
        format!("https:{}", url)
    } else {
        url.to_owned()
    };
    
    // For Google/YouTube URLs, normalize size parameters
    if url.contains("googleusercontent.com") || url.contains("ggpht.com") || url.contains("ytimg.com") {
        // Normalize =w0-h0 or missing size params to =w300-h300-l90-rj for square thumbnails
        // This preserves aspect ratio while ensuring a reasonable size
        let url = if SIZE_WH.is_match(&url) {
            SIZE_WH.replace(&url, "=w300-h300-l90-rj").into_owned()
        } else if SIZE_S.is_match(&url) {
            SIZE_S.replace(&url, "=s300").into_owned()
        } else if !url.contains("=w") && !url.contains("=s") {
            // Append size param if missing
            format!("{}=w300-h300-l90-rj", url)
        } else {
            url
        };
        return url;
    }
    
    url
}

/// Last (largest) entry of a `{ "thumbnails": [...] }` object.
fn thumbnail_url(value: &Value) -> String {
    let url = arr(&value["thumbnails"]).last().map(|thumbnail| text(&thumbnail["url"])).unwrap_or_default();
    normalize_thumbnail_url(url)
}

/// A thumbnail in any of its wrappers (`musicThumbnailRenderer`, `croppedSquareThumbnailRenderer`, bare).
fn renderer_thumbnail(value: &Value) -> String {
    let renderer = or(&value["musicThumbnailRenderer"], &value["croppedSquareThumbnailRenderer"]);
    thumbnail_url(or(&renderer["thumbnail"], or(&value["thumbnail"], value)))
}

/// Google/YouTube image URLs carry their size as a suffix; ask for `px` instead.
/// Also handles ytimg.com URLs. Other hosts are left alone.
/// Fixes protocol-relative URLs (//...) by adding https: prefix.
pub(crate) fn sized(url: &str, px: u32) -> String {
    let url = url.trim();
    
    // Fix protocol-relative URLs first
    let url = if url.starts_with("//") {
        format!("https:{}", url)
    } else {
        url.to_owned()
    };
    
    if !(url.contains("googleusercontent.com") || url.contains("ggpht.com") || url.contains("ytimg.com")) {
        return url;
    }
    
    // Ensure px is reasonable (minimum 48, maximum 1920)
    let px = px.clamp(48, 1920);
    
    if SIZE_WH.is_match(&url) {
        SIZE_WH.replace(&url, format!("=w{px}-h{px}-l90-rj")).into_owned()
    } else if SIZE_S.is_match(&url) {
        SIZE_S.replace(&url, format!("=s{px}")).into_owned()
    } else if !url.contains("=") || url.ends_with("=") {
        // Append size param if missing or empty
        format!("{}=w{px}-h{px}-l90-rj", url.trim_end_matches('='))
    } else {
        url
    }
}

/// First continuation token of a `continuations` array.
fn continuation_of(continuations: &Value) -> Option<String> {
    let first = &continuations[0];
    let data = or(&first["nextContinuationData"], &first["nextRadioContinuationData"]);
    non_blank(text(&data["continuation"]).to_owned())
}

/// Token of a `continuationItemRenderer` among list contents.
fn continuation_item(contents: &Value) -> Option<String> {
    arr(contents).iter().find_map(|item| {
        non_blank(
            text(&item["continuationItemRenderer"]["continuationEndpoint"]["continuationCommand"]["token"]).to_owned(),
        )
    })
}

fn is_explicit(badges: &Value) -> bool {
    arr(badges).iter().any(|badge| text(&badge["musicInlineBadgeRenderer"]["icon"]["iconType"]) == "MUSIC_EXPLICIT_BADGE")
}

fn dedupe(items: Vec<YtItem>) -> Vec<YtItem> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|item| seen.insert(item.uri().to_owned())).collect()
}

// ── items ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Song,
    Artist,
    Album,
    Playlist,
    Unknown,
}

/// What an endpoint leads to: by page type, and by the id's prefix when the response has none.
fn classify(nav: &Value) -> Kind {
    if !watch(nav).is_null() {
        return Kind::Song;
    }
    let kind = page_type(nav);
    let id = browse_id(nav);
    if kind.contains("ARTIST") {
        Kind::Artist
    } else if kind.contains("ALBUM") || kind.contains("AUDIOBOOK") {
        Kind::Album
    } else if kind.contains("PLAYLIST") {
        Kind::Playlist
    } else if !kind.is_empty() {
        Kind::Unknown
    } else if id.starts_with("MPREb_") {
        Kind::Album
    } else if id.starts_with("UC") || id.starts_with("MPLAUC") {
        Kind::Artist
    } else if ["VL", "PL", "RD", "OLAK5uy_"].iter().any(|prefix| id.starts_with(prefix)) {
        Kind::Playlist
    } else {
        Kind::Unknown
    }
}

fn artist_names(artists: &[ArtistRef]) -> String {
    artists.iter().map(|artist| artist.name.as_str()).collect::<Vec<_>>().join(", ")
}

fn make_track(
    video_id: &str,
    title: String,
    artists: Vec<ArtistRef>,
    album: Option<(String, String)>,
    seconds: Option<i64>,
    thumbnail: &str,
    explicit: bool,
) -> TrackItem {
    let (album, album_id) = album.unwrap_or_default();
    TrackItem {
        track_id: video_id.to_owned(),
        uri: track_uri(video_id),
        title,
        artists: artist_names(&artists),
        artist_refs: artists,
        album,
        album_id,
        cover: sized(thumbnail, 300),
        cover_large: sized(thumbnail, 640),
        duration_ms: seconds.unwrap_or(0) * 1000,
        explicit,
        available: true,
        ..Default::default()
    }
}

/// An item described by a title, where it leads, and one subtitle line (two-row tiles, card shelves and
/// the non-song rows of lists).
fn card_item(title: &str, nav: &Value, subtitle: &[Run<'_>], thumbnail: &str, explicit: bool) -> Option<YtItem> {
    let all = groups(subtitle);
    let marker = all.first().and_then(|group| group.first()).filter(|run| is_type_marker(&run.text));
    let marker_text = marker.map(|run| run.text.trim().to_lowercase()).unwrap_or_default();
    let rest = if marker.is_some() { &all[1..] } else { &all[..] };
    match classify(nav) {
        Kind::Song => {
            let video_id = text(&watch(nav)["videoId"]);
            (!video_id.is_empty()).then(|| {
                YtItem::Track(make_track(
                    video_id,
                    title.to_owned(),
                    extract_artists(rest, title),
                    extract_album_runs(subtitle),
                    last_time(rest),
                    thumbnail,
                    explicit,
                ))
            })
        }
        Kind::Artist => {
            let id = browse_id(nav);
            let followers = rest.iter().map(|group| joined(group)).find(|value| value.to_lowercase().contains("subscriber"));
            Some(YtItem::Artist(ArtistItem {
                artist_id: id.to_owned(),
                uri: artist_uri(id),
                name: title.to_owned(),
                cover: sized(thumbnail, 300),
                cover_large: sized(thumbnail, 640),
                followers: followers.map_or(0, |value| parse_abbreviated(&value)),
                ..Default::default()
            }))
        }
        Kind::Album => {
            let id = browse_id(nav);
            let artists = extract_artists(rest, title);
            let kind = match marker_text.as_str() {
                "single" | "singles" => "single",
                "ep" => "ep",
                _ => "album",
            };
            Some(YtItem::Album(AlbumItem {
                album_id: id.to_owned(),
                uri: album_uri(id),
                title: title.to_owned(),
                artists: artist_names(&artists),
                artist_refs: artists,
                year: year_of(rest),
                kind: album_kind_label(kind, 0),
                cover: sized(thumbnail, 300),
                cover_large: sized(thumbnail, 640),
                explicit,
                ..Default::default()
            }))
        }
        Kind::Playlist => {
            let raw = or(&nav["browseEndpoint"]["browseId"], &watch(nav)["playlistId"]);
            let id = text(raw).trim_start_matches("VL");
            if id.is_empty() {
                return None;
            }
            let count = rest.iter().find_map(|group| count_of(&joined(group)));
            let owner = rest.iter().find(|group| {
                let value = joined(group);
                count_of(&value).is_none() && !is_non_album_metadata(&value)
            });
            Some(YtItem::Playlist(PlaylistItem {
                playlist_id: id.to_owned(),
                uri: playlist_uri(id),
                title: title.to_owned(),
                track_count: count.unwrap_or(0),
                cover: sized(thumbnail, 300),
                owner: owner.map(|group| joined(group).trim().to_owned()).unwrap_or_default(),
                owner_id: owner.and_then(|group| group.first()).map(|run| browse_id(run.nav).to_owned()).unwrap_or_default(),
                ..Default::default()
            }))
        }
        Kind::Unknown => None,
    }
}

fn flex_columns(renderer: &Value) -> Vec<Vec<Run<'_>>> {
    arr(&renderer["flexColumns"])
        .iter()
        .map(|column| runs(&column["musicResponsiveListItemFlexColumnRenderer"]["text"]))
        .collect()
}

fn overlay_nav(renderer: &Value) -> &Value {
    &renderer["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]
        ["playNavigationEndpoint"]
}

/// Duration from the fixed column; both spellings of its renderer are accepted.
fn fixed_duration(renderer: &Value) -> Option<i64> {
    let column = &renderer["fixedColumns"][0];
    let inner = or(
        &column["musicResponsiveListItemFixedColumnRenderer"],
        &column["musicResponsiveListItemFlexColumnRenderer"],
    );
    parse_time(text(or(&inner["text"]["runs"][0]["text"], &inner["text"]["simpleText"])))
}

/// A song row of a list (`musicResponsiveListItemRenderer`): playlists, albums, search, shelves.
pub(crate) fn list_track(renderer: &Value) -> Option<PlaylistTrack> {
    let columns = flex_columns(renderer);
    let title = non_blank(joined(columns.first()?))?;
    let first_run_nav = columns[0].first().map_or(&NULL, |run| run.nav);
    let video_id = [
        &renderer["playlistItemData"]["videoId"],
        &watch(&renderer["navigationEndpoint"])["videoId"],
        &watch(overlay_nav(renderer))["videoId"],
        &first_run_nav["watchEndpoint"]["videoId"],
    ]
    .into_iter()
    .map(text)
    .find(|id| !id.is_empty())?;
    let metadata: Vec<Vec<Run<'_>>> = columns.iter().skip(1).flat_map(|column| groups(column)).collect();
    let mut track = make_track(
        video_id,
        title.clone(),
        extract_artists(&metadata, &title),
        extract_album_columns(&columns),
        fixed_duration(renderer).or_else(|| last_time(&metadata)),
        &renderer_thumbnail(&renderer["thumbnail"]),
        is_explicit(&renderer["badges"]),
    );
    track.available =
        text(&renderer["musicItemRendererDisplayPolicy"]) != "MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT";
    Some(PlaylistTrack { track, set_video_id: text(&renderer["playlistItemData"]["playlistSetVideoId"]).to_owned() })
}

/// Any row of a list: a song, or an artist / album / playlist link.
pub(crate) fn list_item(renderer: &Value) -> Option<YtItem> {
    let nav = &renderer["navigationEndpoint"];
    if nav.is_null() || classify(nav) == Kind::Song {
        return list_track(renderer).map(|row| YtItem::Track(row.track));
    }
    let columns = flex_columns(renderer);
    let title = non_blank(joined(columns.first()?))?;
    let mut subtitle: Vec<Run<'_>> = Vec::new();
    for column in columns.iter().skip(1).filter(|column| !column.is_empty()) {
        if !subtitle.is_empty() {
            subtitle.push(Run { text: SEPARATOR.to_owned(), nav: &NULL });
        }
        subtitle.extend(column.iter().cloned());
    }
    card_item(&title, nav, &subtitle, &renderer_thumbnail(&renderer["thumbnail"]), is_explicit(&renderer["badges"]))
}

/// A tile (`musicTwoRowItemRenderer`): grids and carousels.
pub(crate) fn two_row_item(renderer: &Value) -> Option<YtItem> {
    let title = non_blank(runs_text(&renderer["title"]))?;
    card_item(
        &title,
        &renderer["navigationEndpoint"],
        &runs(&renderer["subtitle"]),
        &renderer_thumbnail(&renderer["thumbnailRenderer"]),
        is_explicit(&renderer["subtitleBadges"]),
    )
}

/// The big "top result" card of a search (`musicCardShelfRenderer`).
fn card_shelf_item(renderer: &Value) -> Option<YtItem> {
    let title = non_blank(runs_text(&renderer["title"]))?;
    let nav = or(&renderer["title"]["runs"][0]["navigationEndpoint"], &renderer["onTap"]);
    card_item(
        &title,
        nav,
        &runs(&renderer["subtitle"]),
        &renderer_thumbnail(&renderer["thumbnail"]),
        is_explicit(&renderer["subtitleBadges"]),
    )
}

fn any_item(content: &Value) -> Option<YtItem> {
    let row = &content["musicResponsiveListItemRenderer"];
    if !row.is_null() {
        return list_item(row);
    }
    let tile = &content["musicTwoRowItemRenderer"];
    if tile.is_null() { None } else { two_row_item(tile) }
}

fn rows(contents: &Value) -> Vec<PlaylistTrack> {
    arr(contents).iter().filter_map(|content| list_track(&content["musicResponsiveListItemRenderer"])).collect()
}

fn items(contents: &Value) -> Vec<YtItem> {
    arr(contents).iter().filter_map(any_item).collect()
}

// ── library ──────────────────────────────────────────────────────────────────────────────────────

fn single_column_sections(response: &Value) -> &Value {
    &response["contents"]["singleColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]["content"]
        ["sectionListRenderer"]["contents"]
}

/// `YouTube.library`: the grid or shelf of a library tab.
pub fn library_page(response: &Value) -> ItemsPage {
    for section in arr(single_column_sections(response)) {
        for (key, list) in [("gridRenderer", "items"), ("musicShelfRenderer", "contents")] {
            let renderer = &section[key];
            if !renderer.is_null() {
                return ItemsPage {
                    items: items(&renderer[list]),
                    continuation: continuation_of(&renderer["continuations"]).or_else(|| continuation_item(&renderer[list])),
                };
            }
        }
    }
    ItemsPage::default()
}

/// `YouTube.libraryContinuation`; an empty page ends the list.
pub fn library_continuation(response: &Value) -> ItemsPage {
    let contents = &response["continuationContents"];
    for (key, list) in [("gridContinuation", "items"), ("musicShelfContinuation", "contents")] {
        let renderer = &contents[key];
        if !renderer.is_null() {
            let items = items(&renderer[list]);
            let continuation = if items.is_empty() { None } else { continuation_of(&renderer["continuations"]) };
            return ItemsPage { items, continuation };
        }
    }
    ItemsPage::default()
}

// ── playlists ────────────────────────────────────────────────────────────────────────────────────

/// `YouTube.playlist`: both layouts (two-column, and the detail header that "Liked Music" uses).
pub fn playlist_page(response: &Value, playlist_id: &str) -> Result<PlaylistPage> {
    let id = playlist_id.trim_start_matches("VL");
    let two_column = &response["contents"]["twoColumnBrowseResultsRenderer"];
    let first_column = &two_column["tabs"][0]["tabRenderer"]["content"]["sectionListRenderer"]["contents"];
    let editable = &first_column[0]["musicEditablePlaylistDetailHeaderRenderer"];
    let header =
        or(&first_column[0]["musicResponsiveHeaderRenderer"], &editable["header"]["musicResponsiveHeaderRenderer"]);

    let mut info = PlaylistItem { playlist_id: id.to_owned(), uri: playlist_uri(id), ..Default::default() };
    if header.is_null() {
        let top_editable = &response["header"]["musicEditablePlaylistDetailHeaderRenderer"];
        let detail =
            or(&response["header"]["musicDetailHeaderRenderer"], &top_editable["header"]["musicDetailHeaderRenderer"]);
        info.title = non_blank(text(&detail["title"]["runs"][0]["text"]).to_owned())
            .ok_or(YouTubeError::PlaylistUnavailable)?;
        info.cover = sized(&renderer_thumbnail(&detail["thumbnail"]), 640);
        info.description = runs_text(&detail["description"]);
        info.track_count = count_of(&runs_text(&detail["secondSubtitle"])).unwrap_or(0);
        info.can_edit = !top_editable.is_null();
        let sections = arr(single_column_sections(response));
        let shelf = ["musicPlaylistShelfRenderer", "musicShelfRenderer"]
            .into_iter()
            .find_map(|key| sections.iter().map(|section| &section[key]).find(|shelf| !shelf.is_null()))
            .unwrap_or(&NULL);
        return Ok(PlaylistPage {
            info,
            tracks: rows(&shelf["contents"]),
            continuation: continuation_of(&shelf["continuations"]).or_else(|| continuation_item(&shelf["contents"])),
        });
    }

    info.title =
        non_blank(text(&header["title"]["runs"][0]["text"]).to_owned()).ok_or(YouTubeError::PlaylistUnavailable)?;
    info.cover = sized(&renderer_thumbnail(&header["thumbnail"]), 640);
    info.can_edit = !editable.is_null();
    let owner = &header["straplineTextOne"]["runs"][0];
    info.owner = text(&owner["text"]).to_owned();
    info.owner_id = browse_id(&owner["navigationEndpoint"]).to_owned();
    info.track_count =
        groups(&runs(&header["secondSubtitle"])).iter().find_map(|group| count_of(&joined(group))).unwrap_or(0);
    info.description = [
        &editable["header"]["musicDetailHeaderRenderer"]["description"],
        &header["description"]["musicDescriptionShelfRenderer"]["description"],
    ]
    .into_iter()
    .map(runs_text)
    .chain(arr(first_column).iter().map(|section| runs_text(&section["musicDescriptionShelfRenderer"]["description"])))
    .find(|description| !description.is_empty())
    .unwrap_or_default();

    let secondary = &two_column["secondaryContents"]["sectionListRenderer"];
    let shelf = &secondary["contents"][0]["musicPlaylistShelfRenderer"];
    Ok(PlaylistPage {
        info,
        tracks: rows(&shelf["contents"]),
        continuation: continuation_item(&shelf["contents"])
            .or_else(|| continuation_of(&shelf["continuations"]))
            .or_else(|| continuation_of(&secondary["continuations"])),
    })
}

/// `playlistContinuationPageFromResponse`: the first of four known shapes that holds songs.
pub fn playlist_continuation(response: &Value) -> TracksPage {
    let contents = &response["continuationContents"];
    let appended = &response["onResponseReceivedActions"][0]["appendContinuationItemsAction"]["continuationItems"];
    let section_list = &contents["sectionListContinuation"];
    let mut sectioned: Vec<&Value> = arr(&section_list["contents"])
        .iter()
        .flat_map(|section| arr(&section["musicPlaylistShelfRenderer"]["contents"]))
        .collect();
    sectioned.extend(arr(appended));
    let shelf = |key: &str| {
        let renderer = &contents[key];
        let list: Vec<&Value> = arr(&renderer["contents"]).iter().collect();
        (list, continuation_of(&renderer["continuations"]).or_else(|| continuation_item(&renderer["contents"])))
    };
    let candidates = [
        (sectioned, continuation_of(&section_list["continuations"]).or_else(|| continuation_item(appended))),
        shelf("musicPlaylistShelfContinuation"),
        shelf("musicShelfContinuation"),
        (arr(appended).iter().collect(), continuation_item(appended)),
    ];
    let parsed: Vec<(bool, Vec<PlaylistTrack>, Option<String>)> = candidates
        .into_iter()
        .map(|(list, continuation)| {
            let tracks =
                list.iter().filter_map(|content| list_track(&content["musicResponsiveListItemRenderer"])).collect();
            (!list.is_empty(), tracks, continuation)
        })
        .collect();
    let chosen = parsed
        .iter()
        .position(|(_, tracks, _)| !tracks.is_empty())
        .or_else(|| parsed.iter().position(|(has_contents, ..)| *has_contents));
    match chosen.and_then(|index| parsed.into_iter().nth(index)) {
        Some((_, tracks, continuation)) => TracksPage { tracks, continuation },
        None => TracksPage::default(),
    }
}

// ── search ───────────────────────────────────────────────────────────────────────────────────────

fn search_sections(response: &Value) -> &[Value] {
    arr(&response["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]["content"]
        ["sectionListRenderer"]["contents"])
}

/// `YouTube.search` (one filter): every shelf's rows, without duplicates.
pub fn search_page(response: &Value) -> ItemsPage {
    let mut found = Vec::new();
    let mut continuation = None;
    for section in search_sections(response) {
        let nested = arr(&section["itemSectionRenderer"]["contents"]);
        let shelves = std::iter::once(&section["musicShelfRenderer"])
            .chain(nested.iter().map(|content| &content["musicShelfRenderer"]))
            .filter(|shelf| !shelf.is_null());
        for shelf in shelves {
            found.extend(items(&shelf["contents"]));
            continuation = continuation
                .or_else(|| continuation_of(&shelf["continuations"]))
                .or_else(|| continuation_item(&shelf["contents"]));
        }
        found.extend(nested.iter().filter_map(any_item));
    }
    ItemsPage { items: dedupe(found), continuation }
}

/// `YouTube.searchContinuation`.
pub fn search_continuation(response: &Value) -> ItemsPage {
    let shelf = &response["continuationContents"]["musicShelfContinuation"];
    let items = items(&shelf["contents"]);
    let continuation = if items.is_empty() {
        None
    } else {
        continuation_of(&shelf["continuations"]).or_else(|| continuation_item(&shelf["contents"]))
    };
    ItemsPage { items, continuation }
}

fn shelf_title(renderer: &Value) -> String {
    non_blank(runs_text(&renderer["title"])).unwrap_or_else(|| mf_i18n::t("Other"))
}

/// `YouTube.searchSummary` (no filter): "Top results" plus one shelf per kind.
pub fn search_summary(response: &Value) -> Vec<Shelf> {
    let mut top = Vec::new();
    let mut shelves = Vec::new();
    let mut add_shelf = |renderer: &Value| {
        let found = dedupe(items(&renderer["contents"]));
        if !found.is_empty() {
            shelves.push(Shelf { title: shelf_title(renderer), items: found, more: None, continuation: None });
        }
    };
    for section in search_sections(response) {
        let card = &section["musicCardShelfRenderer"];
        if !card.is_null() {
            top.extend(card_shelf_item(card));
            top.extend(items(&card["contents"]));
            continue;
        }
        let nested = &section["itemSectionRenderer"]["contents"];
        if !nested.is_null() {
            top.extend(arr(nested).iter().filter_map(any_item));
            arr(nested).iter().map(|content| &content["musicShelfRenderer"]).filter(|shelf| !shelf.is_null()).for_each(&mut add_shelf);
            continue;
        }
        if !section["musicShelfRenderer"].is_null() {
            add_shelf(&section["musicShelfRenderer"]);
        }
    }
    let top = dedupe(top);
    if !top.is_empty() {
        shelves.insert(0, Shelf { title: mf_i18n::t("Top results"), items: top, more: None, continuation: None });
    }
    shelves
}

/// `YouTube.searchSuggestions`.
pub fn suggestions(response: &Value) -> Suggestions {
    let section = |index: usize| arr(&response["contents"][index]["searchSuggestionsSectionRenderer"]["contents"]);
    Suggestions {
        queries: section(0)
            .iter()
            .filter_map(|content| non_blank(runs_text(&content["searchSuggestionRenderer"]["suggestion"])))
            .collect(),
        items: section(1).iter().filter_map(any_item).collect(),
    }
}

// ── shelves (home, explore, charts, artist pages) ────────────────────────────────────────────────

/// The section list of a browse response, wherever the layout puts it.
fn section_list(response: &Value) -> &Value {
    let contents = &response["contents"];
    [
        &contents["singleColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]["content"]["sectionListRenderer"],
        &contents["twoColumnBrowseResultsRenderer"]["secondaryContents"]["sectionListRenderer"],
        &contents["sectionListRenderer"],
        &response["continuationContents"]["sectionListContinuation"],
    ]
    .into_iter()
    .find(|list| !list.is_null())
    .unwrap_or(&NULL)
}

fn push_shelf(out: &mut Vec<Shelf>, title: String, items: Vec<YtItem>, more: Option<BrowseRef>, continuation: Option<String>) {
    if !items.is_empty() {
        out.push(Shelf { title, items, more, continuation });
    }
}

/// Every known kind of section as a titled shelf; empty and unknown sections are skipped.
pub fn sections(contents: &Value) -> Vec<Shelf> {
    let mut out = Vec::new();
    for section in arr(contents) {
        let carousel = &section["musicCarouselShelfRenderer"];
        if !carousel.is_null() {
            let header = &carousel["header"]["musicCarouselShelfBasicHeaderRenderer"];
            let more = or(
                &header["moreContentButton"]["buttonRenderer"]["navigationEndpoint"]["browseEndpoint"],
                &header["title"]["runs"][0]["navigationEndpoint"]["browseEndpoint"],
            );
            push_shelf(&mut out, runs_text(&header["title"]), items(&carousel["contents"]), browse_ref(more), None);
        }
        for key in ["musicShelfRenderer", "musicPlaylistShelfRenderer"] {
            let shelf = &section[key];
            if !shelf.is_null() {
                push_shelf(
                    &mut out,
                    runs_text(&shelf["title"]),
                    items(&shelf["contents"]),
                    browse_ref(&shelf["bottomEndpoint"]["browseEndpoint"]),
                    continuation_of(&shelf["continuations"]).or_else(|| continuation_item(&shelf["contents"])),
                );
            }
        }
        let grid = &section["gridRenderer"];
        if !grid.is_null() {
            push_shelf(
                &mut out,
                runs_text(&grid["header"]["gridHeaderRenderer"]["title"]),
                items(&grid["items"]),
                None,
                continuation_of(&grid["continuations"]),
            );
        }
        let card = &section["musicCardShelfRenderer"];
        if !card.is_null() {
            let mut found: Vec<YtItem> = card_shelf_item(card).into_iter().collect();
            found.extend(items(&card["contents"]));
            push_shelf(&mut out, mf_i18n::t("Top results"), found, None, None);
        }
        out.extend(sections(&section["itemSectionRenderer"]["contents"]));
    }
    out
}

/// Home, explore, charts, history, a mood's playlists, "see all" pages — and their continuations.
pub fn shelves_page(response: &Value) -> ShelvesPage {
    let list = section_list(response);
    ShelvesPage { shelves: sections(&list["contents"]), continuation: continuation_of(&list["continuations"]) }
}

/// Moods & genres: the navigation buttons of every grid.
pub fn moods(response: &Value) -> Vec<Mood> {
    let mut out = Vec::new();
    for section in arr(&section_list(response)["contents"]) {
        let grid = &section["gridRenderer"];
        let group = runs_text(&grid["header"]["gridHeaderRenderer"]["title"]);
        for item in arr(&grid["items"]) {
            let button = &item["musicNavigationButtonRenderer"];
            let title = runs_text(&button["buttonText"]);
            if let (false, Some(browse)) = (title.is_empty(), browse_ref(&button["clickCommand"]["browseEndpoint"])) {
                out.push(Mood { group: group.clone(), title, browse });
            }
        }
    }
    out
}

// ── album, artist ────────────────────────────────────────────────────────────────────────────────

/// `YouTube.album`: header plus the songs listed on the page itself.
pub fn album_page(response: &Value, browse_id: &str) -> Result<AlbumPage> {
    let two_column = &response["contents"]["twoColumnBrowseResultsRenderer"];
    let responsive =
        &two_column["tabs"][0]["tabRenderer"]["content"]["sectionListRenderer"]["contents"][0]["musicResponsiveHeaderRenderer"];
    let header = or(responsive, &response["header"]["musicDetailHeaderRenderer"]);
    let title = non_blank(text(&header["title"]["runs"][0]["text"]).to_owned())
        .ok_or(YouTubeError::Unexpected("album title"))?;

    let subtitle = runs(&header["subtitle"]);
    let subtitle_groups = groups(&subtitle);
    let strapline = groups(&runs(&header["straplineTextOne"]));
    let artists = match extract_artists(&strapline, &title) {
        artists if artists.is_empty() => extract_artists(&subtitle_groups, &title),
        artists => artists,
    };
    let kind = match subtitle_groups.first().map(|group| joined(group).trim().to_lowercase()).as_deref() {
        Some("single") => "single",
        Some("ep") => "ep",
        _ => "album",
    };
    let thumbnail = renderer_thumbnail(&header["thumbnail"]);
    let cover = sized(&thumbnail, 300);
    let canonical = text(&response["microformat"]["microformatDataRenderer"]["urlCanonical"]);
    let play = arr(&header["buttons"])
        .iter()
        .map(|button| watch(&button["musicPlayButtonRenderer"]["playNavigationEndpoint"]))
        .find(|endpoint| !endpoint.is_null())
        .unwrap_or(&NULL);
    let playlist_id = match canonical.split_once("list=") {
        Some((_, id)) => id.split('&').next().unwrap_or_default().to_owned(),
        None => text(&play["playlistId"]).to_owned(),
    };

    let secondary = &two_column["secondaryContents"]["sectionListRenderer"]["contents"];
    let sections_single = single_column_sections(response);
    let shelf = arr(secondary)
        .iter()
        .chain(arr(sections_single))
        .map(|section| &section["musicShelfRenderer"])
        .find(|shelf| !shelf.is_null())
        .unwrap_or(&NULL);
    let mut tracks: Vec<TrackItem> = rows(&shelf["contents"]).into_iter().map(|row| row.track).collect();
    for track in &mut tracks {
        if track.album.is_empty() {
            track.album = title.clone();
        }
        // the row's own album link (if any) is the same album
        track.album_id = browse_id.to_owned();
        if track.cover.is_empty() {
            track.cover = cover.clone();
            track.cover_large = sized(&thumbnail, 640);
        }
        if track.artist_refs.is_empty() {
            track.artist_refs = artists.clone();
            track.artists = artist_names(&artists);
        }
    }
    let year = year_of(&subtitle_groups);
    let listed = groups(&runs(&header["secondSubtitle"])).iter().find_map(|group| count_of(&joined(group)));
    let other_versions = arr(secondary)
        .iter()
        .flat_map(|section| arr(&section["musicCarouselShelfRenderer"]["contents"]))
        .filter_map(|content| match two_row_item(&content["musicTwoRowItemRenderer"]) {
            Some(YtItem::Album(album)) => Some(album),
            _ => None,
        })
        .collect();
    Ok(AlbumPage {
        info: AlbumInfo {
            item: AlbumItem {
                album_id: browse_id.to_owned(),
                uri: album_uri(browse_id),
                title,
                version: String::new(),
                artists: artist_names(&artists),
                artist_refs: artists,
                year,
                kind: album_kind_label(kind, 0),
                track_count: listed.unwrap_or(tracks.len() as i64),
                cover,
                cover_large: sized(&thumbnail, 640),
                explicit: tracks.iter().any(|track| track.explicit),
                saved: false,
            },
            duration_ms: tracks.iter().map(|track| track.duration_ms).sum(),
            release_date: if year > 0 { year.to_string() } else { String::new() },
            ..Default::default()
        },
        playlist_id,
        description: runs_text(or(
            &header["description"]["musicDescriptionShelfRenderer"]["description"],
            &header["description"],
        )),
        tracks,
        other_versions,
    })
}

/// `YouTube.artist`: any of the three header kinds plus the page's shelves.
pub fn artist_page(response: &Value, browse_id: &str) -> Result<ArtistPage> {
    let headers = &response["header"];
    let header = ["musicImmersiveHeaderRenderer", "musicVisualHeaderRenderer", "musicHeaderRenderer"]
        .into_iter()
        .map(|key| &headers[key])
        .find(|header| !header.is_null())
        .ok_or(YouTubeError::Unexpected("artist header"))?;
    let name = non_blank(runs_text(&header["title"])).ok_or(YouTubeError::Unexpected("artist name"))?;
    let thumbnail = [&header["thumbnail"], &header["foregroundThumbnail"]]
        .into_iter()
        .map(renderer_thumbnail)
        .find(|url| !url.is_empty())
        .unwrap_or_default();
    let subscribe = &header["subscriptionButton"]["subscribeButtonRenderer"];
    Ok(ArtistPage {
        info: ArtistInfo {
            item: ArtistItem {
                artist_id: browse_id.to_owned(),
                uri: artist_uri(browse_id),
                name,
                cover: sized(&thumbnail, 300),
                cover_large: sized(&thumbnail, 640),
                genres: String::new(),
                followers: parse_abbreviated(&runs_text(&subscribe["subscriberCountText"])),
            },
            listeners: parse_abbreviated(&runs_text(&header["monthlyListenerCount"])),
            followed: subscribe["subscribed"].as_bool().unwrap_or(false),
            biography: runs_text(&header["description"]),
            header_image: sized(&thumbnail, 1440),
        },
        channel_id: text(&subscribe["channelId"]).to_owned(),
        shelves: sections(&section_list(response)["contents"]),
    })
}

// ── queue, account, lyrics, mutations ────────────────────────────────────────────────────────────

fn panel_track(renderer: &Value) -> Option<TrackItem> {
    let video_id = non_blank(text(&renderer["videoId"]).to_owned())?;
    let title = non_blank(text(&renderer["title"]["runs"][0]["text"]).to_owned())?;
    let byline = runs(&renderer["longBylineText"]);
    let segments = groups(&byline);
    let album = extract_album_runs(&byline).or_else(|| {
        segments.get(1).and_then(|group| group.first()).filter(|run| !run.nav["browseEndpoint"].is_null()).map(album_of)
    });
    Some(make_track(
        &video_id,
        title,
        to_artists(segments.first()?),
        album,
        parse_time(text(&renderer["lengthText"]["runs"][0]["text"])),
        &thumbnail_url(&renderer["thumbnail"]),
        is_explicit(&renderer["badges"]),
    ))
}

/// Returns the next page token and optionally automix playlist ID and params.
pub type AutomixInfo = Option<(String, Option<String>)>;

/// `YouTube.next`: the queue, and the automix playlist to follow when the panel ends with a preview.
pub fn next_page(response: &Value) -> Result<(NextPage, AutomixInfo)> {
    let tabs = &response["contents"]["singleColumnMusicWatchNextResultsRenderer"]["tabbedRenderer"]
        ["watchNextTabbedResultsRenderer"]["tabs"];
    let queue = &tabs[0]["tabRenderer"]["content"]["musicQueueRenderer"];
    let panel = or(&response["continuationContents"]["playlistPanelContinuation"], &queue["content"]["playlistPanelRenderer"]);
    if panel.is_null() {
        return Err(YouTubeError::Unexpected("queue panel"));
    }
    let mut tracks = Vec::new();
    let mut current_index = None;
    for content in arr(&panel["contents"]) {
        let renderer = &content["playlistPanelVideoRenderer"];
        if let Some(track) = panel_track(renderer) {
            if renderer["selected"].as_bool() == Some(true) && current_index.is_none() {
                current_index = Some(tracks.len());
            }
            tracks.push(track);
        }
    }
    let automix = arr(&panel["contents"]).last().and_then(|content| {
        let endpoint = &content["automixPreviewVideoRenderer"]["content"]["automixPlaylistVideoRenderer"]
            ["navigationEndpoint"]["watchPlaylistEndpoint"];
        let playlist_id = non_blank(text(&endpoint["playlistId"]).to_owned())?;
        Some((playlist_id, endpoint["params"].as_str().map(str::to_owned)))
    });
    let page = NextPage {
        title: text(&queue["header"]["musicQueueHeaderRenderer"]["subtitle"]["runs"][0]["text"]).to_owned(),
        tracks,
        current_index,
        continuation: continuation_of(&panel["continuations"]),
        lyrics: browse_ref(&tabs[1]["tabRenderer"]["endpoint"]["browseEndpoint"]),
        related: browse_ref(&tabs[2]["tabRenderer"]["endpoint"]["browseEndpoint"]),
    };
    Ok((page, automix))
}

/// `YouTube.accountInfo`; `None` when the response is not a signed-in account menu.
pub fn account_info(response: &Value) -> Option<AccountInfo> {
    let header = &response["actions"][0]["openPopupAction"]["popup"]["multiPageMenuRenderer"]["header"]
        ["activeAccountHeaderRenderer"];
    let name = non_blank(runs_text(&header["accountName"]))?;
    Some(AccountInfo { name, email: runs_text(&header["email"]), avatar: thumbnail_url(&header["accountPhoto"]) })
}

/// Plain lyrics of a track's lyrics tab.
pub fn lyrics(response: &Value) -> Option<String> {
    let shelf = &response["contents"]["sectionListRenderer"]["contents"][0]["musicDescriptionShelfRenderer"];
    non_blank(runs_text(&shelf["description"]))
}

/// `YouTube.transcript`: cue groups as LRC lines (`[mm:ss.mmm]text`).
pub fn transcript(response: &Value) -> Option<String> {
    let cues = &response["actions"][0]["updateEngagementPanelAction"]["content"]["transcriptRenderer"]["body"]
        ["transcriptBodyRenderer"]["cueGroups"];
    let lines: Vec<String> = arr(cues)
        .iter()
        .filter_map(|group| {
            let cue = &group["transcriptCueGroupRenderer"]["cues"][0]["transcriptCueRenderer"];
            let start = &cue["startOffsetMs"];
            let ms = start.as_i64().or_else(|| text(start).parse().ok())?;
            let line = text(&cue["cue"]["simpleText"]).trim_matches(|c| c == '♪' || c == ' ');
            Some(format!("[{:02}:{:02}.{:03}]{line}", ms / 60_000, (ms / 1000) % 60, ms % 1000))
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Id of the playlist a `playlist/create` call made.
pub fn created_playlist_id(response: &Value) -> Option<String> {
    non_blank(text(&response["playlistId"]).to_owned())
}

/// A `browse/edit_playlist` answer: whether it succeeded, and the set-video ids of added rows.
pub fn edit_result(response: &Value) -> (bool, Vec<String>) {
    let added = arr(&response["playlistEditResults"])
        .iter()
        .filter_map(|result| non_blank(text(&result["playlistEditVideoAddedResultData"]["setVideoId"]).to_owned()))
        .collect();
    (text(&response["status"]) == "STATUS_SUCCEEDED", added)
}
