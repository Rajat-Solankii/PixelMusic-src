import QtQuick
import Md3
import PixelMusic.Core

// The Pixel Music logo as a launcher icon: the mark (Settings → logo colour) on a white tile cut in a
// Material 3 Expressive shape. Used by the sidebar header, Settings → About and the logo colour picker.
Item {
    id: root

    property real size: 40
    property var shape: "cookie9"
    property string colorway: Settings.logoColorway     // bluePurple | babyPinkPurple | graphiteGray | …
    property color tileColor: "white"
    property real markScale: 1.0                         // the mark's share of the tile (its SVG has its own margin)
    property real markOpacity: 1
    property alias tile: tileShape

    // Settings.logoColorway → assets/logo/<file>_mark.svg (tools/pixelmusic_logo.py)
    readonly property var _files: ({
        bluePurple: "blue_purple", babyPinkPurple: "baby_pink_purple", graphiteGray: "graphite",
        limeGreen: "lime_green", limeLemon: "lime_lemon", orangeYellow: "orange_yellow"
    })
    readonly property url markSource: Qt.resolvedUrl("../../assets/logo/" + (_files[colorway] ?? "blue_purple") + "_mark.svg")

    implicitWidth: size
    implicitHeight: size

    MorphShape {
        id: tileShape
        anchors.fill: parent
        shape: root.shape
        color: root.tileColor
        // a white tile on a light surface needs an edge to read as a tile
        strokeColor: Theme.dark ? "transparent" : Qt.alpha(Theme.outlineVariant, 0.9)
        strokeWidth: Theme.dark ? 0 : 1
    }

    Image {
        anchors.centerIn: parent
        width: root.size * root.markScale
        height: width
        source: root.markSource
        sourceSize: Qt.size(Math.ceil(width * Screen.devicePixelRatio), Math.ceil(height * Screen.devicePixelRatio))
        smooth: true
        mipmap: true
        opacity: root.markOpacity
        visible: opacity > 0
        scale: 0.6 + 0.4 * root.markOpacity
    }
}
