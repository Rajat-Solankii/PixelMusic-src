import QtQuick

// Play/Pause в «печеньке» MD3 Expressive. Во время воспроизведения фигура медленно вращается,
// при нажатии сжимается в более круглую, при смене состояния морфится.
Item {
    id: root
    // the colour scheme it draws with: Theme, or PlayerTheme on the player surfaces
    property var scheme: Theme

    property bool playing: false
    property real size: 64
    property color containerColor: root.scheme.primary
    property color iconColor: root.scheme.fgPrimary
    property var playingShape: "cookie9"
    property var pausedShape: "cookie4"
    property bool spin: true
    // выбранный трек ещё грузится: вместо значка — индикатор MD3, кнопка при этом остаётся нажимаемой
    property bool loading: false

    signal clicked()

    implicitWidth: size
    implicitHeight: size

    MorphShape {
        id: shape
        anchors.fill: parent
        anchors.margins: area.pressed ? root.size * 0.06 : 0
        Behavior on anchors.margins { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack } }
        shape: area.pressed ? "circle" : root.playing ? root.playingShape : root.pausedShape
        duration: Theme.motion.spatialDefault
        // недоступна (ничего не играет) — приглушённый контейнер MD3
        color: root.enabled ? root.containerColor : Qt.alpha(root.containerColor, 0.45)

        TickAnimation {
            running: root.spin && root.playing && root.visible && !Theme.quiet && !Theme.reducedMotion
            // поворот элемента, а не контура (angle) — без пересборки пути каждый кадр
            onTriggered: shape.rotation = (shape.rotation + frameTime * 30) % 360
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: root.iconColor
        opacity: area.containsMouse && !area.pressed ? Theme.stateLayer.hover : 0
        Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
    }

    Icon {
        anchors.centerIn: parent
        visible: !root.loading
        name: root.playing ? "pause" : "play_arrow"
        size: root.size * 0.45
        fill: 1
        color: root.enabled ? root.iconColor : Qt.alpha(root.iconColor, 0.6)
    }

    LoadingIndicator {
        anchors.centerIn: parent
        size: root.size * 0.5
        color: root.enabled ? root.iconColor : root.scheme.fgSurface
        visible: root.loading
        running: root.loading && root.visible
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: root.clicked()
    }
}
