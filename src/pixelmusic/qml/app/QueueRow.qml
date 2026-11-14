import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Queue row as PixelPlayer Desktop's EnhancedSongListItem: a surfaceContainerLow card (20 px corners, 10 px
// padding, 48 px cover with 10 px corners) with the drag handle on the left and the kebab menu in a 34 px circle on
// the right. The playing song fades (280 ms) into a solid primaryContainer pill: corners 20 → 32, cover 10 → 22,
// bold title and the playing bars. A lifted card turns surfaceContainerHighest and casts a soft shadow that fades in
// (kept inside the list's bleed so the list's clip never cuts it into a band). Every container is a solid colour.
// The row height includes the gap between cards (rowGap, 8 px like the target's spacedBy(8.dp)).
Item {
    id: root

    property var track: ({})              // строка Player.queue (QUEUE_KEYS)
    property bool current: false
    property bool draggable: false
    property bool dragging: false
    property bool compact: false
    readonly property bool hovered: rowHover.hovered
    readonly property real rowGap: 8

    signal activated()
    signal dragStarted()
    signal dragMoved(real dy, real sceneY)
    signal dragFinished()

    implicitHeight: (compact ? 60 : 68) + rowGap
    implicitWidth: 300

    function openMenu(item, x, y) {
        const p = item.mapToItem(null, x, y)
        Catalog.requestTrackMenu(track, p.x, p.y)
    }
    function openMenuAt(button) {
        const p = button.mapToItem(null, 0, 0)
        Catalog.requestTrackMenuAt(track, p.x, p.y, button.width, button.height)
    }
    function _mix(a, b, t) {
        return Qt.rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, a.a + (b.a - a.a) * t)
    }

    // 0 → 1 while the row becomes the playing one (tween 280 ms, as the target)
    property real hl: current ? 1 : 0
    Behavior on hl { enabled: !Theme.reducedMotion; NumberAnimation { duration: 280; easing.type: Easing.InOutQuad } }

    readonly property var _coverSquare: ({ kind: "square", p: 7 })
    readonly property var _coverRound: ({ kind: "square", p: 3 })

    readonly property color fg: _mix(Theme.fgSurface, Theme.fgPrimaryContainer, hl)
    // The target's cards are surfaceContainerLow on its `background`, one tone step above the panel. Pixel Music's
    // panels are surfaceContainer, so the same step up is surfaceContainerHigh: solid, also over Now Playing's
    // cover backdrop (the scheme follows the cover there).
    readonly property color _base: Theme.surfaceContainerHigh
    readonly property color _container: _mix(_base, Theme.primaryContainer, hl)

    // the lifted card grows a little (1.025, no bounce)
    scale: dragging ? 1.025 : 1
    Behavior on scale { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.OutCubic } }

    // the lift shadow: soft, a little below the card, fading with the lift
    RectangularShadow {
        anchors.fill: bg
        radius: bg.radius
        offset.y: 3
        blur: 10
        spread: 0
        color: Qt.alpha(Theme.shadow, Theme.dark ? 0.45 : 0.22)
        opacity: root.dragging ? 1 : 0
        visible: opacity > 0
        Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsDefault } }
    }

    Rectangle {
        id: bg
        anchors.fill: parent
        anchors.topMargin: root.rowGap / 2
        anchors.bottomMargin: root.rowGap / 2
        radius: 20 + 12 * root.hl
        color: root.dragging ? Theme.surfaceContainerHighest
             : rowHover.hovered && !root.current ? Qt.tint(root._container, Qt.alpha(Theme.fgSurface, 0.05))
             : root._container
        Behavior on color { enabled: !Theme.transitioning && root.hl % 1 === 0; ColorAnimation { duration: Theme.motion.effectsFast } }
    }

    // под содержимым: клик по строке — перейти к ней
    StateLayer {
        anchors.fill: bg
        radius: bg.radius
        color: root.fg
        // the card's own hover tint is the state layer here
        opacity: root.dragging ? 1 : 0
        onClicked: root.activated()
    }

    RowLayout {
        anchors.fill: bg
        anchors.leftMargin: 10
        anchors.rightMargin: 10
        spacing: 0
        opacity: (root.track.available ?? true) ? 1 : Theme.stateLayer.disabledContent

        // drag handle on the left, as the target; the whole 32 px square grabs
        Item {
            id: handle
            visible: root.draggable
            Layout.preferredWidth: 32
            Layout.preferredHeight: 32
            Layout.rightMargin: 6
            Icon {
                anchors.centerIn: parent
                name: "drag_indicator"
                size: 20
                color: root.current || root.dragging ? Qt.alpha(Theme.fgPrimaryContainer, 0.85)
                     : Qt.alpha(Theme.fgSurfaceVariant, handleHover.hovered ? 0.8 : 0.45)
                Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsFast } }
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

        // cover: a rounded square that rounds further on the playing pill (10 → 22 px corners at 48 px)
        MorphImage {
            readonly property real side: root.compact ? 40 : 48
            Layout.preferredWidth: side
            Layout.preferredHeight: side
            Layout.rightMargin: root.compact ? 12 : 14
            source: root.track.cover ?? ""
            fallbackKind: (root.track.album ?? "") !== "" ? "album" : "track"
            fallbackArtist: (root.track.artists ?? "").split(", ")[0]
            fallbackTitle: (root.track.album ?? "") !== "" ? root.track.album : (root.track.title ?? "")
            shape: root.current ? root._coverRound : root._coverSquare
            placeholderColor: root.current ? Qt.alpha(Theme.fgPrimaryContainer, 0.12) : Theme.surfaceContainerHighest
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                Label {
                    Layout.fillWidth: true
                    Layout.maximumWidth: Math.ceil(implicitWidth)
                    text: root.track.title ?? ""
                    type: "bodyLarge"
                    font.pixelSize: root.compact ? 14 : 15
                    weight: root.current ? 700 : 600
                    color: root.fg
                }
                Rectangle {
                    visible: root.track.explicit === true
                    implicitWidth: 16
                    implicitHeight: 16
                    radius: 4
                    color: root.current ? Qt.tint(Theme.primaryContainer, Qt.alpha(Theme.fgPrimaryContainer, 0.14)) : Theme.surfaceContainerHighest
                    Label { anchors.centerIn: parent; text: "E"; type: "labelSmall"; color: Qt.alpha(root.fg, 0.7) }
                }
                Item { Layout.fillWidth: true }
            }
            ArtistLinks {
                Layout.fillWidth: true
                visible: root.track.isEpisode !== true
                refs: root.track.artistRefs ?? []
                text: root.track.artists ?? ""
                type: "bodyMedium"
                color: Qt.alpha(root.fg, 0.7 + 0.15 * root.hl)
            }
            LinkLabel {
                Layout.fillWidth: true
                visible: root.track.isEpisode === true
                text: root.track.album || root.track.artists || ""
                type: "bodyMedium"
                color: Qt.alpha(root.fg, 0.7 + 0.15 * root.hl)
                linkEnabled: (root.track.showId ?? "") !== ""
                onClicked: Catalog.openShow(root.track.showId)
            }
        }

        PlayingEqIcon {
            visible: root.current
            Layout.preferredWidth: 20
            Layout.preferredHeight: 18
            Layout.leftMargin: 6
            Layout.rightMargin: 6
            color: Theme.fgPrimaryContainer
            playing: Player.playing
        }

        // kebab menu in a 34 px circle (solid: highest container, on the playing pill primaryContainer tinted 15 %)
        Item {
            id: more
            Layout.preferredWidth: 34
            Layout.preferredHeight: 34
            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: root._mix(Theme.surfaceContainerHighest, Qt.tint(Theme.primaryContainer, Qt.alpha(Theme.fgPrimaryContainer, 0.15)), root.hl)
            }
            Icon {
                anchors.centerIn: parent
                name: "more_vert"
                size: 20
                color: root._mix(Theme.fgSurfaceVariant, Theme.fgPrimaryContainer, root.hl)
            }
            StateLayer {
                anchors.fill: parent
                radius: width / 2
                color: root.fg
                onClicked: root.openMenuAt(more)
            }
        }
    }

    HoverHandler { id: rowHover }
    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: point => root.openMenu(root, point.position.x, point.position.y)
    }
}
