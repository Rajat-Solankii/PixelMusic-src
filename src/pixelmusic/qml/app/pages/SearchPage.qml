import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core
import ".."

// Поиск: поле с задержкой ввода, фильтры-чипы. «Всё» — лучший результат + по несколько из каждой
// категории, остальные фильтры — полный список одной категории.
Page {
    id: page
    title: I18n.t("Search")
    showHeader: false

    property int filter: 0
    readonly property var filters: [
        { text: I18n.t("All"), type: "all" }, { text: I18n.t("Tracks"), type: "tracks" }, { text: I18n.t("Albums"), type: "albums" },
        { text: I18n.t("Artists"), type: "artists" }, { text: I18n.t("Playlists"), type: "playlists" }
    ]
    function showFilter(type) { filter = Math.max(0, filters.findIndex(f => f.type === type)) }
    readonly property string type: filters[filter].type
    readonly property bool all: type === "all"
    // search goes to YouTube Music and needs no account
    readonly property bool signedIn: true
    // в старом бинарнике новых моделей поиска может ещё не быть
    readonly property var showsModel: Catalog.searchShows ?? null
    readonly property var episodesModel: Catalog.searchEpisodes ?? null
    readonly property int perRow: Math.max(1, Math.floor((width - 2 * sideMargin + 20) / (168 + 20)))

    // empty field: Browse & Discover and the recent searches instead of results
    readonly property bool idle: field.text.trim() === ""
    readonly property bool showHistory: idle && Catalog.searchHistory.length > 0
    // YouTube Music's own browse pages, opened as a shelf page; each tile takes one colour role of the scheme
    readonly property var browseTiles: [
        { title: I18n.t("New releases"), subtitle: I18n.t("Fresh albums & singles"), icon: "album",
          browseId: "FEmusic_new_releases_albums", role: [Theme.primary, Theme.fgPrimary] },
        { title: I18n.t("Charts"), subtitle: I18n.t("Top trending tracks"), icon: "insights",
          browseId: "FEmusic_charts", role: [Theme.secondaryContainer, Theme.fgSecondaryContainer] },
        { title: I18n.t("Moods & genres"), subtitle: I18n.t("Vibes for every mood"), icon: "mood",
          browseId: "FEmusic_moods_and_genres", role: [Theme.tertiaryContainer, Theme.fgTertiaryContainer] },
        { title: I18n.t("Podcasts"), subtitle: I18n.t("Shows & episodes"), icon: "podcasts",
          browseId: "FEmusic_non_music_audio", role: [Theme.surfaceContainerHighest, Theme.fgSurfaceVariant] }
    ]
    function openBrowse(t) {
        page.router.push("shelf", { shelf: { key: "youtube:browse:" + t.browseId, title: t.title, kind: "tiles", items: [], browseId: t.browseId } })
    }

    function followersText(n) {
        if (!(n > 0))
            return ""
        return I18n.nCompact(n, "{0} follower", "{0} followers")
    }
    function focusSearch() { field.inputItem.forceActiveFocus() }
    // запрос уже запущен снаружи (Catalog.openUri("spotify:search:…")): только показать его
    function showQuery(q) {
        if (q === "" || q === field.text)
            return
        filter = 0
        field.text = q
        debounce.stop()
        // Record this query in history
        Catalog.addSearchQuery(q)
    }
    function runSearch(q) {
        if (q.trim() === "")
            return
        field.text = q
        debounce.stop()
        Catalog.search(q, page.type)
        Catalog.addSearchQuery(q)
    }
    // The router keeps this page alive (results stay built between visits) and may build it hidden in advance:
    // each time it is shown, pick up a query started elsewhere and focus the field
    function _shown() {
        if (!visible)
            return
        showQuery(Catalog.searchQuery)
        Qt.callLater(focusSearch)
    }
    Component.onCompleted: Qt.callLater(_shown)
    onVisibleChanged: _shown()

    // the search field, chips, Browse & Discover and recent searches follow the window: the whole width up to an
    // ultra-wide cap (lines that long stop reading as one block)
    readonly property real sectionWidth: 1400

    Timer {
        id: debounce
        interval: 350
        // typing searches as you go; history keeps only what was sent with Enter or picked from it
        onTriggered: Catalog.search(field.text, page.type)
    }
    onTypeChanged: if (field.text.trim() !== "") Catalog.search(field.text, page.type)

    SearchField {
        id: field
        Layout.fillWidth: true
        Layout.maximumWidth: page.sectionWidth
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 8
        implicitHeight: 56
        placeholder: I18n.t("Search for any tracks, albums, artists…")
        onTextChanged: debounce.restart()
        // Enter — искать сразу, не дожидаясь паузы ввода; Esc — очистить поле
        onAccepted: {
            debounce.stop()
            if (field.text.trim() !== "") {
                Catalog.search(field.text, page.type)
                Catalog.addSearchQuery(field.text)
            }
        }
        Keys.onEscapePressed: event => {
            if (field.text !== "")
                field.text = ""
            else
                event.accepted = false
        }
    }

    Flow {
        Layout.fillWidth: true
        Layout.maximumWidth: page.sectionWidth
        Layout.alignment: Qt.AlignHCenter
        // filters only once there is something to filter
        visible: !page.idle
        spacing: 8
        Repeater {
            model: page.filters
            Chip {
                required property var modelData
                required property int index
                text: modelData.text
                selected: page.filter === index
                onClicked: page.filter = index
            }
        }
    }

    // ---- Browse & Discover (empty field) ----
    // Material 3 Expressive tiles: each tinted by its own colour role of the app's dynamic scheme (primary,
    // secondary, tertiary, neutral) — an extra-large rounded card with a soft vertical wash and a round role icon
    Label {
        Layout.fillWidth: true
        Layout.maximumWidth: page.sectionWidth
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 16
        visible: page.idle
        text: I18n.t("Browse & Discover")
        type: "headlineSmall"
        weight: 700
    }
    GridLayout {
        id: browseGrid
        // all four side by side; a narrow window falls back to two per row
        readonly property bool oneRow: width >= 640
        Layout.fillWidth: true
        Layout.maximumWidth: page.sectionWidth
        Layout.alignment: Qt.AlignHCenter
        visible: page.idle
        columns: oneRow ? 4 : 2
        columnSpacing: oneRow ? 12 : 16
        rowSpacing: 16
        Repeater {
            model: page.browseTiles
            Rectangle {
                id: tile
                required property var modelData
                readonly property color roleColor: modelData.role[0]
                readonly property color onRoleColor: modelData.role[1]
                Layout.fillWidth: true
                Layout.preferredWidth: 1      // equal columns, whatever the text width
                Layout.preferredHeight: 150
                radius: tileArea.pressed ? Theme.shape.large : Theme.shape.extraLarge
                Behavior on radius { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack; easing.overshoot: Theme.motion.overshoot } }
                scale: tileArea.pressed ? 0.97 : 1
                Behavior on scale { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack; easing.overshoot: Theme.motion.overshoot } }
                gradient: Gradient {
                    GradientStop { position: 0; color: Qt.tint(Theme.surfaceContainerLow, Qt.alpha(tile.roleColor, Theme.dark ? 0.38 : 0.55)) }
                    GradientStop { position: 1; color: Theme.surfaceContainerLow }
                }

                Rectangle {
                    x: browseGrid.oneRow ? 16 : 20
                    y: browseGrid.oneRow ? 16 : 20
                    width: 52
                    height: 52
                    radius: 26
                    color: tile.roleColor
                    Icon { anchors.centerIn: parent; name: tile.modelData.icon; size: 26; fill: 1; weight: 500; color: tile.onRoleColor }
                }
                ColumnLayout {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: browseGrid.oneRow ? 16 : 20
                    spacing: 2
                    Label { Layout.fillWidth: true; text: tile.modelData.title; type: browseGrid.oneRow ? "titleMedium" : "titleLarge"; weight: 600; elide: Text.ElideRight }
                    Label {
                        Layout.fillWidth: true
                        text: tile.modelData.subtitle
                        type: browseGrid.oneRow ? "bodySmall" : "bodyMedium"
                        color: Theme.fgSurfaceVariant
                        // narrow tiles: up to two lines rather than cutting the hint off
                        wrapMode: Text.WordWrap
                        maximumLineCount: 2
                        elide: Text.ElideRight
                    }
                }
                StateLayer {
                    id: tileArea
                    anchors.fill: parent
                    radius: tile.radius
                    color: tile.onRoleColor
                    onClicked: page.openBrowse(tile.modelData)
                }
            }
        }
    }

    // ---- Recent searches ----
    ColumnLayout {
        Layout.fillWidth: true
        Layout.maximumWidth: page.sectionWidth
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 24
        visible: page.showHistory
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            Layout.bottomMargin: 4
            spacing: 16

            Label {
                Layout.fillWidth: true
                text: I18n.t("Recent searches")
                type: "headlineSmall"
                weight: 700
            }
            Button {
                style: "text"
                size: "s"
                text: I18n.t("Clear all")
                onClicked: Catalog.clearSearchHistory()
            }
        }

        Repeater {
            model: Catalog.searchHistory

            // a full pill: the whole row searches again (a played song plays again), the bin on its right forgets it
            Rectangle {
                id: historyRow
                required property var modelData
                readonly property string query: modelData && modelData.query ? modelData.query : ""
                readonly property var track: modelData && modelData.track ? modelData.track : null
                readonly property var artist: modelData && modelData.artist ? modelData.artist : null
                readonly property string cover: track ? (track.cover ?? "") : artist ? (artist.cover ?? "") : ""
                readonly property string key: modelData && modelData.key ? modelData.key : query
                Layout.fillWidth: true
                implicitHeight: 64
                radius: rowArea.pressed ? Theme.shape.large : height / 2
                Behavior on radius { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack; easing.overshoot: Theme.motion.overshoot } }
                color: Theme.surfaceContainerLow

                StateLayer {
                    id: rowArea
                    anchors.fill: parent
                    radius: historyRow.radius
                    onClicked: {
                        if (historyRow.track)
                            Player.playTrackRadio(historyRow.track)
                        else if (historyRow.artist)
                            Catalog.openArtist(historyRow.artist.artistId)
                        else
                            page.runSearch(historyRow.query)
                    }
                }
                Rectangle {
                    id: historyIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    width: 48
                    height: 48
                    radius: 24
                    color: Theme.secondaryContainer
                    Icon { visible: historyRow.cover === ""; anchors.centerIn: parent; name: "history"; size: 24; weight: 500; color: Theme.fgSecondaryContainer }
                    // a played song or an opened artist: the cover / photo
                    MorphImage {
                        anchors.fill: parent
                        visible: historyRow.cover !== ""
                        source: historyRow.cover
                        shape: "circle"
                    }
                }
                Column {
                    anchors.left: historyIcon.right
                    anchors.leftMargin: 16
                    anchors.right: forget.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    Label {
                        width: parent.width
                        text: historyRow.query
                        type: "bodyLarge"
                        elide: Text.ElideRight
                    }
                    Label {
                        visible: historyRow.track !== null || historyRow.artist !== null
                        width: parent.width
                        text: historyRow.track ? (historyRow.track.artists ?? "") : I18n.t("Artist")
                        type: "bodySmall"
                        color: Theme.fgSurfaceVariant
                        elide: Text.ElideRight
                    }
                }
                IconButton {
                    id: forget
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    icon: "delete"
                    size: "s"
                    onClicked: Catalog.removeSearchQuery(historyRow.key)
                }
            }
        }
    }

    // ---- Состояния ----
    EmptyState {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 48
        // the empty field shows Browse & Discover instead of the idle hint
        visible: !page.idle && (!page.signedIn || Catalog.searchState === "idle" || Catalog.searchState === "empty" || Catalog.searchState === "error")
        icon: !page.signedIn ? "login" : Catalog.searchState === "empty" ? "search_off"
            : Catalog.searchState === "error" ? "cloud_off" : "travel_explore"
        title: !page.signedIn ? I18n.t("Sign in to search")
             : Catalog.searchState === "empty" ? I18n.t("Nothing found")
             : Catalog.searchState === "error" ? I18n.t("Search didn't respond")
             : I18n.t("What do you want to hear?")
        text: !page.signedIn ? I18n.t("Search works through your Spotify account")
            : Catalog.searchState === "empty" ? I18n.t("Try different wording or another filter")
            : Catalog.searchState === "error" ? I18n.t("Check your connection and try again")
            : I18n.t("Search YouTube Music for songs, albums, artists and playlists. You can also paste a YouTube Music link")
        actionText: !page.signedIn ? I18n.t("Sign in") : Catalog.searchState === "error" ? I18n.t("Retry") : ""
        actionIcon: !page.signedIn ? "login" : "refresh"
        onAction: {
            if (!page.signedIn)
                page.router.reset("account")
            else
                Catalog.search(field.text, page.type)
        }
    }
    LoadingIndicator {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 48
        size: 64
        contained: true
        visible: !page.idle && page.signedIn && Catalog.searchState === "loading"
    }

    // ---- Результаты ----
    ColumnLayout {
        id: results
        Layout.fillWidth: true
        spacing: 24
        visible: !page.idle && page.signedIn && Catalog.searchState === "ok"

        // Лучший результат
        Item {
            id: bestCard
            readonly property var best: Catalog.best
            readonly property var item: best.item ?? ({})
            visible: page.all && best.type !== undefined
            Layout.fillWidth: true
            Layout.preferredHeight: 168

            Rectangle {
                anchors.fill: parent
                radius: Theme.shape.extraLarge
                color: Theme.surfaceContainerHigh
            }
            StateLayer {
                anchors.fill: parent
                radius: Theme.shape.extraLarge
                onClicked: page.openBest()
            }
            RowLayout {
                anchors.fill: parent
                anchors.margins: 20
                spacing: 24
                MorphImage {
                    Layout.preferredWidth: 128
                    Layout.preferredHeight: 128
                    source: bestCard.item.coverLarge || bestCard.item.cover || ""
                    shape: bestCard.best.type === "artist" ? "circle" : "softSquare"
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4
                    Label {
                        text: (({ track: I18n.t("Track"), album: bestCard.item.kind ? I18n.any(bestCard.item.kind) : I18n.t("Album"), artist: I18n.t("Artist"), playlist: I18n.t("Playlist"), show: I18n.t("Podcast") })[bestCard.best.type] ?? "").toUpperCase()
                        type: "labelLarge"
                        color: Theme.primary
                        font.letterSpacing: 1.2
                    }
                    Label {
                        Layout.fillWidth: true
                        text: bestCard.item.title ?? bestCard.item.name ?? ""
                        type: "headlineMedium"
                        weight: 700
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: text !== ""
                        text: bestCard.item.artists || bestCard.item.owner || bestCard.item.publisher
                              || page.followersText(bestCard.item.followers ?? 0)
                              || bestCard.item.genres || ""
                        type: "bodyLarge"
                        color: Theme.fgSurfaceVariant
                    }
                }
                PlayButton {
                    visible: bestCard.best.type === "track"
                    size: 64
                    playing: Player.playing && Player.trackId === (bestCard.item.trackId ?? "-")
                    onClicked: page.openBest()
                }
            }
        }

        // Треки
        SectionHeader {
            visible: Catalog.searchTracks.count > 0 && (page.all || page.type === "tracks")
            text: I18n.t("Tracks")
            actionText: page.all && Catalog.totals.tracks > 5 ? I18n.t("Show all") : ""
            onAction: page.showFilter("tracks")
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: -12
            visible: page.all || page.type === "tracks"
            spacing: 0
            Repeater {
                model: Catalog.searchTracks
                TrackRow {
                    required property int index
                    required property var model
                    visible: !page.all || index < 5
                    Layout.fillWidth: true
                    trackId: model.trackId
                    title: model.title
                    version: model.version
                    artists: model.artists
                    artistRefs: model.artistRefs
                    albumId: model.albumId
                    album: model.album
                    cover: model.cover
                    durationMs: model.durationMs
                    explicit: model.explicit
                    available: model.available
                    liked: Library.likesRevision >= 0 && Library.isLiked(model.trackId)
                    current: model.trackId === Player.trackId
                    onActivated: page.playResult(index)
                    onLikeClicked: Library.toggleLike(Catalog.searchTracks.get(index))
                    track: Catalog.searchTracks.get(index)
                }
            }
        }

        // Исполнители
        SectionHeader {
            visible: Catalog.searchArtists.count > 0 && (page.all || page.type === "artists")
            text: I18n.t("Artists")
            actionText: page.all && Catalog.totals.artists > page.perRow ? I18n.t("Show all") : ""
            onAction: page.showFilter("artists")
        }
        Flow {
            Layout.fillWidth: true
            visible: page.all || page.type === "artists"
            spacing: 20
            Repeater {
                model: Catalog.searchArtists
                MediaTile {
                    required property int index
                    required property var model
                    visible: !page.all || index < page.perRow
                    size: 148
                    round: true
                    placeholderIcon: "person"
                    cover: model.cover
                    title: model.name
                    onClicked: page.openArtistResult(model.artistId, model.name, model.cover)
                }
            }
        }

        // Альбомы
        SectionHeader {
            visible: Catalog.searchAlbums.count > 0 && (page.all || page.type === "albums")
            text: I18n.t("Albums")
            actionText: page.all && Catalog.totals.albums > page.perRow ? I18n.t("Show all") : ""
            onAction: page.showFilter("albums")
        }
        Flow {
            Layout.fillWidth: true
            visible: page.all || page.type === "albums"
            spacing: 20
            Repeater {
                model: Catalog.searchAlbums
                MediaTile {
                    required property int index
                    required property var model
                    visible: !page.all || index < page.perRow
                    cover: model.cover
                    title: model.title
                    subtitle: [model.artists, model.year || ""].filter(x => x).join(" · ")
                    explicit: model.explicit
                    onClicked: Catalog.openAlbum(model.albumId)
                }
            }
        }

        // Плейлисты
        SectionHeader {
            visible: Catalog.searchPlaylists.count > 0 && (page.all || page.type === "playlists")
            text: I18n.t("Playlists")
            actionText: page.all && Catalog.totals.playlists > page.perRow ? I18n.t("Show all") : ""
            onAction: page.showFilter("playlists")
        }
        Flow {
            Layout.fillWidth: true
            visible: page.all || page.type === "playlists"
            spacing: 20
            Repeater {
                model: Catalog.searchPlaylists
                MediaTile {
                    required property int index
                    required property var model
                    visible: !page.all || index < page.perRow
                    placeholderIcon: "queue_music"
                    cover: model.cover
                    title: model.title
                    subtitle: model.owner
                    onClicked: {
                        const p = Catalog.searchPlaylists.get(index)
                        Catalog.openPlaylist({ playlistId: p.playlistId, title: p.title, cover: p.cover })
                    }
                }
            }
        }

        // Подкасты
        SectionHeader {
            visible: page.showsModel !== null && page.showsModel.count > 0 && (page.all || page.type === "shows")
            text: I18n.t("Podcasts")
            actionText: page.all && (Catalog.totals.shows ?? 0) > page.perRow ? I18n.t("Show all") : ""
            onAction: page.showFilter("shows")
        }
        Flow {
            Layout.fillWidth: true
            visible: page.all || page.type === "shows"
            spacing: 20
            Repeater {
                model: page.showsModel
                MediaTile {
                    required property int index
                    required property var model
                    visible: !page.all || index < page.perRow
                    placeholderIcon: "podcasts"
                    cover: model.cover
                    title: model.name
                    subtitle: model.publisher
                    explicit: model.explicit
                    onClicked: Catalog.openShow(model.showId)
                }
            }
        }

        // Выпуски
        SectionHeader {
            visible: page.episodesModel !== null && page.episodesModel.count > 0 && (page.all || page.type === "episodes")
            text: I18n.t("Episodes")
            actionText: page.all && (Catalog.totals.episodes ?? 0) > 3 ? I18n.t("Show all") : ""
            onAction: page.showFilter("episodes")
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: -12
            visible: page.all || page.type === "episodes"
            spacing: 0
            Repeater {
                model: page.episodesModel
                EpisodeRow {
                    required property int index
                    visible: !page.all || index < 3
                    Layout.fillWidth: true
                    compact: true
                    episode: page.episodesModel.get(index)
                }
            }
        }
    }

    function openBest() {
        const b = Catalog.best
        if (!b.type)
            return
        if (b.type === "artist") page.openArtistResult(b.item.artistId, b.item.name ?? b.item.title ?? "", b.item.cover ?? "")
        else if (b.type === "album") Catalog.openAlbum(b.item.albumId)
        else if (b.type === "playlist") Catalog.openPlaylist({ playlistId: b.item.playlistId, title: b.item.title, cover: b.item.cover })
        else if (b.type === "show") Catalog.openShow(b.item.showId)
        else if (b.type === "track") {
            const row = Catalog.searchTracks.find("trackId", b.item.trackId)
            if (row >= 0) page.playResult(row)
        }
    }

    // an artist opened from the results goes to the recent searches in place of the text that found them
    function openArtistResult(artistId, name, cover) {
        Catalog.openArtist(artistId)
        if ((artistId ?? "") !== "")
            Catalog.addSearchArtist(field.text, artistId, name ?? "", cover ?? "")
    }

    // a song played from the results goes to the recent searches in place of the text that found it
    function playResult(row) {
        Player.playRadioFrom(Catalog.searchTracks, row)
        const t = Catalog.searchTracks.get(row)
        if (t && (t.uri ?? "") !== "")
            Catalog.addSearchTrack(field.text, t)
    }
}
