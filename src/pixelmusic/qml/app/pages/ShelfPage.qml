import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core
import ".."

// "See all" of a YouTube Music Home shelf ("Similar to …", community playlists, your mixes): the shelf's own
// items at once, then its full page (Home.browseYouTube) when the shelf has one. shelf: a Home shelf
// ({key, title, kind, items, strapline?, thumbnail?, browseId?, browseParams?}).
Page {
    id: page

    property var shelf: ({})

    title: shelf.title ?? ""
    showHeader: false

    readonly property bool hasFullPage: (shelf.browseId ?? "") !== ""
    // the full page once it arrived, else the shelf itself (also when the full page is empty or fails)
    readonly property var sections: {
        const loaded = hasFullPage && !Home.browseLoading ? (Home.browseShelves ?? []) : []
        return loaded.length > 0 ? loaded : [shelf]
    }
    readonly property int columns: Math.max(2, Math.floor((width - 2 * sideMargin + 24) / (168 + 24)))
    // Moods & genres: chips pick one group ("Moods & moments", "Genres"), cards in 2+ columns
    readonly property bool isMoods: sections.length > 0 && sections.every(s => (s.kind ?? "") === "moods")
    property string moodFilter: ""          // "" — all groups, else a section title
    readonly property int moodColumns: Math.max(2, Math.floor((width - 2 * sideMargin + 16) / (220 + 16)))
    // a colour for categories YouTube gave none: the scheme's own container tones in turn
    function moodTint(item, i) {
        const c = item.color ?? ""
        if (c !== "")
            return c
        return [Theme.primary, Theme.tertiary, Theme.secondary][i % 3]
    }

    // Home keeps one browse page: a shelf page opened from this one (a mood or genre) replaces it, so coming back
    // reads this page's own again
    property bool _loaded: false
    function _load() { Home.browseYouTube(shelf.browseId ?? "", shelf.browseParams ?? ""); _loaded = true }
    Component.onCompleted: _load()
    onVisibleChanged: if (visible && _loaded && hasFullPage) _load()

    function openEntry(e) {
        const k = e.kind ?? ""
        if (k === "browse")
            router.push("shelf", { shelf: { key: "youtube:browse:" + (e.browseId ?? ""), title: e.title ?? "", kind: "tiles", items: [],
                                             browseId: e.browseId ?? "", browseParams: e.browseParams ?? "" } })
        else if (k === "playlist")
            router.push("playlist", { playlistId: e.id, playlistTitle: e.title ?? "", cover: e.cover ?? "" })
        else if (e.uri)
            Catalog.openUri(e.uri)
    }
    // New releases: Play / Shuffle queue the songs of the first 15 singles and EPs, in list order
    readonly property bool isNewReleases: (shelf.browseId ?? "") === "FEmusic_new_releases_albums"
    readonly property var releaseUris: {
        if (!isNewReleases)
            return []
        const uris = []
        for (const s of sections)
            for (const e of (s.items ?? []))
                if (uris.length < 15 && (e.kind ?? "") === "album" && ["single", "ep"].includes(e.albumType ?? "") && (e.uri ?? "") !== "")
                    uris.push(e.uri)
        return uris
    }
    property string releasesBusy: ""      // "play" | "shuffle" while the releases' songs are read
    function playReleases(shuffle) {
        if (releaseUris.length === 0 || releasesBusy !== "")
            return
        releasesBusy = shuffle ? "shuffle" : "play"
        Player.playAlbums(releaseUris, shuffle, I18n.t("New releases"))
    }
    Connections {
        target: Player
        function onAlbumsLoaded(ok) { page.releasesBusy = "" }
    }
    Timer {
        running: page.releasesBusy !== ""
        interval: 30000
        onTriggered: page.releasesBusy = ""
    }

    function playEntry(e) {
        if (Player.playing && (e.uri ?? "") !== "" && Player.contextUri === e.uri)
            Player.togglePlay()
        else if (e.uri)
            Player.playContext(e.uri, "")
    }

    // ---- header: the strapline over the title, the artist's picture of a "Similar to" shelf ----
    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: 16
        spacing: 20
        Avatar {
            visible: (page.shelf.thumbnail ?? "") !== ""
            size: 96
            shape: "circle"
            name: page.shelf.title ?? ""
            source: page.shelf.thumbnail ?? ""
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            Label {
                visible: text !== ""
                Layout.fillWidth: true
                text: (page.shelf.strapline ?? "").toUpperCase()
                type: "labelLarge"
                weight: 600
                color: Theme.fgSurfaceVariant
            }
            Label {
                Layout.fillWidth: true
                text: page.shelf.title ?? ""
                type: "displaySmall"
                weight: 600
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                visible: !page.isMoods
                text: I18n.n(page.sections.reduce((n, s) => n + (s.items ?? []).length, 0), "{0} item", "{0} items")
                type: "bodyLarge"
                color: Theme.fgSurfaceVariant
            }
            RowLayout {
                visible: page.isNewReleases && page.releaseUris.length > 0
                Layout.topMargin: 12
                spacing: 12
                Button {
                    text: I18n.t("Play")
                    icon: "play_arrow"
                    size: "m"
                    enabled: page.releasesBusy === ""
                    opacity: page.releasesBusy === "shuffle" ? 0.6 : 1
                    onClicked: page.playReleases(false)
                }
                Button {
                    text: I18n.t("Shuffle")
                    icon: "shuffle"
                    style: "tonal"
                    size: "m"
                    enabled: page.releasesBusy === ""
                    opacity: page.releasesBusy === "play" ? 0.6 : 1
                    onClicked: page.playReleases(true)
                }
                LoadingIndicator {
                    size: 36
                    visible: page.releasesBusy !== ""
                }
                Label {
                    Layout.fillWidth: true
                    text: page.releasesBusy !== "" ? I18n.t("Getting the songs ready…")
                                                   : I18n.n(page.releaseUris.length, "Songs from the newest {0} single or EP", "Songs from the newest {0} singles and EPs")
                    type: "bodyMedium"
                    color: Theme.fgSurfaceVariant
                    elide: Text.ElideRight
                }
            }
        }
    }

    Flow {
        Layout.fillWidth: true
        Layout.topMargin: -4
        visible: page.isMoods && page.sections.length > 1
        spacing: 8
        Chip {
            text: I18n.t("All")
            selected: page.moodFilter === ""
            onClicked: page.moodFilter = ""
        }
        Repeater {
            model: page.isMoods ? page.sections : []
            Chip {
                required property var modelData
                text: modelData.title ?? ""
                selected: page.moodFilter === text
                onClicked: page.moodFilter = text
            }
        }
    }

    Repeater {
        model: page.sections
        ColumnLayout {
            id: section
            required property var modelData
            required property int index
            readonly property string kind: modelData.kind ?? "tiles"
            readonly property var items: modelData.items ?? []
            Layout.fillWidth: true
            spacing: 12
            visible: !page.isMoods || page.moodFilter === "" || page.moodFilter === (modelData.title ?? "")

            SectionHeader {
                // a full page of several shelves names each one; the shelf itself is already the page title
                visible: !page.isMoods && page.sections.length > 1 && (section.modelData.title ?? "") !== ""
                text: section.modelData.title ?? ""
            }

            // songs: a list that plays as one
            ColumnLayout {
                visible: section.kind === "tracks"
                Layout.fillWidth: true
                Layout.leftMargin: -12
                Layout.rightMargin: -12
                spacing: 0
                Repeater {
                    model: section.kind === "tracks" ? section.items : []
                    TrackRow {
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        number: index + 1
                        track: modelData
                        trackId: modelData.trackId ?? ""
                        title: modelData.title ?? ""
                        version: modelData.version ?? ""
                        artists: modelData.artists ?? ""
                        artistRefs: modelData.artistRefs ?? []
                        albumId: modelData.albumId ?? ""
                        album: modelData.album ?? ""
                        cover: modelData.cover ?? ""
                        uri: modelData.uri ?? ""
                        durationMs: modelData.durationMs ?? 0
                        explicit: modelData.explicit ?? false
                        available: modelData.available ?? true
                        liked: Library.likesRevision >= 0 && Library.isLiked(trackId)
                        current: trackId !== "" && trackId === Player.trackId
                        onActivated: {
                            Player.rememberTrack(modelData)
                            Player.playTracks(section.items, index)
                        }
                        onLikeClicked: Library.toggleLike(modelData)
                    }
                }
            }

            // moods & genres: the group's name with a count badge, then a grid of colour cards
            RowLayout {
                visible: section.kind === "moods"
                Layout.fillWidth: true
                Layout.topMargin: 12
                spacing: 12
                Label {
                    Layout.fillWidth: true
                    text: section.modelData.title ?? ""
                    type: "headlineSmall"
                    weight: 700
                    elide: Text.ElideRight
                }
                Rectangle {
                    implicitWidth: Math.max(height, countLabel.implicitWidth + 24)
                    implicitHeight: 32
                    radius: height / 2
                    color: Theme.surfaceContainerHigh
                    Label {
                        id: countLabel
                        anchors.centerIn: parent
                        text: section.items.length
                        type: "labelLarge"
                        weight: 600
                        color: Theme.fgSurfaceVariant
                    }
                }
            }
            GridLayout {
                visible: section.kind === "moods"
                Layout.fillWidth: true
                columns: page.moodColumns
                columnSpacing: 16
                rowSpacing: 16
                Repeater {
                    model: section.kind === "moods" ? section.items : []
                    MoodCard {
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        Layout.preferredWidth: 1
                        Layout.preferredHeight: Math.round(width / 1.42)
                        title: modelData.title ?? ""
                        browseId: modelData.browseId ?? ""
                        browseParams: modelData.browseParams ?? ""
                        tint: page.moodTint(modelData, index + section.index)
                        onClicked: page.openEntry(modelData)
                    }
                }
            }

            // albums, playlists, artists: a grid of tiles
            GridLayout {
                visible: section.kind !== "tracks" && section.kind !== "moods"
                Layout.fillWidth: true
                columns: page.columns
                columnSpacing: 24
                rowSpacing: 20
                Repeater {
                    model: section.kind !== "tracks" && section.kind !== "moods" ? section.items : []
                    MediaTile {
                        required property var modelData
                        // "artists" shelves hold ArtistItems, the rest LibraryEntry tiles
                        readonly property bool artistItem: section.kind === "artists"
                        readonly property string entryKind: artistItem ? "artist" : (modelData.kind ?? "")
                        readonly property var entry: artistItem
                            ? { kind: "artist", uri: modelData.uri ?? "", title: modelData.name ?? "", cover: modelData.cover ?? "" }
                            : modelData
                        Layout.alignment: Qt.AlignTop
                        size: 168
                        cover: entry.cover ?? ""
                        coverKind: entryKind === "album" || entryKind === "artist" ? entryKind : ""
                        coverArtist: entryKind === "artist" ? (entry.title ?? "") : (entry.subtitle ?? "").split(", ")[0]
                        coverTitle: entryKind === "album" ? (entry.title ?? "") : ""
                        title: entry.title ?? ""
                        subtitle: artistItem ? I18n.t("Artist") : (entry.subtitle ?? "")
                        badge: entry.albumType === "single" ? I18n.t("Single").toUpperCase() : entry.albumType === "ep" ? "EP" : ""
                        round: entryKind === "artist"
                        placeholderIcon: ({ playlist: "queue_music", album: "album", artist: "person", track: "music_note" })[entryKind] ?? "album"
                        showPlay: entryKind !== "artist"
                        playing: Player.playing && (entry.uri ?? "") !== "" && Player.contextUri === entry.uri
                        onClicked: page.openEntry(entry)
                        onPlayClicked: page.playEntry(entry)
                    }
                }
            }
        }
    }

    LoadingIndicator {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 24
        size: 48
        contained: true
        visible: page.hasFullPage && Home.browseLoading
    }
}
