import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Home → "Listening stats" this week (PixelPlayer's home card): the time listened in big type, plays and the
// average per day as pills, the most played song, and a bar per day Mon–Sun where the busiest day is a full
// primary pill with its minutes in a round badge on top (the others tonal, today's label bold). Data:
// Extras.stats("week") (sf-services/stats.rs, the listen log on this computer); "See all" opens Settings → stats.
// Nothing played this week — no card.
Rectangle {
    id: root

    property bool compact: false
    property var summary: ({})
    readonly property var buckets: summary.buckets ?? []
    readonly property var topTrack: (summary.topTracks ?? [])[0] ?? null
    readonly property int busiest: {
        let best = -1
        for (let i = 0; i < buckets.length; i++)
            if (buckets[i].seconds > 0 && (best < 0 || buckets[i].seconds > buckets[best].seconds))
                best = i
        return best
    }
    readonly property real maxSeconds: busiest >= 0 ? buckets[busiest].seconds : 1
    readonly property int today: (new Date().getDay() + 6) % 7     // Monday = 0, like the buckets

    signal seeAll()

    function reload() { summary = Extras.stats("week") }
    Component.onCompleted: reload()
    Connections {
        target: Extras
        function onStatsChanged() { root.reload() }
    }

    visible: (summary.plays ?? 0) > 0
    implicitHeight: body.implicitHeight + 40
    radius: Theme.shape.extraLarge
    color: Theme.surfaceContainerHigh

    ColumnLayout {
        id: body
        anchors.fill: parent
        anchors.margins: 20
        spacing: 16

        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            Rectangle {
                implicitWidth: 40
                implicitHeight: 40
                radius: 20
                color: Theme.tertiaryContainer
                Icon { anchors.centerIn: parent; name: "insights"; size: 22; color: Theme.fgTertiaryContainer }
            }
            ColumnLayout {
                spacing: 0
                Label { text: I18n.t("Listening stats"); type: "titleLarge"; weight: 700 }
                Label { text: I18n.t("This week"); type: "bodyMedium"; color: Theme.fgSurfaceVariant }
            }
            Item { Layout.fillWidth: true }
            Button {
                text: I18n.t("See all")
                style: "text"
                icon: "chevron_right"
                onClicked: root.seeAll()
            }
        }

        GridLayout {
            Layout.fillWidth: true
            columns: root.compact ? 1 : 2
            columnSpacing: 32
            rowSpacing: 20

            // totals and the most played song
            ColumnLayout {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                spacing: 12

                ColumnLayout {
                    spacing: 0
                    Label {
                        text: Extras.formatTime(root.summary.totalSeconds ?? 0)
                        type: "displaySmall"
                        weight: 700
                        color: Theme.primary
                    }
                    Label { text: I18n.t("listened this week"); type: "bodyMedium"; color: Theme.fgSurfaceVariant }
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: 8
                    Repeater {
                        model: [
                            { icon: "headphones", text: I18n.n(root.summary.plays ?? 0, "{0} play", "{0} plays") },
                            { icon: "schedule", text: I18n.t("{0} a day", Extras.formatTime(root.summary.averagePerDaySeconds ?? 0)) },
                            { icon: "person", text: I18n.n(root.summary.distinctArtists ?? 0, "{0} artist", "{0} artists") }
                        ]
                        delegate: Rectangle {
                            required property var modelData
                            implicitWidth: pill.implicitWidth + 24
                            implicitHeight: 32
                            radius: 16
                            color: Theme.secondaryContainer
                            Row {
                                id: pill
                                anchors.centerIn: parent
                                spacing: 6
                                Icon { name: modelData.icon; size: 18; color: Theme.fgSecondaryContainer; anchors.verticalCenter: parent.verticalCenter }
                                Label { text: modelData.text; type: "labelLarge"; color: Theme.fgSecondaryContainer; anchors.verticalCenter: parent.verticalCenter }
                            }
                        }
                    }
                }
                // most played this week
                Rectangle {
                    visible: root.topTrack !== null
                    Layout.fillWidth: true
                    implicitHeight: 64
                    radius: Theme.shape.large
                    color: Theme.surfaceContainerHighest
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 12
                        MorphImage {
                            Layout.preferredWidth: 48
                            Layout.preferredHeight: 48
                            source: root.topTrack ? (root.topTrack.cover ?? "") : ""
                            shape: "cookie9"
                            fallbackKind: "track"
                            fallbackTitle: root.topTrack ? root.topTrack.title : ""
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Label { text: I18n.t("Top track"); type: "labelMedium"; color: Theme.primary }
                            Label { Layout.fillWidth: true; text: root.topTrack ? root.topTrack.title : ""; type: "titleSmall"; elide: Text.ElideRight }
                            Label { Layout.fillWidth: true; text: root.topTrack ? root.topTrack.subtitle : ""; type: "bodySmall"; color: Theme.fgSurfaceVariant; elide: Text.ElideRight }
                        }
                        Label {
                            text: root.topTrack ? I18n.t("{0}×", root.topTrack.plays) : ""
                            type: "titleMedium"
                            weight: 700
                            color: Theme.fgSurfaceVariant
                            Layout.rightMargin: 8
                        }
                    }
                }
            }

            // Mon–Sun bars
            Item {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.preferredHeight: 196
                Layout.maximumHeight: 196
                // the bars never leave the chart: while the card re-lays out (Home hidden under "See all",
                // new stats arriving) a bar could compute a huge height and paint stripes over the whole page
                clip: true
                Row {
                    id: bars
                    anchors.fill: parent
                    spacing: 10
                    Repeater {
                        model: root.buckets
                        delegate: Item {
                            id: day
                            required property var modelData
                            required property int index
                            readonly property bool best: index === root.busiest
                            readonly property real chart: height - 28      // labels under the bars
                            // the tallest a bar may be: the chart minus the busiest day's badge on top
                            readonly property real room: Math.max(12, chart - 40)
                            readonly property real share: root.maxSeconds > 0 ? Math.min(1, modelData.seconds / root.maxSeconds) : 0
                            width: Math.max(0, (bars.width - bars.spacing * 6) / 7)
                            height: bars.height

                            Rectangle {
                                id: bar
                                anchors.horizontalCenter: parent.horizontalCenter
                                y: day.chart - height
                                width: Math.min(parent.width, 40)
                                // the busiest is full height minus its badge; the rest scale against it
                                // a day without plays is a short flat pill, a light day at least a circle
                                height: day.modelData.seconds > 0
                                        ? Math.min(day.room, Math.max(width, day.room * day.share)) : 12
                                radius: Math.min(width, height) / 2
                                color: day.best ? Theme.primary
                                     : day.modelData.seconds > 0 ? Theme.secondaryContainer : Theme.surfaceContainerHighest
                                Behavior on height { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.OutCubic } }
                            }
                            // the busiest day's minutes in a round badge sitting on its bar
                            Rectangle {
                                visible: day.best
                                anchors.horizontalCenter: parent.horizontalCenter
                                anchors.bottom: bar.top
                                anchors.bottomMargin: 4
                                width: Math.max(36, badge.implicitWidth + 12)
                                height: 36
                                radius: 18
                                color: Theme.tertiaryContainer
                                Label {
                                    id: badge
                                    anchors.centerIn: parent
                                    text: Math.round(day.modelData.seconds / 60) + I18n.t("m")
                                    type: "labelMedium"
                                    weight: 700
                                    color: Theme.fgTertiaryContainer
                                }
                            }
                            Label {
                                anchors.horizontalCenter: parent.horizontalCenter
                                anchors.bottom: parent.bottom
                                text: Qt.locale().dayName((day.index + 1) % 7, Locale.NarrowFormat)
                                type: "labelLarge"
                                weight: day.index === root.today ? 800 : 400
                                color: day.index === root.today ? Theme.fgSurface : Theme.fgSurfaceVariant
                            }
                        }
                    }
                }
            }
        }
    }
}
