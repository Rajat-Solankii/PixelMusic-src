import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Queue row as a PixelPlayer-style card (the row height keeps a 4 px gap between cards): cover · title/artists ·
// length 36 px (the menu on hover) · drag handle on the right (rows that can move, on this computer). The playing
// song is a primary-container card with a cookie cover; a lifted card grows, rises and gets an outline.
Item {
    id: root

    property var track: ({})              // строка Player.queue (QUEUE_KEYS)
    property bool current: false
    property bool draggable: false
    property bool dragging: false
    property bool compact: false
    readonly property bool hovered: rowHover.hovered

    signal activated()
    signal dragStarted()
    signal dragMoved(real dy, real sceneY)
    signal dragFinished()

    implicitHeight: compact ? 60 : 68
    implicitWidth: 300

    function openMenu(item, x, y) {
        const p = item.mapToItem(null, x, y)
        Catalog.requestTrackMenu(track, p.x, p.y)
    }
    function openMenuAt(button) {   // у кнопки «⋯»
        const p = button.mapToItem(null, 0, 0)
        Catalog.requestTrackMenuAt(track, p.x, p.y, button.width, button.height)
    }
    function _time(ms) {
        const s = Math.round((ms ?? 0) / 1000)
        if (s >= 3600)
            return Math.floor(s / 3600) + ":" + String(Math.floor(s / 60) % 60).padStart(2, "0") + ":" + String(s % 60).padStart(2, "0")
        return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
    }

    readonly property color fg: current ? Theme.fgPrimaryContainer : Theme.fgSurface
    readonly property color fgVariant: current ? Qt.alpha(Theme.fgPrimaryContainer, 0.78) : Theme.fgSurfaceVariant

    Rectangle {
        id: bg
        anchors.fill: parent
        anchors.topMargin: 2
        anchors.bottomMargin: 2
        radius: root.current ? Theme.shape.extraLarge : Theme.shape.large
        // translucent: the queue also sits on Now Playing's cover backdrop
        color: root.dragging ? Theme.surfaceContainerHighest
             : root.current ? Theme.primaryContainer : Qt.alpha(Theme.surfaceContainerHigh, 0.55)
        border.width: root.dragging ? 1 : 0
        border.color: Qt.alpha(Theme.primary, 0.5)
        Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsDefault } }
    }
    Elevation { target: bg; level: root.dragging ? 3 : 0 }

    // под содержимым: клик по строке — перейти к ней
    StateLayer {
        anchors.fill: bg
        radius: bg.radius
        dragged: root.dragging
        onClicked: root.activated()
    }

    RowLayout {
        anchors.fill: bg
        anchors.leftMargin: 8
        anchors.rightMargin: root.draggable ? 2 : 10
        spacing: 10
        opacity: (root.track.available ?? true) ? 1 : Theme.stateLayer.disabledContent

        MorphImage {
            readonly property real side: root.compact ? 40 : 48
            Layout.preferredWidth: side
            Layout.preferredHeight: side
            source: root.track.cover ?? ""
            fallbackKind: (root.track.album ?? "") !== "" ? "album" : "track"
            fallbackArtist: (root.track.artists ?? "").split(", ")[0]
            fallbackTitle: (root.track.album ?? "") !== "" ? root.track.album : (root.track.title ?? "")
            shape: root.current ? "cookie9" : "softSquare"
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                Label {
                    // сжимается до ширины строки (иначе длинное название выталкивает длительность за край)
                    Layout.fillWidth: true
                    Layout.maximumWidth: Math.ceil(implicitWidth)
                    text: root.track.title ?? ""
                    type: "bodyLarge"
                    weight: 500
                    color: root.fg
                    Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsDefault } }
                }
                Rectangle {
                    visible: root.track.explicit === true
                    implicitWidth: 16
                    implicitHeight: 16
                    radius: 4
                    color: root.current ? Qt.alpha(Theme.fgPrimaryContainer, 0.14) : Theme.surfaceContainerHighest
                    Label { anchors.centerIn: parent; text: "E"; type: "labelSmall"; color: root.fgVariant }
                }
                Item { Layout.fillWidth: true }
            }
            ArtistLinks {
                Layout.fillWidth: true
                visible: root.track.isEpisode !== true
                refs: root.track.artistRefs ?? []
                text: root.track.artists ?? ""
                type: "bodyMedium"
                color: root.fgVariant
            }
            LinkLabel {
                Layout.fillWidth: true
                visible: root.track.isEpisode === true
                text: root.track.album || root.track.artists || ""
                type: "bodyMedium"
                color: root.fgVariant
                linkEnabled: (root.track.showId ?? "") !== ""
                onClicked: Catalog.openShow(root.track.showId)
            }
        }

        // Длительность; при наведении на её месте — «⋯» (панель узкая: отдельная колонка под кнопку
        // отнимала у названия ~40 px, и оно обрезалось даже у коротких треков)
        Item {
            Layout.preferredWidth: 36
            Layout.fillHeight: true
            readonly property bool menuShown: root.hovered || root.activeFocus
                                             || (menuLoader.item !== null && menuLoader.item.pressed)
            Label {
                anchors.fill: parent
                text: root._time(root.track.durationMs)
                type: "bodySmall"
                color: root.fgVariant
                horizontalAlignment: Text.AlignRight
                opacity: parent.menuShown ? 0 : 1
                Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
            }
            // «⋯» создаётся с первого наведения и остаётся: строк много, а наведена одна
            Loader {
                id: menuLoader
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                width: 32
                height: 32
                // активная с клавиатуры строка тоже показывает «⋯», иначе она скрывает длительность без кнопки
                active: root.hovered || root.activeFocus || item !== null
                sourceComponent: IconButton {
                    id: moreButton
                    anchors.fill: parent
                    icon: "more_horiz"
                    size: "xs"
                    iconSize: 22
                    iconWeight: 700
                    opacity: (root.hovered || moreButton.pressed || root.activeFocus) ? 1 : 0
                    visible: opacity > 0
                    Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
                    onClicked: root.openMenuAt(moreButton)
                }
            }
        }

        // drag handle: on the right and always shown, like PixelPlayer; the whole 32 px column grabs
        Item {
            id: handle
            visible: root.draggable
            Layout.preferredWidth: 32
            Layout.fillHeight: true
            Icon {
                anchors.centerIn: parent
                name: "drag_indicator"
                size: 20
                color: root.dragging ? Theme.primary : handleHover.hovered ? root.fg : root.fgVariant
                opacity: root.hovered || root.dragging || root.activeFocus ? 1 : 0.55
                Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
            }
            HoverHandler { id: handleHover; enabled: root.draggable; cursorShape: root.dragging ? Qt.ClosedHandCursor : Qt.OpenHandCursor }
            DragHandler {
                enabled: root.draggable
                target: null
                xAxis.enabled: false
                grabPermissions: PointerHandler.CanTakeOverFromAnything
                onActiveChanged: active ? root.dragStarted() : root.dragFinished()
                onTranslationChanged: if (active) root.dragMoved(translation.y, centroid.scenePosition.y)
            }
        }
    }

    HoverHandler { id: rowHover }
    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: point => root.openMenu(root, point.position.x, point.position.y)
    }
}
