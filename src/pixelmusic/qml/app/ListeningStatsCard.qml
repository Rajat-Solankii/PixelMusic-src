import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Listening stats, Material 3 Expressive (the mobile app's stats screen): range chips (Today / Week / Month /
// Year / All time), two stat cards (listening time, plays) with an icon, a big number and a pill, the rhythm
// chart (rounded pill bars, the picked bar with a check badge; listening time or play count), the top tracks /
// artists / albums with a "Top pick" hero card and ranked rows with progress bars, listening habits (2 × 2) and
// listening variety (a stacked bar of the top artists' share). Every container is a solid colour.
// Data: Extras.stats(range) (sf-services stats.rs summary), from the listen log kept on this computer.
ColumnLayout {
    id: root

    property bool compact: false
    readonly property var ranges: ["today", "week", "month", "year", "all"]
    property int rangeIndex: 1
    property var summary: ({})
    readonly property string range: ranges[rangeIndex]
    readonly property var buckets: summary.buckets ?? []
    readonly property bool hasPlays: (summary.plays ?? 0) > 0
    // chart: 0 listening time, 1 play count
    property int metric: 0
    // the bar picked by a click (-1: the biggest one)
    property int picked: -1
    // top lists: 0 tracks, 1 artists, 2 albums
    property int topKind: 0

    spacing: 16

    function reload() { summary = Extras.stats(range); picked = -1 }
    Component.onCompleted: reload()
    onRangeIndexChanged: reload()
    Connections {
        target: Extras
        function onStatsChanged() { root.reload() }
    }

    function _value(b) { return metric === 0 ? (b.seconds ?? 0) : (b.plays ?? 0) }
    readonly property real _max: Math.max(1, ...buckets.map(b => _value(b)))
    readonly property int _biggest: {
        let best = -1, most = 0
        for (let i = 0; i < buckets.length; i++) {
            const v = _value(buckets[i])
            if (v > most) { most = v; best = i }
        }
        return best
    }
    readonly property int _selected: picked >= 0 && picked < buckets.length ? picked : _biggest

    function _barLabel(b, i) {
        if (range === "today") {
            const h = parseInt(b.label, 10)
            return Qt.formatTime(new Date(2000, 0, 1, h, 0), "h AP").replace(":00", "")
        }
        if (range === "year")
            return Qt.locale().standaloneMonthName(i, Locale.NarrowFormat)
        const d = new Date(b.label + "T12:00:00")
        if (range === "week")
            return Qt.locale().standaloneDayName(d.getDay(), Locale.ShortFormat)
        // month: every fifth day, the rest stay quiet
        return (d.getDate() === 1 || d.getDate() % 5 === 0) ? String(d.getDate()) : ""
    }
    function _barTitle(b) {
        if (range === "today") {
            const h = parseInt(b.label, 10)
            return I18n.t("{0}–{1}", Qt.formatTime(new Date(2000, 0, 1, h, 0), "h AP"),
                          Qt.formatTime(new Date(2000, 0, 1, (h + 3) % 24, 0), "h AP"))
        }
        if (range === "year")
            return Qt.locale().standaloneMonthName(parseInt(b.label.split("-")[1], 10) - 1, Locale.LongFormat)
        return new Date(b.label + "T12:00:00").toLocaleDateString(Qt.locale(), "dddd, d MMMM")
    }
    function _metricText(b) {
        return metric === 0 ? Extras.formatTime(b.seconds ?? 0) : I18n.n(b.plays ?? 0, "{0} play", "{0} plays")
    }

    // ---- building blocks ----
    component StatCard: Rectangle {
        id: card
        property string icon
        property string label
        property string value
        property string pill
        property color tone
        property color fgTone
        Layout.fillWidth: true
        implicitHeight: cardCol.implicitHeight + 40
        radius: 28
        color: tone
        ColumnLayout {
            id: cardCol
            x: 20
            y: 20
            width: parent.width - 40
            spacing: 10
            RowLayout {
                spacing: 10
                Rectangle {
                    Layout.preferredWidth: 40
                    Layout.preferredHeight: 40
                    radius: 20
                    color: card.fgTone
                    Icon { anchors.centerIn: parent; name: card.icon; size: 22; fill: 1; color: card.tone }
                }
                Label { text: card.label; type: "titleMedium"; weight: 600; color: card.fgTone }
            }
            Label {
                Layout.fillWidth: true
                text: card.value
                type: "displaySmall"
                weight: 700
                color: card.fgTone
                fontSizeMode: Text.HorizontalFit
                minimumPixelSize: 20
            }
            Rectangle {
                visible: card.pill !== ""
                implicitWidth: pillText.implicitWidth + 24
                implicitHeight: 28
                radius: 14
                color: Qt.tint(card.tone, Qt.alpha(card.fgTone, 0.12))
                Label { id: pillText; anchors.centerIn: parent; text: card.pill; type: "labelLarge"; weight: 600; color: card.fgTone }
            }
        }
    }

    component HabitCard: Rectangle {
        id: habit
        property string icon
        property string label
        property string value
        property string note
        Layout.fillWidth: true
        Layout.preferredHeight: Math.max(124, habitCol.implicitHeight + 32)
        radius: 24
        color: Theme.surfaceContainerHigh
        ColumnLayout {
            id: habitCol
            x: 16
            y: 16
            width: parent.width - 32
            spacing: 6
            Rectangle {
                Layout.preferredWidth: 36
                Layout.preferredHeight: 36
                radius: 12
                color: Theme.secondaryContainer
                Icon { anchors.centerIn: parent; name: habit.icon; size: 20; fill: 1; color: Theme.fgSecondaryContainer }
            }
            Label { Layout.fillWidth: true; text: habit.label; type: "labelLarge"; color: Theme.fgSurfaceVariant }
            Label { Layout.fillWidth: true; text: habit.value; type: "titleLarge"; weight: 700; elide: Text.ElideRight }
            Label {
                visible: habit.note !== ""
                Layout.fillWidth: true
                text: habit.note
                type: "bodySmall"
                color: Theme.fgSurfaceVariant
                elide: Text.ElideRight
            }
        }
    }

    component SectionLabel: Label {
        Layout.fillWidth: true
        Layout.topMargin: 8
        Layout.leftMargin: 4
        type: "titleMedium"
        weight: 700
    }

    // ---- range chips ----
    Flow {
        Layout.fillWidth: true
        spacing: 8
        Repeater {
            model: [I18n.t("Today"), I18n.t("Week"), I18n.t("Month"), I18n.t("Year"), I18n.t("All time")]
            Chip {
                required property string modelData
                required property int index
                text: modelData
                selected: root.rangeIndex === index
                onClicked: root.rangeIndex = index
            }
        }
    }

    // ---- the two stat cards ----
    GridLayout {
        Layout.fillWidth: true
        columns: root.compact ? 1 : 2
        columnSpacing: 12
        rowSpacing: 12
        StatCard {
            icon: "schedule"
            label: I18n.t("Listening time")
            value: Extras.formatTime(root.summary.totalSeconds ?? 0)
            pill: root.range === "today" ? "" : I18n.t("{0} a day", Extras.formatTime(root.summary.averagePerDaySeconds ?? 0))
            tone: Theme.primaryContainer
            fgTone: Theme.fgPrimaryContainer
        }
        StatCard {
            icon: "headphones"
            label: I18n.t("Plays")
            value: String(root.summary.plays ?? 0)
            pill: I18n.n(root.summary.distinctTracks ?? 0, "{0} track", "{0} tracks")
            tone: Theme.tertiaryContainer
            fgTone: Theme.fgTertiaryContainer
        }
    }

    // ---- rhythm chart ----
    Rectangle {
        Layout.fillWidth: true
        visible: root.buckets.length > 0
        implicitHeight: chartCol.implicitHeight + 40
        radius: 28
        color: Theme.surfaceContainerHigh
        ColumnLayout {
            id: chartCol
            x: 20
            y: 20
            width: parent.width - 40
            spacing: 16
            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Label {
                        Layout.fillWidth: true
                        text: root.range === "today" ? I18n.t("Today by the hour")
                            : root.range === "week" ? I18n.t("Weekly rhythm")
                            : root.range === "month" ? I18n.t("Month at a glance") : I18n.t("Year at a glance")
                        type: "titleLarge"
                        weight: 700
                    }
                    Label {
                        Layout.fillWidth: true
                        text: root._selected >= 0
                              ? root._barTitle(root.buckets[root._selected]) + " · " + root._metricText(root.buckets[root._selected])
                              : I18n.t("Nothing listened in this period yet")
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                        elide: Text.ElideRight
                    }
                }
                ButtonGroup {
                    visible: !root.compact
                    model: [I18n.t("Listening time"), I18n.t("Play count")]
                    size: "xs"
                    currentIndex: root.metric
                    onActivated: i => root.metric = i
                }
            }
            ButtonGroup {
                visible: root.compact
                model: [I18n.t("Listening time"), I18n.t("Play count")]
                size: "xs"
                currentIndex: root.metric
                onActivated: i => root.metric = i
            }
            Item {
                id: bars
                Layout.fillWidth: true
                Layout.preferredHeight: 168
                readonly property real gap: root.buckets.length > 12 ? 3 : 8
                readonly property real barWidth: Math.max(4, (width - gap * (root.buckets.length - 1)) / Math.max(1, root.buckets.length))
                readonly property real plotHeight: height - 24
                Repeater {
                    model: root.buckets
                    Item {
                        id: bar
                        required property var modelData
                        required property int index
                        readonly property bool selected: index === root._selected
                        readonly property real v: root._value(modelData)
                        x: index * (bars.barWidth + bars.gap)
                        width: bars.barWidth
                        height: bars.height
                        Rectangle {
                            id: pillBar
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: 24
                            anchors.horizontalCenter: parent.horizontalCenter
                            // a pill at most 36 px wide in its slot; an empty day is a dot
                            width: Math.min(parent.width, 36)
                            height: Math.max(width, bars.plotHeight * bar.v / root._max)
                            radius: width / 2
                            color: bar.selected ? Theme.primary : bar.v > 0 ? Theme.secondaryContainer : Theme.surfaceContainerHighest
                            Behavior on height { enabled: !Theme.reducedMotion; NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.motion.emphasizedDecelerate } }
                            Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsFast } }
                            // the check badge on the picked bar
                            Rectangle {
                                visible: bar.selected && bar.v > 0 && parent.width >= 14
                                anchors.horizontalCenter: parent.horizontalCenter
                                anchors.top: parent.top
                                anchors.topMargin: 4
                                width: Math.min(parent.width - 4, 22)
                                height: width
                                radius: width / 2
                                color: Theme.fgPrimary
                                Icon { anchors.centerIn: parent; name: "check"; size: parent.width * 0.75; weight: 700; color: Theme.primary }
                            }
                        }
                        Label {
                            anchors.bottom: parent.bottom
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: root._barLabel(bar.modelData, bar.index)
                            type: "labelSmall"
                            weight: bar.selected ? 700 : 400
                            color: bar.selected ? Theme.primary : Theme.fgSurfaceVariant
                        }
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.picked = bar.index
                        }
                    }
                }
            }
        }
    }

    // ---- top lists ----
    SectionLabel { visible: root.hasPlays; text: I18n.t("Your top picks") }
    Flow {
        visible: root.hasPlays
        Layout.fillWidth: true
        spacing: 8
        Repeater {
            model: [
                { text: I18n.t("Tracks"), icon: "music_note" },
                { text: I18n.t("Artists"), icon: "person" },
                { text: I18n.t("Albums"), icon: "album" }
            ]
            Chip {
                required property var modelData
                required property int index
                text: modelData.text
                icon: modelData.icon
                selected: root.topKind === index
                onClicked: root.topKind = index
            }
        }
    }
    ColumnLayout {
        id: topList
        visible: root.hasPlays && items.length > 0
        Layout.fillWidth: true
        spacing: 2
        readonly property var items: root.topKind === 0 ? (root.summary.topTracks ?? [])
                                   : root.topKind === 1 ? (root.summary.topArtists ?? []) : (root.summary.topAlbums ?? [])
        readonly property var first: items.length > 0 ? items[0] : ({})
        function measure(e) { return root.topKind === 0 ? (e.plays ?? 0) : (e.seconds ?? 0) }
        function measureText(e) {
            return root.topKind === 0 ? I18n.n(e.plays ?? 0, "{0} play", "{0} plays") : Extras.formatTime(e.seconds ?? 0)
        }
        function sub(e) {
            if (root.topKind === 1)
                return I18n.n(parseInt(e.subtitle ?? "0", 10) || 0, "{0} song", "{0} songs")
            return e.subtitle ?? ""
        }

        // "Top pick" hero
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 136
            topLeftRadius: 28
            topRightRadius: 28
            bottomLeftRadius: topList.items.length > 1 ? 8 : 28
            bottomRightRadius: bottomLeftRadius
            color: Theme.primaryContainer
            RowLayout {
                anchors.fill: parent
                anchors.margins: 20
                spacing: 16
                MorphImage {
                    Layout.preferredWidth: 96
                    Layout.preferredHeight: 96
                    source: topList.first.cover ?? ""
                    shape: root.topKind === 1 ? "circle" : "cookie9"
                    fallbackKind: root.topKind === 1 ? "artist" : root.topKind === 2 ? "album" : "track"
                    fallbackTitle: topList.first.title ?? ""
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4
                    Rectangle {
                        implicitWidth: pickRow.implicitWidth + 20
                        implicitHeight: 26
                        radius: 13
                        color: Theme.primary
                        Row {
                            id: pickRow
                            anchors.centerIn: parent
                            spacing: 4
                            Icon { name: "star"; size: 16; fill: 1; color: Theme.fgPrimary; anchors.verticalCenter: parent.verticalCenter }
                            Label { text: I18n.t("Top pick"); type: "labelMedium"; weight: 700; color: Theme.fgPrimary; anchors.verticalCenter: parent.verticalCenter }
                        }
                    }
                    Label { Layout.fillWidth: true; text: topList.first.title ?? ""; type: "titleLarge"; weight: 700; color: Theme.fgPrimaryContainer; elide: Text.ElideRight }
                    Label { Layout.fillWidth: true; text: topList.sub(topList.first); type: "bodyMedium"; color: Theme.fgPrimaryContainer; opacity: 0.8; elide: Text.ElideRight }
                    Label { text: topList.measureText(topList.first); type: "labelLarge"; weight: 600; color: Theme.fgPrimaryContainer }
                }
            }
        }
        // ranked rows
        Repeater {
            model: topList.items.slice(1)
            Rectangle {
                id: ranked
                required property var modelData
                required property int index
                readonly property bool last: index === topList.items.length - 2
                Layout.fillWidth: true
                implicitHeight: 68
                topLeftRadius: 8
                topRightRadius: 8
                bottomLeftRadius: last ? 28 : 8
                bottomRightRadius: bottomLeftRadius
                color: Theme.surfaceContainerHigh
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 16
                    anchors.rightMargin: 16
                    spacing: 12
                    Label {
                        Layout.preferredWidth: 22
                        text: String(ranked.index + 2)
                        type: "titleMedium"
                        weight: 700
                        color: Theme.primary
                        horizontalAlignment: Text.AlignHCenter
                    }
                    MorphImage {
                        Layout.preferredWidth: 44
                        Layout.preferredHeight: 44
                        source: ranked.modelData.cover ?? ""
                        shape: root.topKind === 1 ? "circle" : ({ kind: "square", p: 6 })
                        fallbackKind: root.topKind === 1 ? "artist" : root.topKind === 2 ? "album" : "track"
                        fallbackTitle: ranked.modelData.title ?? ""
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 4
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8
                            Label { Layout.fillWidth: true; text: ranked.modelData.title ?? ""; type: "bodyLarge"; weight: 600; elide: Text.ElideRight }
                            Label { text: topList.measureText(ranked.modelData); type: "labelMedium"; color: Theme.fgSurfaceVariant }
                        }
                        // the share of the top pick
                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 6
                            radius: 3
                            color: Theme.surfaceContainerHighest
                            Rectangle {
                                width: parent.width * Math.min(1, topList.measure(ranked.modelData) / Math.max(1, topList.measure(topList.first)))
                                height: parent.height
                                radius: 3
                                color: Theme.primary
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- listening habits ----
    SectionLabel { visible: root.hasPlays; text: I18n.t("Listening habits") }
    GridLayout {
        visible: root.hasPlays
        Layout.fillWidth: true
        columns: 2
        columnSpacing: 12
        rowSpacing: 12
        HabitCard {
            icon: "bedtime"
            label: I18n.t("Peak hour")
            value: root.summary.peakHour === null || root.summary.peakHour === undefined ? "—"
                 : Qt.formatTime(new Date(2000, 0, 1, root.summary.peakHour, 0), "h AP")
            note: I18n.t("When you listen most")
        }
        HabitCard {
            icon: "today"
            label: I18n.t("Busiest day")
            value: root.summary.busiestWeekday === null || root.summary.busiestWeekday === undefined ? "—"
                 : Qt.locale().standaloneDayName((root.summary.busiestWeekday + 1) % 7, Locale.LongFormat)
            note: root.range === "today" ? I18n.t("Today") : I18n.t("Most listening")
        }
        HabitCard {
            icon: "timer"
            label: I18n.t("Longest session")
            value: Extras.formatTime(root.summary.longestSessionSeconds ?? 0)
            note: I18n.t("Without a break of 15 minutes")
        }
        HabitCard {
            icon: "insights"
            label: I18n.t("Active days")
            value: String(root.summary.activeDays ?? 0)
            note: I18n.t("Days with music")
        }
    }

    // ---- listening variety: the top artists' share of the time ----
    Rectangle {
        id: variety
        visible: root.hasPlays && parts.length > 0
        Layout.fillWidth: true
        implicitHeight: varietyCol.implicitHeight + 40
        radius: 28
        color: Theme.surfaceContainerHigh
        readonly property real total: Math.max(1, root.summary.totalSeconds ?? 0)
        readonly property var palette: [Theme.primary, Theme.tertiary, Theme.secondary, Theme.primaryFixedDim, Theme.tertiaryFixedDim]
        readonly property var parts: {
            const artists = (root.summary.topArtists ?? []).slice(0, 4)
            const list = artists.map((a, i) => ({ name: a.title, seconds: a.seconds, color: palette[i] }))
            const rest = total - artists.reduce((sum, a) => sum + a.seconds, 0)
            if (rest > 0)
                list.push({ name: I18n.t("Everyone else"), seconds: rest, color: Theme.outlineVariant })
            return list
        }
        ColumnLayout {
            id: varietyCol
            x: 20
            y: 20
            width: parent.width - 40
            spacing: 14
            Label { Layout.fillWidth: true; text: I18n.t("Listening variety"); type: "titleLarge"; weight: 700 }
            Label {
                Layout.fillWidth: true
                text: I18n.t("{0} artists · {1} tracks", root.summary.distinctArtists ?? 0, root.summary.distinctTracks ?? 0)
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
            }
            // stacked bar: rounded ends, 3 px between parts
            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: 20
                Row {
                    anchors.fill: parent
                    spacing: 3
                    Repeater {
                        model: variety.parts
                        Rectangle {
                            required property var modelData
                            required property int index
                            width: Math.max(4, (parent.width - 3 * (variety.parts.length - 1)) * modelData.seconds / variety.total)
                            height: parent.height
                            radius: index === 0 || index === variety.parts.length - 1 ? 10 : 4
                            color: modelData.color
                        }
                    }
                }
            }
            Flow {
                Layout.fillWidth: true
                spacing: 16
                Repeater {
                    model: variety.parts
                    Row {
                        required property var modelData
                        spacing: 6
                        Rectangle { width: 10; height: 10; radius: 5; color: modelData.color; anchors.verticalCenter: parent.verticalCenter }
                        Label {
                            text: modelData.name + " · " + Math.round(100 * modelData.seconds / variety.total) + "%"
                            type: "labelLarge"
                            color: Theme.fgSurfaceVariant
                        }
                    }
                }
            }
        }
    }

    // ---- empty ----
    Rectangle {
        visible: !root.hasPlays
        Layout.fillWidth: true
        implicitHeight: 120
        radius: 28
        color: Theme.surfaceContainerHigh
        ColumnLayout {
            anchors.centerIn: parent
            spacing: 8
            Icon { Layout.alignment: Qt.AlignHCenter; name: "graphic_eq"; size: 32; color: Theme.primary }
            Label { Layout.alignment: Qt.AlignHCenter; text: I18n.t("Nothing listened in this period yet"); type: "bodyLarge"; color: Theme.fgSurfaceVariant }
        }
    }

    // ---- clear ----
    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: 8
        implicitHeight: 72
        radius: 28
        color: Theme.surfaceContainerHigh
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 20
            anchors.rightMargin: 12
            spacing: 12
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Label { Layout.fillWidth: true; text: I18n.t("Clear listening stats"); type: "titleMedium"; weight: 600 }
                Label { Layout.fillWidth: true; text: I18n.t("Kept on this computer only"); type: "bodyMedium"; color: Theme.fgSurfaceVariant }
            }
            Button {
                text: I18n.t("Clear")
                icon: "delete_sweep"
                style: "tonal"
                size: "xs"
                onClicked: Extras.clearStats()
            }
        }
    }
}
