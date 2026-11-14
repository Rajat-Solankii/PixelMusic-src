import QtQuick

// Линейный прогресс MD3 Expressive: проигранная часть — бегущая волна, остаток — прямой трек
// с зазором и stop-точкой. На паузе (`wavy: false`) волна плавно выпрямляется.
// interactive: true превращает его в перемотку (сигналы moved/committed).
Item {
    id: root
    // the colour scheme it draws with: Theme, or PlayerTheme on the player surfaces
    property var scheme: Theme

    property real value: 0              // 0..1
    property bool wavy: true
    property bool interactive: false
    property real thickness: 4
    property real amplitude: 3
    property real wavelength: 40
    property real speed: 1              // периодов волны в секунду
    property real gap: 4
    property color activeColor: root.scheme.primary
    property color trackColor: root.scheme.secondaryContainer
    property color stopColor: root.scheme.primary
    // round thumb at the playhead (seek bars): a filled circle in activeColor drawn over the wave, a bit bigger
    // while hovered or dragged; 0 — none
    property real thumbSize: interactive ? Math.min(16, Math.max(10, Math.round(thickness * 2.5))) : 0
    readonly property real _thumb: thumbSize > 0 ? thumbSize * (drag.containsMouse || dragging ? 1.25 : 1) : 0

    readonly property bool dragging: drag.pressed
    property real dragValue: 0
    // after a release the bar stays where it was let go until the player reports the new position (no jump back)
    property bool _held: false
    readonly property real shownValue: dragging || _held ? dragValue : value
    onValueChanged: if (_held && Math.abs(value - dragValue) < 0.01) _held = false
    Timer { id: holdTimer; interval: 2000; onTriggered: root._held = false }

    signal moved(real value)
    signal committed(real value)

    implicitWidth: 240
    implicitHeight: Math.max(thickness + 2 * amplitude, 24) + 4

    // reduced motion: прямая линия, волна не бежит
    property real _amp: wavy && !Theme.reducedMotion ? amplitude : 0
    Behavior on _amp { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.motion.emphasized } }
    property real _phase: 0

    TickAnimation {
        running: root.visible && root._amp > 0.01 && root.speed !== 0 && !Theme.quiet
        onTriggered: root._phase = (root._phase + frameTime * root.speed * Math.PI * 2) % (Math.PI * 2)
    }

    readonly property real _cy: height / 2
    readonly property real _cap: thickness / 2
    // with a thumb the bar is inset by its radius, so the thumb stays inside the item at 0 % and 100 %
    readonly property real _x0: Math.max(_cap, thumbSize / 2)
    readonly property real _x1: width - _x0
    readonly property real _activeEnd: _x0 + (_x1 - _x0) * Math.max(0, Math.min(1, shownValue))

    // Отрисовка — шейдер (shaders/wavy.frag): смена фазы и значения — только uniform'ы, без пересборки
    // контура на CPU (Shape с сотнями точек пересобирался каждый кадр бегущей волны).
    ShaderEffect {
        anchors.fill: parent
        property size size: Qt.size(width, height)
        property real thickness: root.thickness
        // a sliver of progress shorter than a wave stays nearly flat instead of a lone hump
        property real amp: root._amp * Math.min(1, Math.max(0, root._activeEnd - root._x0) / root.wavelength)
        property real k: Math.PI * 2 / root.wavelength
        property real phase: root._phase
        property real activeEnd: root._activeEnd
        property real x0: root._x0
        property real x1: root._x1
        // the gap before the rest of the track starts at the thumb's edge when there is one
        readonly property real _clearance: root.gap + Math.max(root.thickness, root._thumb / 2)
        property real trackStart: Math.min(root._x1, root._activeEnd + _clearance)
        property real showStop: root._activeEnd < root._x1 - _clearance ? 1 : 0
        property real showActive: root.shownValue > 0 ? 1 : 0
        // the wave grows out of the centre line at its start and settles back onto it at its head (half a wave)
        property real taper: Math.min(root.wavelength * 0.5, Math.max(0, root._activeEnd - root._x0) * 0.5)
        property color activeColor: root.activeColor
        property color trackColor: root.trackColor
        property color stopColor: root.stopColor
        fragmentShader: Qt.resolvedUrl("shaders/wavy.frag.qsb")
    }

    Rectangle {
        visible: root.thumbSize > 0
        width: root._thumb
        height: width
        radius: width / 2
        x: root._activeEnd - width / 2
        y: root._cy - height / 2
        color: root.activeColor
        antialiasing: true
        Behavior on width { NumberAnimation { duration: Theme.motion.effectsFast; easing.type: Easing.OutCubic } }
    }

    MouseArea {
        id: drag
        anchors.fill: parent
        anchors.topMargin: -8
        anchors.bottomMargin: -8
        enabled: root.interactive
        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        preventStealing: true
        hoverEnabled: root.thumbSize > 0

        function valueAt(x) { return Math.max(0, Math.min(1, (x - root._x0) / (root._x1 - root._x0))) }

        onPressed: mouse => { root.dragValue = valueAt(mouse.x); root.moved(root.dragValue) }
        onPositionChanged: mouse => { if (pressed) { root.dragValue = valueAt(mouse.x); root.moved(root.dragValue) } }
        onReleased: { root._held = true; holdTimer.restart(); root.committed(root.dragValue) }
    }
}
