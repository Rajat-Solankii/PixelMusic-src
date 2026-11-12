import QtQuick
import QtQuick.Layouts
import Md3
import Materialfy.Core

// Listening stats (soundwave1 StatsScreen): time listened, top tracks and artists for a period, from the listen
// log kept on this computer (Spotify through this app and YouTube Music alike).
SettingsCard {
    id: root
    title: I18n.t("Listening stats")
    icon: "history"

    property bool compact: false
    readonly property var ranges: ["week", "month", "year", "all"]
    property int rangeIndex: 0
    property var summary: ({})
    readonly property var buckets: summary.buckets ?? []
    readonly property real maxBucket: Math.max(1, ...buckets.map(b => b.seconds))

    function reload() { summary = Extras.stats(ranges[rangeIndex]) }
    Component.onCompleted: reload()
    onRangeIndexChanged: reload()

    Connections {
        target: Extras
        function onStatsChanged() { root.reload() }
    }

    ButtonGroup {
        model: [I18n.t("This week"), I18n.t("This month"), I18n.t("This year"), I18n.t("All time")]
        size: root.compact ? "xs" : "s"
        currentIndex: root.rangeIndex
        onActivated: i => root.rangeIndex = i
    }

    // totals
    Flow {
        Layout.fillWidth: true
        spacing: 24
        Repeater {
            model: [
                { label: I18n.t("Listened"), value: Extras.formatTime(root.summary.totalSeconds ?? 0) },
                { label: I18n.t("Per day"), value: Extras.formatTime(root.summary.averagePerDaySeconds ?? 0) },
                { label: I18n.t("Plays"), value: String(root.summary.plays ?? 0) },
                { label: I18n.t("Artists"), value: String(root.summary.distinctArtists ?? 0) },
                { label: "YouTube Music", value: Extras.formatTime(root.summary.youtubeSeconds ?? 0) }
            ]
            delegate: ColumnLayout {
                required property var modelData
                spacing: 2
                Label { text: modelData.value; type: "titleLarge" }
                Label { text: modelData.label; type: "bodySmall"; color: Theme.fgSurfaceVariant }
            }
        }
    }

    // minutes per day (week, month) or per month (year)
    Item {
        Layout.fillWidth: true
        Layout.preferredHeight: 72
        visible: root.buckets.length > 0
        Row {
            anchors.fill: parent
            spacing: root.buckets.length > 12 ? 2 : 6
            Repeater {
                model: root.buckets
                delegate: Item {
                    required property var modelData
                    width: (parent.width - parent.spacing * (root.buckets.length - 1)) / root.buckets.length
                    height: parent.height
                    Rectangle {
                        anchors.bottom: parent.bottom
                        width: parent.width
                        height: Math.max(4, parent.height * modelData.seconds / root.maxBucket)
                        radius: Math.min(width / 2, 6)
                        color: modelData.seconds > 0 ? Theme.primary : Theme.secondaryContainer
                    }
                }
            }
        }
    }

    Label {
        visible: (root.summary.plays ?? 0) === 0
        Layout.fillWidth: true
        text: I18n.t("Nothing listened in this period yet")
        type: "bodyMedium"
        color: Theme.fgSurfaceVariant
    }

    GridLayout {
        Layout.fillWidth: true
        visible: (root.summary.plays ?? 0) > 0
        columns: root.compact ? 1 : 2
        columnSpacing: 24
        rowSpacing: 8

        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignTop
            spacing: 6
            Label { text: I18n.t("Top tracks"); type: "titleSmall" }
            Repeater {
                model: root.summary.topTracks ?? []
                delegate: RowLayout {
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true
                    spacing: 8
                    Label { text: String(index + 1); type: "labelLarge"; color: Theme.fgSurfaceVariant; Layout.preferredWidth: 20 }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Label { Layout.fillWidth: true; text: modelData.title; type: "bodyMedium"; elide: Text.ElideRight }
                        Label { Layout.fillWidth: true; text: modelData.subtitle; type: "bodySmall"; color: Theme.fgSurfaceVariant; elide: Text.ElideRight }
                    }
                    Label { text: I18n.t("{0}×", modelData.plays); type: "labelMedium"; color: Theme.fgSurfaceVariant }
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignTop
            spacing: 6
            Label { text: I18n.t("Top artists"); type: "titleSmall" }
            Repeater {
                model: root.summary.topArtists ?? []
                delegate: RowLayout {
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true
                    spacing: 8
                    Label { text: String(index + 1); type: "labelLarge"; color: Theme.fgSurfaceVariant; Layout.preferredWidth: 20 }
                    Label { Layout.fillWidth: true; text: modelData.title; type: "bodyMedium"; elide: Text.ElideRight }
                    Label { text: Extras.formatTime(modelData.seconds); type: "labelMedium"; color: Theme.fgSurfaceVariant }
                }
            }
        }
    }

    SettingRow {
        title: I18n.t("Clear listening stats")
        description: I18n.t("Kept on this computer only")
        Button {
            text: I18n.t("Clear")
            icon: "delete_sweep"
            style: "text"
            size: "xs"
            onClicked: Extras.clearStats()
        }
    }
}
