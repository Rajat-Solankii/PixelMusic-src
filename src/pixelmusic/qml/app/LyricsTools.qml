import QtQuick
import QtQuick.Layouts
import QtQuick.Dialogs
import QtQuick.Templates as T
import Md3
import PixelMusic.Core

// Lyrics toolbar (PixelPlayer): a search button opens the Search Lyrics dialog (LRCLIB by the user's query, results
// with a SYNCED badge, Import of an .lrc/.txt file); "more" opens the Lyrics options card (text alignment and size,
// sync offset, romanization, save as .lrc, reset to what the sources find). Both are popups over the whole window.
Row {
    id: root
    spacing: 4

    function say(text) {
        const w = root.Window.window
        if (w && typeof w.notify === "function")
            w.notify(text, "", null)
    }
    function openSearch() {
        options.close()
        const query = [Player.artist, Player.title].filter(s => (s ?? "") !== "").join(" ")
        search.open()
        field.text = query
        field.inputItem.forceActiveFocus()
        if (query !== "")
            Lyrics.search(query)
    }

    function openOptions() { options.open() }

    readonly property var sizes: [0.85, 1.0, 1.25]
    readonly property int sizeIndex: {
        let best = 0
        for (let i = 1; i < sizes.length; i++)
            if (Math.abs(sizes[i] - Settings.lyricsFontScale) < Math.abs(sizes[best] - Settings.lyricsFontScale))
                best = i
        return best
    }

    // Material 3 Expressive icon buttons: a wide tonal pill for search, a narrow toggle for ⋮ that morphs to a
    // rounded square in the primary colour while the options card is open
    IconButton {
        icon: "search"
        style: "tonal"
        widthMode: "wide"
        iconWeight: 500
        enabled: Player.hasTrack && !Player.isEpisode
        onClicked: root.openSearch()
    }
    IconButton {
        id: moreButton
        icon: "more_vert"
        style: "filled"
        widthMode: "narrow"
        iconWeight: 600
        checkable: true
        autoToggle: false
        checked: options.visible
        // the press that just closed the card (press outside) does not open it again: it toggles
        property bool swallowClick: false
        onPressedChanged: if (pressed) swallowClick = !options.opened && Date.now() - options.closedAt < 80
        onClicked: {
            if (swallowClick)
                swallowClick = false
            else if (options.opened)
                options.close()
            else
                options.open()
        }
    }

    // ── Lyrics options ─────────────────────────────────────────────────────
    // A solid Material 3 Expressive card: an opaque surface with elevation, segmented groups across its width and
    // the options as groups of pill rows (large outer, small inner corners, 2 px apart).
    T.Popup {
        id: options
        parent: T.Overlay.overlay
        width: 360
        // Templates' Popup has no implicit size of its own: without this the popup is 0 px high and the card
        // (background) is not drawn — the rows hung over the lyrics with nothing behind them
        implicitHeight: contentItem.implicitHeight + topPadding + bottomPadding
        padding: 16
        topPadding: 20
        focus: true
        closePolicy: T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutside
        property bool syncOpen: false
        // a press on ⋮ while the card is open closes it (press outside) and then reaches the button in the same
        // dispatch; the click of that press must not open the card again (see moreButton)
        property real closedAt: 0
        onAboutToHide: closedAt = Date.now()
        onAboutToShow: {
            syncOpen = false
            const o = parent
            const p = moreButton.mapToItem(o, 0, 0)
            x = Math.max(8, Math.min(p.x + moreButton.width - width, o.width - width - 8))
            const below = p.y + moreButton.height + 6
            y = Math.max(8, Math.min(below, o.height - implicitHeight - 8))
        }
        enter: Transition {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.motion.effectsFast }
            NumberAnimation { property: "scale"; from: 0.94; to: 1; duration: Theme.motion.spatialFast; easing.type: Easing.OutCubic }
        }
        exit: Transition { NumberAnimation { property: "opacity"; to: 0; duration: Theme.motion.effectsFast } }
        transformOrigin: T.Popup.TopRight

        background: Rectangle {
            id: optionsCard
            radius: Theme.shape.extraLarge
            color: Qt.alpha(Theme.surfaceContainer, 1)
            Elevation { target: optionsCard; level: 3 }
            // the card is solid: wheel and clicks on it never reach the lyrics underneath
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onWheel: wheel => wheel.accepted = true }
        }

        contentItem: ColumnLayout {
            spacing: 8

            RowLayout {
                Layout.leftMargin: 4
                Layout.bottomMargin: 4
                spacing: 10
                Icon { name: "lyrics"; size: 22; fill: 1; color: Theme.primary }
                Label { Layout.fillWidth: true; text: I18n.t("Lyrics options"); type: "titleMedium"; weight: 600 }
            }

            Label {
                Layout.leftMargin: 4
                text: I18n.t("Text alignment")
                type: "labelLarge"
                color: Theme.fgSurfaceVariant
            }
            ButtonGroup {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                fill: true
                model: [{ icon: "format_align_left", text: I18n.t("Left") },
                        { icon: "format_align_center", text: I18n.t("Center") },
                        { icon: "format_align_right", text: I18n.t("Right") }]
                currentIndex: Math.max(0, ["left", "center", "right"].indexOf(Settings.lyricsTextAlign))
                onActivated: i => Settings.lyricsTextAlign = ["left", "center", "right"][i]
            }

            Label {
                Layout.leftMargin: 4
                Layout.topMargin: 4
                text: I18n.t("Text size")
                type: "labelLarge"
                color: Theme.fgSurfaceVariant
            }
            ButtonGroup {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                fill: true
                model: [I18n.t("Small"), I18n.t("Medium"), I18n.t("Large")]
                currentIndex: root.sizeIndex
                onActivated: i => Settings.lyricsFontScale = root.sizes[i]
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.topMargin: 8
                spacing: 2

                OptionRow {
                    first: true
                    icon: "tune"
                    title: I18n.t("Adjust lyrics sync")
                    subtitle: Settings.lyricsSyncMs === 0 ? I18n.t("Calibrate the sync offset")
                            : I18n.t("{0} ms", (Settings.lyricsSyncMs > 0 ? "+" : "") + Settings.lyricsSyncMs)
                    expandable: true
                    expanded: options.syncOpen
                    onClicked: options.syncOpen = !options.syncOpen
                }
                // the offset: + shows lyrics earlier, − later
                Rectangle {
                    Layout.fillWidth: true
                    visible: options.syncOpen
                    implicitHeight: syncBox.implicitHeight + 20
                    radius: Theme.shape.extraSmall
                    color: Theme.surfaceContainerHigh
                    ColumnLayout {
                        id: syncBox
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.leftMargin: 12
                        anchors.rightMargin: 12
                        spacing: 6
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 4
                            IconButton {
                                icon: "remove"
                                style: "tonal"
                                onClicked: Settings.lyricsSyncMs = Math.max(-3000, Settings.lyricsSyncMs - 100)
                            }
                            Slider {
                                Layout.fillWidth: true
                                wheelEnabled: false
                                from: -3000
                                to: 3000
                                stepSize: 50
                                value: Settings.lyricsSyncMs
                                valueText: v => (v > 0 ? "+" : "") + Math.round(v) + " ms"
                                onMoved: v => Settings.lyricsSyncMs = Math.round(v / 50) * 50
                            }
                            IconButton {
                                icon: "add"
                                style: "tonal"
                                onClicked: Settings.lyricsSyncMs = Math.min(3000, Settings.lyricsSyncMs + 100)
                            }
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            Label {
                                Layout.fillWidth: true
                                text: I18n.t("+ shows lyrics earlier, − later")
                                type: "bodySmall"
                                color: Theme.fgSurfaceVariant
                                wrapMode: Text.WordWrap
                            }
                            Button {
                                text: I18n.t("Reset")
                                style: "text"
                                enabled: Settings.lyricsSyncMs !== 0
                                onClicked: Settings.lyricsSyncMs = 0
                            }
                        }
                    }
                }

                OptionRow {
                    icon: "translate"
                    title: I18n.t("Show romanization")
                    subtitle: I18n.t("Hindi, Punjabi, Korean, Japanese kana, Cyrillic, Greek")
                    onClicked: Settings.lyricsRomanize = !Settings.lyricsRomanize
                    Switch {
                        checked: Settings.lyricsRomanize
                        onToggled: c => Settings.lyricsRomanize = c
                    }
                }
                OptionRow {
                    last: true
                    icon: "save"
                    title: I18n.t("Save lyrics…")
                    subtitle: I18n.t("Export as an .lrc file")
                    enabled: Lyrics.status === "ok" && Lyrics.trackId === Player.trackId
                    onClicked: {
                        options.close()
                        saveDialog.currentFile = "file:///" + ((Player.artist ? Player.artist + " - " : "") + Player.title)
                            .replace(/[\\/:*?"<>|]/g, "_") + ".lrc"
                        saveDialog.open()
                    }
                }
            }

            OptionRow {
                Layout.topMargin: 6
                first: true
                last: true
                icon: "restart_alt"
                title: I18n.t("Reset lyrics")
                subtitle: I18n.t("Forget the chosen lyrics and ask the sources again")
                danger: true
                enabled: Player.hasTrack && !Player.isEpisode
                onClicked: {
                    options.close()
                    Lyrics.retry()
                }
            }
        }
    }

    // One option of the card: a solid pill row of a group (`first`/`last` round the group's outer corners).
    component OptionRow: Rectangle {
        id: row
        property string icon
        property string title
        property string subtitle
        property bool danger: false
        property bool first: false
        property bool last: false
        property bool expandable: false
        property bool expanded: false
        default property alias trailing: trailingBox.data
        signal clicked()
        Layout.fillWidth: true
        implicitHeight: Math.max(64, texts.implicitHeight + 24)
        readonly property real _outer: Theme.shape.extraLarge
        readonly property real _inner: Theme.shape.extraSmall
        topLeftRadius: first ? _outer : _inner
        topRightRadius: first ? _outer : _inner
        bottomLeftRadius: last ? _outer : _inner
        bottomRightRadius: last ? _outer : _inner
        readonly property color _fill: danger ? Theme.errorContainer : Theme.surfaceContainerHighest
        readonly property color _ink: danger ? Theme.fgErrorContainer : Theme.fgSurface
        color: area.containsMouse && enabled ? Qt.tint(_fill, Qt.alpha(_ink, Theme.stateLayer.hover)) : _fill
        Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsFast } }
        opacity: enabled ? 1 : 0.45
        MouseArea {
            id: area
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: row.clicked()
        }
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 12
            anchors.rightMargin: 16
            spacing: 14
            Rectangle {
                Layout.preferredWidth: 40
                Layout.preferredHeight: 40
                radius: 20
                color: row.danger ? Theme.error : Theme.secondaryContainer
                Icon {
                    anchors.centerIn: parent
                    name: row.icon
                    size: 20
                    fill: 1
                    color: row.danger ? Theme.fgError : Theme.fgSecondaryContainer
                }
            }
            ColumnLayout {
                id: texts
                Layout.fillWidth: true
                spacing: 0
                Label {
                    Layout.fillWidth: true
                    text: row.title
                    type: "titleSmall"
                    weight: 600
                    color: row._ink
                }
                Label {
                    Layout.fillWidth: true
                    text: row.subtitle
                    type: "bodySmall"
                    color: row.danger ? Theme.fgErrorContainer : Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                }
            }
            Item {
                id: trailingBox
                implicitWidth: childrenRect.width
                implicitHeight: childrenRect.height
                visible: children.length > 0
            }
            Icon {
                visible: row.expandable
                name: "keyboard_arrow_down"
                size: 22
                color: Theme.fgSurfaceVariant
                rotation: row.expanded ? 180 : 0
                Behavior on rotation { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutCubic } }
            }
        }
    }

    // ── Search Lyrics ──────────────────────────────────────────────────────
    T.Popup {
        id: search
        parent: T.Overlay.overlay
        x: 0
        y: 0
        width: parent ? parent.width : 0
        height: parent ? parent.height : 0
        padding: 0
        modal: true
        dim: false
        focus: true
        closePolicy: T.Popup.CloseOnEscape
        background: null
        property string error: ""
        onAboutToShow: error = ""
        enter: Transition { NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.motion.effectsFast } }
        exit: Transition { NumberAnimation { property: "opacity"; to: 0; duration: Theme.motion.effectsFast } }

        function pick(index) {
            const err = Lyrics.applyResult(index)
            if (err !== "") {
                error = err
                return
            }
            close()
            root.say(I18n.t("Lyrics updated"))
        }

        contentItem: Item {
            Rectangle {
                anchors.fill: parent
                color: Theme.scrim
                opacity: 0.32
                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.AllButtons
                    onClicked: search.close()
                    onWheel: wheel => wheel.accepted = true
                }
            }

            Rectangle {
                id: card
                anchors.centerIn: parent
                width: Math.min(600, parent.width - 48)
                height: Math.min(parent.height - 48, 640)
                radius: Theme.shape.extraLarge
                color: Theme.surfaceContainerHigh
                Elevation { target: card; level: 3 }
                MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onWheel: wheel => wheel.accepted = true }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 24
                    spacing: 16

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        Icon { name: "lyrics"; size: 26; fill: 1; color: Theme.primary }
                        Label { Layout.fillWidth: true; text: I18n.t("Search lyrics"); type: "headlineSmall" }
                        IconButton { icon: "close"; onClicked: search.close() }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        SearchField {
                            id: field
                            Layout.fillWidth: true
                            placeholder: I18n.t("Song title and artist")
                            onAccepted: Lyrics.search(field.text)
                        }
                        Button {
                            text: I18n.t("Search")
                            icon: "search"
                            enabled: field.text.trim() !== ""
                            onClicked: Lyrics.search(field.text)
                        }
                    }

                    Item {
                        Layout.fillWidth: true
                        Layout.fillHeight: true

                        ListView {
                            id: results
                            anchors.fill: parent
                            clip: true
                            spacing: 8
                            visible: !Lyrics.searching
                            model: Lyrics.searchResults
                            boundsBehavior: Flickable.StopAtBounds
                            delegate: Rectangle {
                                id: hit
                                required property var modelData
                                required property int index
                                readonly property bool synced: (modelData.synced ?? "") !== ""
                                width: results.width
                                implicitHeight: hitText.implicitHeight + 24
                                radius: Theme.shape.large
                                color: hitArea.containsMouse ? Qt.tint(Theme.surfaceContainerHighest, Qt.alpha(Theme.fgSurface, Theme.stateLayer.hover))
                                                             : Theme.surfaceContainerHighest
                                MouseArea {
                                    id: hitArea
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: search.pick(hit.index)
                                }
                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 16
                                    anchors.rightMargin: 16
                                    spacing: 12
                                    ColumnLayout {
                                        id: hitText
                                        Layout.fillWidth: true
                                        spacing: 2
                                        Label {
                                            Layout.fillWidth: true
                                            text: hit.modelData.title ?? ""
                                            type: "titleSmall"
                                            weight: 600
                                            elide: Text.ElideRight
                                        }
                                        Label {
                                            Layout.fillWidth: true
                                            text: [hit.modelData.artist ?? "", hit.modelData.album ?? ""].filter(s => s !== "").join(" • ")
                                            type: "bodySmall"
                                            color: Theme.fgSurfaceVariant
                                            elide: Text.ElideRight
                                        }
                                    }
                                    Label {
                                        visible: (hit.modelData.duration ?? 0) > 0
                                        text: {
                                            const s = Math.round(hit.modelData.duration ?? 0)
                                            return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
                                        }
                                        type: "bodySmall"
                                        color: Theme.fgSurfaceVariant
                                    }
                                    Rectangle {
                                        implicitWidth: badge.implicitWidth + 16
                                        implicitHeight: 24
                                        radius: 8
                                        color: hit.synced ? Theme.primaryContainer : Theme.surfaceContainer
                                        Label {
                                            id: badge
                                            anchors.centerIn: parent
                                            text: hit.modelData.instrumental && !hit.synced && (hit.modelData.plain ?? "") === ""
                                                  ? I18n.t("INSTRUMENTAL") : hit.synced ? I18n.t("SYNCED") : I18n.t("PLAIN")
                                            type: "labelSmall"
                                            weight: 700
                                            color: hit.synced ? Theme.fgPrimaryContainer : Theme.fgSurfaceVariant
                                        }
                                    }
                                }
                            }
                        }

                        LoadingIndicator {
                            anchors.centerIn: parent
                            visible: Lyrics.searching
                        }
                        Label {
                            anchors.centerIn: parent
                            width: parent.width - 32
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.WordWrap
                            visible: !Lyrics.searching && results.count === 0
                            text: Lyrics.searchError !== "" ? Lyrics.searchError : I18n.t("Search LRCLIB by song title and artist")
                            color: Theme.fgSurfaceVariant
                        }
                    }

                    Label {
                        Layout.fillWidth: true
                        visible: search.error !== ""
                        text: search.error
                        color: Theme.error
                        wrapMode: Text.WordWrap
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Button {
                            text: I18n.t("Import lyrics")
                            icon: "upload_file"
                            style: "outlined"
                            onClicked: importDialog.open()
                        }
                        Item { Layout.fillWidth: true }
                        Button { text: I18n.t("Close"); style: "text"; onClicked: search.close() }
                    }
                }
            }
        }
    }

    FileDialog {
        id: importDialog
        title: I18n.t("Import lyrics")
        fileMode: FileDialog.OpenFile
        nameFilters: [I18n.t("Lyrics ({0})", "*.lrc *.txt"), I18n.t("All files ({0})", "*")]
        onAccepted: {
            const err = Lyrics.importFile(selectedFile.toString())
            if (err !== "") {
                search.error = err
                return
            }
            search.close()
            root.say(I18n.t("Lyrics updated"))
        }
    }

    FileDialog {
        id: saveDialog
        title: I18n.t("Save lyrics")
        fileMode: FileDialog.SaveFile
        defaultSuffix: "lrc"
        nameFilters: [I18n.t("Lyrics ({0})", "*.lrc")]
        onAccepted: {
            const err = Lyrics.saveFile(selectedFile.toString())
            root.say(err !== "" ? err : I18n.t("Lyrics saved"))
        }
    }
}
