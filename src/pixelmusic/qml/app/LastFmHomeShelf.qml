import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Home shelf "From your Last.fm" (setting lastfmExplore, Last.fm connected): the Last.fm feed of SmartMix —
// recently scrobbled tracks (play → found on YouTube Music), top albums and friends — as horizontal strips.
// The feed is read once when the shelf appears; refresh reads it again.
ColumnLayout {
    id: root
    property real sectionPad: 20
    spacing: 12

    Component.onCompleted: {
        if ((SmartMix.recent ?? []).length === 0 && !SmartMix.feedLoading)
            SmartMix.refreshFeed()
    }

    // Last.fm art, else a cover found on YouTube Music (asked for once per tile; key = lastfm_feed::cover_key)
    function coverOf(kind, item) {
        const key = (kind + "|" + (item.artist ?? "").trim() + "|" + (item.title ?? "").trim()).toLowerCase()
        return (item.cover ?? "") !== "" ? item.cover : (SmartMix.covers[key] ?? "")
    }
    function askCover(kind, item) {
        if ((item.cover ?? "") === "" && (item.title ?? "") !== "")
            SmartMix.requestCover(kind, item.title, item.artist ?? "")
    }

    component Strip: ColumnLayout {
        id: st
        property string title
        property var items: []
        property Component tile
        property real rowHeight: 220
        visible: items.length > 0
        Layout.fillWidth: true
        spacing: 8
        Label { text: st.title; type: "titleMedium"; weight: 600 }
        ListView {
            Layout.fillWidth: true
            Layout.leftMargin: -root.sectionPad
            Layout.rightMargin: -root.sectionPad
            Layout.preferredHeight: st.rowHeight
            orientation: ListView.Horizontal
            spacing: 24
            leftMargin: root.sectionPad
            rightMargin: root.sectionPad
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: st.items
            delegate: Loader {
                required property var modelData
                sourceComponent: st.tile
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: 8
        spacing: 8
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            Label {
                Layout.fillWidth: true
                text: I18n.t("RECOMMENDED WITH LAST.FM")
                type: "labelMedium"
                weight: 600
                color: Theme.fgSurfaceVariant
            }
            Label { Layout.fillWidth: true; text: I18n.t("From your Last.fm"); type: "headlineSmall"; weight: 600 }
        }
        LoadingIndicator { visible: SmartMix.feedLoading; size: 32 }
        IconButton {
            icon: "refresh"
            size: "xs"
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

    Strip {
        title: I18n.t("Recently scrobbled")
        items: (SmartMix.recent ?? []).slice(0, 15)
        tile: MediaTile {
            readonly property var e: parent ? parent.modelData : ({})
            size: 168
            cover: root.coverOf("track", e)
            title: e.title ?? ""
            subtitle: [e.artist ?? "", e.dateText ?? ""].filter(x => x).join(" · ")
            placeholderIcon: "music_note"
            showPlay: true
            Component.onCompleted: root.askCover("track", e)
            onClicked: SmartMix.playTrack(e.title, e.artist, "youtube")
            onPlayClicked: SmartMix.playTrack(e.title, e.artist, "youtube")
        }
    }
    Strip {
        title: I18n.t("Your top albums")
        items: (SmartMix.topAlbums ?? []).slice(0, 15)
        tile: MediaTile {
            readonly property var e: parent ? parent.modelData : ({})
            size: 168
            cover: root.coverOf("album", e)
            title: e.title ?? ""
            subtitle: [e.artist ?? "", e.playCount > 0 ? I18n.t("{0} plays", e.playCount) : ""].filter(x => x).join(" · ")
            Component.onCompleted: root.askCover("album", e)
        }
    }
    Strip {
        title: I18n.t("Last.fm friends")
        items: SmartMix.friends ?? []
        rowHeight: 180
        tile: MediaTile {
            readonly property var e: parent ? parent.modelData : ({})
            size: 120
            round: true
            cover: e.avatar ?? ""
            title: e.displayName ?? ""
            subtitle: (e.realName ?? "") !== "" ? (e.name ?? "") : ""
            placeholderIcon: "person"
            onClicked: Qt.openUrlExternally("https://www.last.fm/user/" + encodeURIComponent(e.name ?? ""))
        }
    }
}
