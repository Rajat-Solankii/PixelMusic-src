import QtQuick
import Md3
import PixelMusic.Core

// Кодек играющего трека (OGG 320…) — маленькая плашка над временем в мини-плеере.
// У чужого устройства кодека нет: на этом месте RemoteLabel.
// The artists come first: with `room` (what the row leaves after the artist names) the badge drops the
// source ("Monochrome · FLAC 16/44.1" → "FLAC 16/44.1"), then elides, and hides when even that won't fit.
Rectangle {
    id: root

    property real room: Infinity
    readonly property string full: Player.codec
    readonly property string brief: full.split(" · ").pop()
    readonly property real _fullWidth: fullMetrics.advanceWidth + 12
    readonly property real _briefWidth: briefMetrics.advanceWidth + 12
    readonly property bool _showFull: _fullWidth <= room
    readonly property real _minimum: 44

    visible: Player.hasTrack && full !== "" && !Player.remote && room >= _minimum
    implicitWidth: _showFull ? _fullWidth : Math.max(_minimum, Math.min(_briefWidth, room))
    implicitHeight: 16
    radius: 5
    color: Theme.surfaceContainerHigh

    TextMetrics { id: fullMetrics; text: root.full; font: codec.font }
    TextMetrics { id: briefMetrics; text: root.brief; font: codec.font }

    Label {
        id: codec
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        horizontalAlignment: Text.AlignHCenter
        elide: Text.ElideRight
        text: root._showFull ? root.full : root.brief
        type: "labelSmall"
        weight: 600
        color: Theme.fgSurfaceVariant
    }
}
