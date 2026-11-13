import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Правая выдвижная панель: очередь / текст песни.
Surface {
    id: root

    // Вкладки (Settings.rightPanelTab). Подпись — только у выбранной: три подписи в 340 px не помещаются.
    readonly property var tabs: [
        { key: "queue", text: I18n.t("Queue"), icon: "queue_music" },
        { key: "lyrics", text: I18n.t("Song lyrics"), icon: "title" }
    ]
    readonly property int tabIndex: Math.max(0, tabs.findIndex(t => t.key === Settings.rightPanelTab))
    readonly property string tab: tabs[tabIndex].key

    level: "container"
    translucency: Settings.ambientBackground && Player.cover !== "" ? 0.3 : 0

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        anchors.bottomMargin: 0
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            ButtonGroup {
                model: root.tabs.map((t, i) => ({ icon: t.icon, text: i === root.tabIndex ? t.text : "" }))
                autoSelect: false
                currentIndex: root.tabIndex
                onActivated: i => Settings.rightPanelTab = root.tabs[i].key
            }
            Item { Layout.fillWidth: true }
            LyricsTools { visible: root.tab === "lyrics" && Player.hasTrack }
            IconButton { icon: "close"; onClicked: Settings.rightPanelOpen = false }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            QueueView {
                anchors.fill: parent
                visible: root.tab === "queue"
            }
            LyricsView {
                anchors.fill: parent
                anchors.bottomMargin: 16
                visible: root.tab === "lyrics" && Settings.rightPanelOpen
                baseFontSize: 22
                anchorPos: 0.3
            }
        }
    }
}
