import QtQuick
import QtQuick.Layouts
import Md3

// One settings category in the list (PixelPlayer-style): a pastel round icon, title, subtitle and a chevron.
// Rows sit in one rounded group: the first and last rows round the group's outer corners.
// Also used for About's update and project rows: a control placed inside goes before the trailing icon
// (trailingIcon "" hides it); a row that is not `clickable` has no hover or pointer.
Rectangle {
    id: root

    property string title
    property string subtitle
    property string icon
    // the icon's colour pair, a Material 3 container role: "primary" | "secondary" | "tertiary" | "error"
    property string tone: "primary"
    readonly property var _tones: ({
        primary: [Theme.primaryContainer, Theme.fgPrimaryContainer],
        secondary: [Theme.secondaryContainer, Theme.fgSecondaryContainer],
        tertiary: [Theme.tertiaryContainer, Theme.fgTertiaryContainer],
        error: [Theme.errorContainer, Theme.fgErrorContainer]
    })
    // a custom colour for the icon (0..360°, PixelPlayer-style rainbow of categories); < 0 uses `tone` instead.
    // The hue is harmonized toward the theme's primary (Material's "harmonize"), so the set follows the dynamic
    // colour scheme, and the container/icon tones follow dark or light mode.
    property real hue: -1
    readonly property real _hue: {
        if (hue < 0)
            return 0
        const p = Theme.primary.hslHue * 360
        const s = Theme.primary.hslSaturation
        let d = ((p - hue + 540) % 360) - 180
        // pull at most 15° toward primary, only when the theme has a real hue
        const pull = s > 0.08 ? Math.max(-15, Math.min(15, d * 0.25)) : 0
        return (((hue + pull) % 360) + 360) % 360 / 360
    }
    readonly property var _custom: Theme.dark
        ? [Qt.hsla(_hue, 0.62, 0.27, 1), Qt.hsla(_hue, 0.9, 0.86, 1)]
        : [Qt.hsla(_hue, 0.75, 0.87, 1), Qt.hsla(_hue, 0.8, 0.24, 1)]
    readonly property var _tone: hue >= 0 ? _custom : (_tones[tone] ?? _tones.primary)
    property bool first: false
    property bool last: false
    property string trailingIcon: "chevron_right"
    property bool clickable: true
    default property alias trailing: trailingSlot.data
    signal clicked()

    Layout.fillWidth: true
    // PixelPlayer Desktop's settings rows: 88 px, 24 px outer / 4 px inner corners of the group, 56 px icon circle
    implicitHeight: Math.max(88, text.implicitHeight + 32)
    readonly property real _outer: 24
    readonly property real _inner: Theme.shape.extraSmall
    topLeftRadius: first ? _outer : _inner
    topRightRadius: first ? _outer : _inner
    bottomLeftRadius: last ? _outer : _inner
    bottomRightRadius: last ? _outer : _inner
    color: root.clickable && area.containsMouse ? Qt.tint(Theme.surfaceContainerHigh, Qt.alpha(Theme.fgSurface, Theme.stateLayer.hover))
                              : Theme.surfaceContainerHigh
    Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsFast } }

    // under the content, so a control in the trailing slot gets its own clicks
    MouseArea {
        id: area
        anchors.fill: parent
        enabled: root.clickable
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.clicked()
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 16
        anchors.rightMargin: 16
        spacing: 16

        Rectangle {
            Layout.preferredWidth: 56
            Layout.preferredHeight: 56
            radius: 28
            color: root._tone[0]
            Icon {
                anchors.centerIn: parent
                name: root.icon
                size: 24
                fill: 1
                color: root._tone[1]
            }
        }
        ColumnLayout {
            id: text
            Layout.fillWidth: true
            spacing: 2
            Label { Layout.fillWidth: true; text: root.title; type: "titleMedium"; weight: 600 }
            Label {
                Layout.fillWidth: true
                visible: root.subtitle !== ""
                text: root.subtitle
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
        }
        Item {
            id: trailingSlot
            visible: children.length > 0
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
            Layout.preferredWidth: implicitWidth
            Layout.preferredHeight: implicitHeight
        }
        Icon { visible: root.trailingIcon !== ""; name: root.trailingIcon; size: 24; color: Theme.fgSurfaceVariant }
    }

}
