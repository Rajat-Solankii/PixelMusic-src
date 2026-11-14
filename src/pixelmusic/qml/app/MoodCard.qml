pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Effects
import Md3
import PixelMusic.Core

// A Moods & genres card (as in the Android app): a muted gradient of the category's own colour, its name in
// bold white at the top left, and the first two playlist covers of its page fanned at the bottom right,
// partly cut off by the card. Hover fans the covers out a little more; press squares the corners.
Item {
    id: root

    property string title
    property string browseId
    property string browseParams
    property color tint: "#5a6a8a"          // the category's colour (YouTube's tile stripe)
    property var covers: []                 // up to two cover URLs, filled by Home.moodCovers

    signal clicked()

    implicitWidth: 240
    implicitHeight: Math.round(width / 1.42)

    // a quiet version of the colour: same hue, less saturation, mid lightness; white text always reads on it
    readonly property real _h: tint.hslHue < 0 ? 0.6 : tint.hslHue
    readonly property real _s: Math.min(0.42, tint.hslSaturation * 0.6)
    readonly property real _l: Math.max(0.30, Math.min(0.42, tint.hslLightness * 0.8))
    readonly property color _top: Qt.hsla(_h, _s, _l + 0.07, 1)
    readonly property color _bottom: Qt.hsla(_h, Math.min(0.5, _s * 1.1), _l - 0.07, 1)
    readonly property real _radius: area.pressed ? 18 : 28
    readonly property bool _hot: area.containsMouse || activeFocus

    scale: area.pressed ? 0.97 : 1
    Behavior on scale { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutCubic } }

    Item {
        id: body
        anchors.fill: parent
        // the covers stick out of the card: the rounded card itself is the mask
        layer.enabled: true
        layer.smooth: true
        layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: mask
            maskThresholdMin: 0.5
            maskSpreadAtMin: 1.0
        }

        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                orientation: Gradient.Vertical
                GradientStop { position: 0; color: root._top }
                GradientStop { position: 1; color: root._bottom }
            }
        }

        // the two covers: the back one leans left, the front one right, both at the bottom-right corner
        Repeater {
            model: Math.min(2, root.covers.length)
            Rectangle {
                id: coverCard
                required property int index
                readonly property bool front: index === Math.min(2, root.covers.length) - 1
                readonly property real side: Math.round(root.height * 0.50)
                width: side
                height: side
                radius: 10
                color: Qt.rgba(0, 0, 0, 0.25)
                antialiasing: true
                x: root.width - side * (front ? 0.92 : 1.42) + (root._hot ? (front ? 4 : -10) : 0)
                y: root.height - side * (front ? 0.80 : 0.74) + (root._hot ? -6 : 0)
                rotation: (front ? 14 : -12) + (root._hot ? (front ? 4 : -6) : 0)
                z: front ? 2 : 1
                opacity: img.status === Image.Ready ? 1 : 0
                Behavior on x { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.OutBack; easing.overshoot: 1.2 } }
                Behavior on y { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.OutBack; easing.overshoot: 1.2 } }
                Behavior on rotation { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.OutBack; easing.overshoot: 1.2 } }
                Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsDefault } }

                MorphImage {
                    id: img
                    anchors.fill: parent
                    source: root.covers[coverCard.index] ?? ""
                    shape: "softSquare"
                }
            }
        }
    }

    Rectangle {
        id: mask
        anchors.fill: parent
        // the corner animates here (a Behavior can't sit on the read-only _radius); the ripple follows it
        radius: root._radius
        Behavior on radius { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack; easing.overshoot: Theme.motion.overshoot } }
        visible: false
        layer.enabled: true
    }

    Label {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 18
        anchors.topMargin: 16
        text: root.title
        type: root.width < 200 ? "titleMedium" : "titleLarge"
        weight: 700
        color: "white"
        wrapMode: Text.WordWrap
        maximumLineCount: 2
        elide: Text.ElideRight
        style: Text.Normal
    }

    StateLayer {
        id: area
        anchors.fill: parent
        radius: mask.radius
        color: "white"
        onClicked: root.clicked()
    }

    readonly property string _key: browseId + "|" + browseParams
    Connections {
        target: Home
        function onMoodCoversLoaded(key, covers) {
            if (key === root._key)
                root.covers = covers
        }
    }
    Component.onCompleted: if (browseId !== "") Home.moodCovers(browseId, browseParams)
}
