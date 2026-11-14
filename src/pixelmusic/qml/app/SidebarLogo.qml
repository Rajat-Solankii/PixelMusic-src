import QtQuick
import Md3
import PixelMusic.Core

// Sidebar header: the Pixel Music logo (Settings → logo colour) and the name. The logo is also the button that
// collapses / expands the sidebar, as in ChatGPT and Claude: on hover or keyboard focus it fades into a circle
// with the action icon. Transitions follow Theme.motion; instant with Theme.reducedMotion.
Item {
    id: root

    // Settings.logoColorway → the logo built from the Android app's drawables (tools/pixelmusic_logo.py)
    readonly property var _logoFiles: ({
        bluePurple: "blue_purple", babyPinkPurple: "baby_pink_purple", graphiteGray: "graphite",
        limeGreen: "lime_green", limeLemon: "lime_lemon", orangeYellow: "orange_yellow"
    })
    readonly property url logoSource: Qt.resolvedUrl("../../assets/logo/" + (_logoFiles[Settings.logoColorway] ?? "blue_purple") + ".svg")

    property real markSize: 40                 // logo side
    // PixelPlayer Desktop's sidebar header: a fully rounded pill (surfaceContainerHighest at 60 %) behind the
    // logo glyph (primary, 24 px) and the bold name. In the rail the pill is just the circle around the mark.
    property real pillWidth: markSize
    property bool expanded: true               // true — действие «свернуть», false — «развернуть»
    property bool wordmarkVisible: expanded
    // Наведение или фокус; стенды и снимки могут выставить вручную
    property bool highlighted: area.containsMouse || mark.activeFocus
    readonly property string actionIcon: expanded ? "left_panel_close" : "left_panel_open"
    readonly property string actionText: expanded ? I18n.t("Collapse sidebar") : I18n.t("Expand sidebar")
    readonly property alias markItem: mark

    signal activated()

    readonly property bool _instant: Theme.reducedMotion

    implicitWidth: wordmark.x + wordmark.width
    implicitHeight: markSize

    Rectangle {
        id: pill
        height: root.markSize
        width: root.expanded ? Math.max(root.markSize, root.pillWidth) : root.markSize
        anchors.verticalCenter: parent.verticalCenter
        radius: height / 2
        color: Qt.alpha(Theme.surfaceContainerHighest, 0.6)
        Behavior on width { enabled: !root._instant; NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.motion.emphasized } }
    }

    Item {
        id: mark
        objectName: "sidebarLogoMark"
        width: root.markSize
        height: root.markSize
        anchors.verticalCenter: parent.verticalCenter

        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: root.actionText
        Accessible.description: "Pixel Music"
        Accessible.onPressAction: root.activated()
        Keys.onPressed: e => {
            if (e.key === Qt.Key_Space || e.key === Qt.Key_Return || e.key === Qt.Key_Enter) {
                e.accepted = true
                root.activated()
            }
        }

        // 0 — знак (печенька + M), 1 — кнопка (круг + иконка действия)
        property real t: root.highlighted ? 1 : 0
        Behavior on t {
            enabled: !root._instant
            NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.motion.emphasized }
        }
        // «M» гаснет в первой половине морфа, иконка проявляется во второй (с небольшим нахлёстом,
        // чтобы не было пустого кадра) — без каши из двух глифов
        readonly property real glyphOut: Math.max(0, 1 - t / 0.55)
        readonly property real iconIn: Math.max(0, (t - 0.45) / 0.55)

        // Нажатие — короткое сжатие с пружиной
        scale: area.pressed ? 0.9 : 1
        Behavior on scale {
            enabled: !root._instant
            NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack; easing.overshoot: Theme.motion.overshoot }
        }

        // Слой состояния MD3 (Lilac поверх Ultraviolet): наведение 8 %, фокус 10 %, нажатие 10 %
        property real stateOpacity: area.pressed ? Theme.stateLayer.pressed
                                    : root.highlighted ? (mark.activeFocus && !area.containsMouse ? Theme.stateLayer.focus : Theme.stateLayer.hover) : 0
        Behavior on stateOpacity { enabled: !root._instant; NumberAnimation { duration: Theme.motion.effectsFast } }

        // the logo as a launcher icon: the mark on a white M3 Expressive tile (cookie). On hover / focus the tile
        // morphs into the round button behind the action icon and takes the button's colour.
        AppLogo {
            anchors.centerIn: parent
            size: root.markSize - 4
            scale: 1 + mark.t * 4 / (root.markSize - 4)
            shape: mark.t > 0.5 ? "circle" : "cookie9"
            tileColor: mark.t > 0.5 ? Qt.tint(Theme.secondaryContainer, Qt.alpha(Theme.fgSecondaryContainer, mark.stateOpacity)) : "white"
            markScale: 1.12
            markOpacity: mark.glyphOut
            Behavior on tileColor { enabled: !root._instant; ColorAnimation { duration: Theme.motion.effectsFast } }
        }

        Icon {
            anchors.centerIn: parent
            name: root.actionIcon
            size: 24
            weight: 500
            color: Theme.fgSecondaryContainer
            opacity: mark.iconIn
            visible: opacity > 0
            scale: 0.6 + 0.4 * mark.iconIn
            rotation: (1 - mark.iconIn) * (root.expanded ? 45 : -45)
        }

        // Индикатор фокуса MD3: кольцо 3 px с зазором 2 px, только при фокусе с клавиатуры
        Rectangle {
            anchors.centerIn: parent
            width: parent.width + 10
            height: width
            radius: width / 2
            color: "transparent"
            border.width: 3
            border.color: Theme.secondary
            visible: mark.activeFocus
        }

        MouseArea {
            id: area
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onPressed: tip.hide()
            onClicked: root.activated()
        }
    }

    Label {
        id: wordmark
        x: mark.x + mark.width
        anchors.verticalCenter: parent.verticalCenter
        text: "PixelMusic"
        type: "titleMedium"
        font.pixelSize: 18
        weight: 700
        color: Theme.fgSurface
        opacity: root.wordmarkVisible ? 1 : 0
        visible: opacity > 0
        Behavior on opacity { NumberAnimation { duration: root._instant ? 0 : Theme.motion.effectsFast } }
    }

    // Подсказка MD3 (plain tooltip). Живёт в корне окна: панель обрезает своё содержимое, а в рейле
    // подсказка стоит справа от знака, за краем панели. В развёрнутой панели — под знаком.
    Timer {
        id: tipDelay
        interval: 600
        onTriggered: tip.show()
    }
    Connections {
        target: area
        function onContainsMouseChanged() {
            if (area.containsMouse && !area.pressed)
                tipDelay.restart()
            else if (!mark.activeFocus) {
                tipDelay.stop()
                tip.hide()
            }
        }
    }
    Connections {
        target: mark
        function onActiveFocusChanged() {
            if (mark.activeFocus)
                tip.show()
            else if (!area.containsMouse)
                tip.hide()
        }
    }
    onActionTextChanged: if (tip.shown) tip.place()

    Rectangle {
        id: tip
        property bool shown: false
        function place() {
            const host = parent
            if (!host)
                return
            const p = root.expanded ? mark.mapToItem(host, 0, mark.height + 8)
                                    : mark.mapToItem(host, mark.width + 12, (mark.height - height) / 2)
            x = Math.max(8, Math.min(host.width - width - 8, Math.round(p.x)))
            y = Math.round(p.y)
        }
        function show() { if (root.visible) { place(); shown = true } }
        function hide() { tipDelay.stop(); shown = false }

        parent: root.Window.contentItem
        z: 1000
        width: tipLabel.implicitWidth + 16
        height: 24
        radius: Theme.shape.extraSmall
        color: Theme.inverseSurface
        opacity: shown && root.visible ? 1 : 0
        visible: opacity > 0
        Behavior on opacity { NumberAnimation { duration: root._instant ? 0 : Theme.motion.effectsFast } }

        Label {
            id: tipLabel
            anchors.centerIn: parent
            text: root.actionText
            type: "bodySmall"
            color: Theme.fgInverseSurface
        }
    }
}
