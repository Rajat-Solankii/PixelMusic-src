import QtQuick
import QtQuick.Layouts
import Md3
import Materialfy.Core

// Диалог «Импорт плейлиста» (как NewPlaylistDialog): ссылка на плейлист YouTube / YouTube Music или Spotify →
// Library.importPlaylist(text, save). Песни ищутся в YouTube Music; save — сохранить найденное в новый
// приватный плейлист YouTube Music (нужен вход). Ход и итог — свойства Library.import*.
Item {
    id: root

    property bool opened: false
    // нажали «Импорт» в этом открытии: до этого прошлый итог не показывается
    property bool started: false
    readonly property bool running: Library.importRunning
    readonly property bool finished: started && !running
    readonly property bool canSave: Auth.youtubeSignedIn

    function open() {
        field.text = ""
        started = false
        saveSwitch.checked = canSave
        opened = true
        field.inputItem.forceActiveFocus()
    }
    function close() {
        if (running)
            Library.cancelImport()
        opened = false
    }
    function submit() {
        const text = field.text.trim()
        if (text === "" || running)
            return
        started = true
        Library.importPlaylist(text, saveSwitch.checked && canSave)
    }
    function statusText() {
        if (running)
            return Library.importTotal > 0 ? I18n.t("Matching songs: {0} of {1}", Library.importDone, Library.importTotal)
                                           : I18n.t("Reading the playlist…")
        if (Library.importError !== "")
            return Library.importError
        return I18n.t("Found {0} of {1} songs", Library.importTracks ? Library.importTracks.count : 0, Library.importTotal)
    }

    anchors.fill: parent
    visible: opened || scrim.opacity > 0

    Rectangle {
        id: scrim
        anchors.fill: parent
        color: Qt.alpha(Theme.scrim, 0.32)
        opacity: root.opened ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onPressed: root.close()
            onWheel: wheel => {}
        }
    }

    Rectangle {
        id: card
        anchors.centerIn: parent
        width: Math.min(440, parent.width - 48)
        implicitHeight: content.implicitHeight + 48
        height: implicitHeight
        radius: Theme.shape.extraLarge
        color: Theme.surfaceContainerHigh
        scale: root.opened ? 1 : 0.92
        opacity: root.opened ? 1 : 0
        Behavior on scale { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack } }
        Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
        Keys.onEscapePressed: root.close()

        Elevation { target: card; level: 3 }

        // клики по карточке не закрывают диалог
        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }

        ColumnLayout {
            id: content
            anchors.fill: parent
            anchors.margins: 24
            spacing: 16

            Label { text: I18n.t("Import playlist"); type: "headlineSmall" }

            Label {
                Layout.fillWidth: true
                text: I18n.t("Paste a YouTube Music, YouTube or Spotify playlist link. Its songs are found on YouTube Music.")
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
            }

            SearchField {
                id: field
                Layout.fillWidth: true
                enabled: !root.running
                placeholder: I18n.t("Playlist link")
                leadingIcon: "link"
                onAccepted: root.submit()
                Keys.onEscapePressed: root.close()
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                opacity: root.canSave ? 1 : Theme.stateLayer.disabledContent
                Label {
                    Layout.fillWidth: true
                    text: root.canSave ? I18n.t("Save as a YouTube Music playlist")
                                       : I18n.t("Sign in to YouTube Music to save the result as a playlist")
                    type: "bodyMedium"
                    wrapMode: Text.WordWrap
                }
                Switch {
                    id: saveSwitch
                    enabled: root.canSave && !root.running
                    onToggled: c => checked = c
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                visible: root.started
                spacing: 8
                WavyProgress {
                    Layout.fillWidth: true
                    visible: root.running
                    value: Library.importTotal > 0 ? Library.importDone / Library.importTotal : 0
                }
                Label {
                    Layout.fillWidth: true
                    text: root.statusText()
                    type: "bodyMedium"
                    color: root.finished && Library.importError !== "" ? Theme.error : Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                }
            }

            RowLayout {
                Layout.alignment: Qt.AlignRight
                spacing: 8
                Button {
                    text: root.finished ? I18n.t("Close") : I18n.t("Cancel")
                    style: "text"
                    onClicked: root.close()
                }
                Button {
                    visible: root.finished && Library.importPlaylistUri !== ""
                    text: I18n.t("Open")
                    onClicked: {
                        Catalog.openUri(Library.importPlaylistUri)
                        root.close()
                    }
                }
                Button {
                    visible: root.finished && Library.importPlaylistUri === "" && Library.importTracks
                             && Library.importTracks.count > 0
                    text: I18n.t("Play")
                    onClicked: {
                        Player.playFrom(Library.importTracks, 0)
                        root.close()
                    }
                }
                Button {
                    visible: !root.finished
                    text: I18n.t("Import")
                    enabled: field.text.trim() !== "" && !root.running
                    onClicked: root.submit()
                }
            }
        }
    }
}
