import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// PixelPlayer Desktop's queue header (QueueSidePanel): a queue icon in a 34 px primaryContainer circle,
// "Next Up" in bold titleMedium and the song count; on the right locate · shuffle · repeat · clear as one connected
// group of 40 × 36 px buttons (26 px outer, 4 px inner corners, 2 px apart), solid tonal containers with filled,
// heavier icons so they read as buttons, not as disabled glyphs. «Locate» slides in when the playing song
// has scrolled away, and the shuffle button's start corner tightens to meet it.
Item {
    id: root

    property int count: 0
    property bool locateShown: false
    property bool canClear: false

    signal locate()
    signal clear()

    implicitHeight: 48
    implicitWidth: 320

    // on = primary / onPrimary, off = secondaryContainer / onSecondaryContainer (solid); disabled = the off look
    // with a faded icon (the container stays solid so the group keeps its shape)
    component SegButton: Item {
        id: seg
        property string icon
        property bool on: false
        property real startRadius: 4
        property real endRadius: 4
        signal clicked()
        implicitWidth: 40
        implicitHeight: 36
        Behavior on startRadius { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutCubic } }
        Rectangle {
            anchors.fill: parent
            topLeftRadius: seg.startRadius
            bottomLeftRadius: seg.startRadius
            topRightRadius: seg.endRadius
            bottomRightRadius: seg.endRadius
            color: seg.on && seg.enabled ? Theme.primary : Theme.secondaryContainer
            Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsDefault } }
        }
        Icon {
            anchors.centerIn: parent
            name: seg.icon
            size: 20
            fill: 1
            weight: 600
            color: !seg.enabled ? Qt.alpha(Theme.fgSecondaryContainer, 0.38)
                 : seg.on ? Theme.fgPrimary : Theme.fgSecondaryContainer
            Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsDefault } }
        }
        StateLayer {
            anchors.fill: parent
            enabled: seg.enabled
            topLeftRadius: seg.startRadius
            bottomLeftRadius: seg.startRadius
            topRightRadius: seg.endRadius
            bottomRightRadius: seg.endRadius
            color: seg.on ? Theme.fgPrimary : Theme.fgSecondaryContainer
            onClicked: seg.clicked()
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 2
        spacing: 8
        Rectangle {
            Layout.preferredWidth: 34
            Layout.preferredHeight: 34
            radius: 17
            color: Theme.primaryContainer
            Icon { anchors.centerIn: parent; name: "queue_music"; size: 18; color: Theme.fgPrimaryContainer }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            Label {
                Layout.fillWidth: true
                text: I18n.t("Next Up")
                type: "titleMedium"
                weight: 700
            }
            Label {
                Layout.fillWidth: true
                text: I18n.n(root.count, "{0} song", "{0} songs")
                type: "labelSmall"
                color: Qt.alpha(Theme.fgSurfaceVariant, 0.7)
            }
        }
        Row {
            spacing: 2
            Item {
                width: root.locateShown ? 40 : 0
                height: 36
                clip: true
                Behavior on width { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutCubic } }
                SegButton {
                    x: root.locateShown ? 0 : 20
                    opacity: root.locateShown ? 1 : 0
                    Behavior on x { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutCubic } }
                    Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsDefault } }
                    icon: "target"
                    startRadius: 26
                    onClicked: root.locate()
                }
            }
            SegButton {
                icon: "shuffle"
                on: Player.shuffle
                startRadius: root.locateShown ? 4 : 26
                enabled: Player.hasTrack && Player.source !== "radio"
                onClicked: Player.shuffle = !Player.shuffle
            }
            SegButton {
                icon: Player.repeat === "one" ? "repeat_one" : "repeat"
                on: Player.repeat !== "off"
                enabled: Player.hasTrack
                onClicked: Player.cycleRepeat()
            }
            SegButton {
                icon: "delete_sweep"
                endRadius: 26
                enabled: root.canClear
                onClicked: root.clear()
            }
        }
    }
}
