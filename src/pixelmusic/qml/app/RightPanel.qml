import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Правая выдвижная панель: очередь / текст песни.
// The queue is PixelPlayer Desktop's side panel: its own "Next Up" header (QueueView) with 14 px margins and no
// tab row; the player bar's queue and lyrics buttons switch between the two. Lyrics keep a small header.
Surface {
    id: root

    readonly property string tab: Settings.rightPanelTab === "lyrics" ? "lyrics" : "queue"

    level: "container"
    translucency: Settings.ambientBackground && Player.cover !== "" ? 0.3 : 0

    QueueView {
        anchors.fill: parent
        anchors.leftMargin: 14
        anchors.rightMargin: 14
        anchors.topMargin: 10
        visible: root.tab === "queue"
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        anchors.bottomMargin: 0
        spacing: 12
        visible: root.tab === "lyrics"

        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Rectangle {
                Layout.preferredWidth: 34
                Layout.preferredHeight: 34
                radius: 17
                color: Qt.alpha(Theme.primaryContainer, 0.8)
                Icon { anchors.centerIn: parent; name: "lyrics"; size: 18; color: Theme.fgPrimaryContainer }
            }
            Label {
                Layout.fillWidth: true
                text: I18n.t("Song lyrics")
                type: "titleMedium"
                weight: 700
            }
            LyricsTools { visible: Player.hasTrack }
            IconButton { icon: "close"; onClicked: Settings.rightPanelOpen = false }
        }

        LyricsView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.bottomMargin: 16
            visible: Settings.rightPanelOpen
            baseFontSize: 22
            anchorPos: 0.3
        }
    }
}
