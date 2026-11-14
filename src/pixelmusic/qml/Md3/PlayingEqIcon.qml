pragma ComponentBehavior: Bound
import QtQuick

// Three rounded bars that wander up and down while a song plays (PixelPlayer's PlayingEqIcon): each bar follows
// its own sine with a slow drift; paused, the bars fold to dots (240 ms).
Item {
    id: root

    property color color: Theme.primary
    property bool playing: true
    property int bars: 3
    property real minFraction: 0.28
    property real gapFraction: 0.30
    readonly property int _phaseMs: 3600
    readonly property int _wanderMs: 12000

    implicitWidth: 20
    implicitHeight: 18

    property real _phase: 0
    property real _activity: playing ? 1 : 0
    Behavior on _activity { NumberAnimation { duration: 240; easing.type: Easing.OutCubic } }

    Timer {
        interval: 40
        repeat: true
        running: root.playing && root.visible && !Theme.reducedMotion
        property double last: 0
        onRunningChanged: last = Date.now()
        onTriggered: {
            const now = Date.now()
            const dt = Math.min(0.2, (now - last) / 1000)
            last = now
            root._phase = (root._phase + dt * 2 * Math.PI / (root._phaseMs / 1000)) % (2 * Math.PI)
        }
    }

    readonly property real _barW: width / (bars + (bars - 1) * (1 + gapFraction))

    Repeater {
        model: root.bars
        Rectangle {
            required property int index
            readonly property real wander: root._phase * root._phaseMs / root._wanderMs
            readonly property real v: {
                const slowShift = 0.6 * Math.sin(wander + index * 0.4)
                const slowAmp = 0.85 + 0.15 * Math.sin(wander * 0.5 + 1.1 + index * 0.3)
                const s = (Math.sin(root._phase * (index + 1) + index * 0.9 + slowShift) * slowAmp + 1) * 0.5
                return s * s * (3 - 2 * s)
            }
            readonly property real barH: root.height * (root.minFraction + (1 - root.minFraction) * v)
            width: root._barW
            height: width + (barH - width) * root._activity
            x: index * (root._barW + root._barW * root.gapFraction)
            y: (root.height - height) / 2
            radius: width / 2
            color: root.color
        }
    }
}
