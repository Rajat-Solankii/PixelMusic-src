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
                text: I18n.n(page.sections.reduce((n, s) => n + (s.items ?? []).length, 0), "{0} item", "{0} items")
                type: "bodyLarge"
                color: Theme.fgSurfaceVariant
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

            SectionHeader {
                // a full page of several shelves names each one; the shelf itself is already the page title
                visible: page.sections.length > 1 && (section.modelData.title ?? "") !== ""
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

            // moods & genres: tonal pills that open their own page; the three container roles in turn
            Flow {
                visible: section.kind === "moods"
                Layout.fillWidth: true
                spacing: 8
                Repeater {
                    model: section.kind === "moods" ? section.items : []
                    Rectangle {
                        id: mood
                        required property var modelData
                        required property int index
                        readonly property var role: [[Theme.primaryContainer, Theme.fgPrimaryContainer],
                                                     [Theme.secondaryContainer, Theme.fgSecondaryContainer],
                                                     [Theme.tertiaryContainer, Theme.fgTertiaryContainer]][(index + section.index) % 3]
                        implicitWidth: moodLabel.implicitWidth + 40
                        implicitHeight: 56
                        radius: moodArea.pressed ? Theme.shape.medium : height / 2
                        Behavior on radius { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack; easing.overshoot: Theme.motion.overshoot } }
                        color: role[0]
                        Label {
                            id: moodLabel
                            anchors.centerIn: parent
                            text: mood.modelData.title ?? ""
                            type: "titleSmall"
                            weight: 600
                            color: mood.role[1]
                        }
                        StateLayer {
                            id: moodArea
                            anchors.fill: parent
                            radius: mood.radius
                            color: mood.role[1]
                            onClicked: page.openEntry(mood.modelData)
                        }
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
