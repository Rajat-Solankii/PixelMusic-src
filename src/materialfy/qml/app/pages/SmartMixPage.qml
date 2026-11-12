import QtQuick
import QtQuick.Layouts
import Md3
import Materialfy.Core
import ".."

// Smart Mix (port of soundwave1 SmartMixScreen): a playlist built from Last.fm — top, recent or similar tracks,
// a tag, a weighted blend or scored recommendations — found on YouTube Music (SmartMix.tracks).
// Under the list: the Last.fm feed (recent scrobbles, top albums, friends). The account is the Settings card's.
TrackListPage {
    id: page

    property string mode: "mix"
    property string period: "overall"
    property int count: 25
    readonly property string target: "youtube"   // songs are found on YouTube Music only
    property string seedTrack
    property string seedArtist
    property string seedArtistInput
    property string tag

    readonly property var modes: [
        { id: "top", icon: "equalizer", text: I18n.t("Top tracks"), note: I18n.t("Your most played tracks") },
        { id: "recent", icon: "history", text: I18n.t("Recent tracks"), note: I18n.t("What you've been listening to lately") },
        { id: "similar-tracks", icon: "music_note", text: I18n.t("Similar tracks"), note: I18n.t("Tracks similar to one you love") },
        { id: "similar-artists", icon: "group", text: I18n.t("Similar artists"), note: I18n.t("Discover artists like your favourites") },
        { id: "tag", icon: "label", text: I18n.t("By tag or genre"), note: I18n.t("Browse by genre like rock, lofi, jazz") },
        { id: "mix", icon: "shuffle", text: I18n.t("My mix"), note: I18n.t("A blend of top, recent and similar tracks") },
        { id: "recommendations", icon: "star", text: I18n.t("Recommendations"), note: I18n.t("Fresh tracks picked for you") },
        { id: "library", icon: "library_music", text: I18n.t("My library"), note: I18n.t("Rediscover the sounds of your past") }
    ]
    readonly property var periods: [
        { id: "overall", text: I18n.t("All time") },
        { id: "12month", text: I18n.t("12 months") },
        { id: "6month", text: I18n.t("6 months") },
        { id: "3month", text: I18n.t("3 months") },
        { id: "1month", text: I18n.t("1 month") },
        { id: "7day", text: I18n.t("7 days") }
    ]
    readonly property var modeInfo: modes.find(m => m.id === mode) ?? modes[0]
    readonly property bool ready: mode === "similar-tracks" ? seedTrack.trim() !== "" && seedArtist.trim() !== ""
                                : mode === "similar-artists" ? seedArtistInput.trim() !== ""
                                : mode === "tag" ? tag.trim() !== "" : true

    Component.onCompleted: if (LastFm.connected) SmartMix.refreshFeed()
    Connections {
        target: LastFm
        function onConnectedChanged() { if (LastFm.connected) SmartMix.refreshFeed() }
    }

    function generate() {
        SmartMix.generate(mode, count, period, seedTrack, seedArtist, seedArtistInput, tag, target)
    }
    // Last.fm art, else a cover found on YouTube Music (asked for once, when the tile is created).
    // The key matches sf_services::lastfm_feed::cover_key.
    function coverKey(kind, title, artist) {
        return (kind + "|" + (artist ?? "").trim() + "|" + (title ?? "").trim()).toLowerCase()
    }
    function coverOf(kind, item) {
        return (item.cover ?? "") !== "" ? item.cover : (SmartMix.covers[coverKey(kind, item.title, item.artist)] ?? "")
    }
    function askCover(kind, item) {
        if ((item.cover ?? "") === "" && (item.title ?? "") !== "")
            SmartMix.requestCover(kind, item.title, item.artist ?? "")
    }

    function notify(text) {
        const w = page.Window.window
        if (w && typeof w.notify === "function")
            w.notify(text, "", null)
    }

    title: SmartMix.mixTitle !== "" ? SmartMix.mixTitle : I18n.t("Smart Mix")
    overline: I18n.t("SMART MIX")
    heroImage: SmartMix.mixCover
    heroIcon: "star"
    heroShape: "flower8"
    subtitle: SmartMix.generating ? SmartMix.progress
            : SmartMix.trackCount > 0 ? I18n.t("{0} tracks · made from your Last.fm", SmartMix.trackCount)
            : LastFm.connected ? I18n.t("Pick a mode and make a mix") : ""
    model: SmartMix.tracks
    emptyIcon: "star"
    emptyTitle: !LastFm.connected ? I18n.t("Connect Last.fm first")
              : SmartMix.generating ? I18n.t("Making your mix…") : I18n.t("No mix yet")
    emptyText: !LastFm.connected ? I18n.t("Smart Mix uses your Last.fm listening history: connect your account in Settings")
             : SmartMix.mixError

    headerExtra: Component {
        Row {
            spacing: 4
            Button {
                text: SmartMix.generating ? I18n.t("Cancel") : SmartMix.trackCount > 0 ? I18n.t("Regenerate") : I18n.t("Generate")
                icon: SmartMix.generating ? "close" : "star"
                style: SmartMix.trackCount > 0 ? "tonal" : "filled"
                enabled: LastFm.connected && (SmartMix.generating || page.ready)
                onClicked: SmartMix.generating ? SmartMix.cancel() : page.generate()
            }
            IconButton {
                icon: "bookmark_add"
                size: "m"
                enabled: !SmartMix.generating && SmartMix.trackCount > 0
                onClicked: {
                    SmartMix.save()
                    page.notify(I18n.t("Saving the mix as a playlist…"))
                }
            }
        }
    }

    heroNote: Component {
        ColumnLayout {
            spacing: 12
            enabled: LastFm.connected && !SmartMix.generating

            Flow {
                Layout.fillWidth: true
                spacing: 8
                Repeater {
                    model: page.modes
                    Chip {
                        required property var modelData
                        text: modelData.text
                        icon: modelData.icon
                        selected: page.mode === modelData.id
                        onClicked: page.mode = modelData.id
                    }
                }
            }
            Label {
                Layout.fillWidth: true
                text: page.modeInfo.note
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
            }
            Flow {
                Layout.fillWidth: true
                spacing: 12
                MenuField {
                    visible: page.mode === "top"
                    width: 180
                    leadingIcon: "schedule"
                    label: I18n.t("Period")
                    model: page.periods
                    currentValue: page.period
                    onActivated: v => page.period = v
                }
                MenuField {
                    width: 160
                    leadingIcon: "format_list_bulleted"
                    label: I18n.t("Tracks")
                    model: [10, 25, 50, 100].map(n => ({ id: n, text: String(n) }))
                    currentValue: page.count
                    onActivated: v => page.count = v
                }
                TextField {
                    visible: page.mode === "similar-tracks"
                    width: 220
                    label: I18n.t("Seed track")
                    leadingIcon: "music_note"
                    text: page.seedTrack
                    onEdited: page.seedTrack = text
                    onAccepted: if (page.ready) page.generate()
                }
                TextField {
                    visible: page.mode === "similar-tracks"
                    width: 220
                    label: I18n.t("Artist")
                    leadingIcon: "person"
                    text: page.seedArtist
                    onEdited: page.seedArtist = text
                    onAccepted: if (page.ready) page.generate()
                }
                TextField {
                    visible: page.mode === "similar-artists"
                    width: 220
                    label: I18n.t("Seed artist")
                    leadingIcon: "person"
                    text: page.seedArtistInput
                    onEdited: page.seedArtistInput = text
                    onAccepted: if (page.ready) page.generate()
                }
                TextField {
                    visible: page.mode === "tag"
                    width: 220
                    label: I18n.t("Genre or tag")
                    leadingIcon: "label"
                    text: page.tag
                    onEdited: page.tag = text
                    onAccepted: if (page.ready) page.generate()
                }
            }
            Label {
                Layout.fillWidth: true
                visible: SmartMix.mixError !== "" && SmartMix.trackCount > 0
                text: SmartMix.mixError
                type: "bodyMedium"
                color: Theme.error
                wrapMode: Text.WordWrap
            }
        }
    }

    // the Last.fm feed under the mix
    footerExtra: LastFm.connected ? feed : null
    Component {
        id: feed
        ColumnLayout {
            width: parent ? parent.width : 0
            spacing: 16

            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 24
                Label {
                    Layout.fillWidth: true
                    text: I18n.t("From your Last.fm")
                    type: "titleLarge"
                }
                IconButton {
                    icon: "refresh"
                    enabled: !SmartMix.feedLoading
                    onClicked: SmartMix.refreshFeed()
                }
            }
            Label {
                Layout.fillWidth: true
                visible: SmartMix.feedError !== ""
                text: SmartMix.feedError
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
            }

            Label {
                visible: (SmartMix.recent ?? []).length > 0
                text: I18n.t("Recently scrobbled")
                type: "titleMedium"
            }
            Flow {
                Layout.fillWidth: true
                spacing: 20
                Repeater {
                    model: (SmartMix.recent ?? []).slice(0, 12)
                    MediaTile {
                        required property var modelData
                        cover: page.coverOf("track", modelData)
                        title: modelData.title ?? ""
                        subtitle: [modelData.artist ?? "", modelData.dateText ?? ""].filter(x => x).join(" · ")
                        Component.onCompleted: page.askCover("track", modelData)
                        placeholderIcon: "music_note"
                        showPlay: true
                        onClicked: SmartMix.playTrack(modelData.title, modelData.artist, page.target)
                        onPlayClicked: SmartMix.playTrack(modelData.title, modelData.artist, page.target)
                    }
                }
            }

            Label {
                visible: (SmartMix.topAlbums ?? []).length > 0
                text: I18n.t("Your top albums")
                type: "titleMedium"
            }
            Flow {
                Layout.fillWidth: true
                spacing: 20
                Repeater {
                    model: (SmartMix.topAlbums ?? []).slice(0, 12)
                    MediaTile {
                        required property var modelData
                        cover: page.coverOf("album", modelData)
                        Component.onCompleted: page.askCover("album", modelData)
                        title: modelData.title ?? ""
                        subtitle: [modelData.artist ?? "", modelData.playCount > 0 ? I18n.t("{0} plays", modelData.playCount) : ""].filter(x => x).join(" · ")
                    }
                }
            }

            Label {
                visible: (SmartMix.friends ?? []).length > 0
                text: I18n.t("Last.fm friends")
                type: "titleMedium"
            }
            Flow {
                Layout.fillWidth: true
                spacing: 20
                Repeater {
                    model: SmartMix.friends ?? []
                    MediaTile {
                        required property var modelData
                        size: 120
                        round: true
                        cover: modelData.avatar ?? ""
                        title: modelData.displayName ?? ""
                        subtitle: modelData.realName !== "" ? modelData.name : ""
                        placeholderIcon: "person"
                    }
                }
            }
        }
    }
}
