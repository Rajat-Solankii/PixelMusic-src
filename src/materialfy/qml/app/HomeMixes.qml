import QtQuick
import QtQuick.Layouts
import Md3
import Materialfy.Core

// Home: "Your Mix" and "Daily Mix" cards and the "Recently played" pills, as on the SoundWave Android home.
// Everything comes from the local listening history (Home.yourMix / dailyMix / recentlyPlayed, home/mixes.rs);
// HomePage shows this block only when something was played. Playback goes through the track models
// (Home.*Tracks), so the queue is the mix itself.
ColumnLayout {
    id: root

    property bool compact: false
    signal seeAll(string mix)          // "daily" | "recent"

    readonly property var daily: Home.dailyMix ?? []
    readonly property var your: Home.yourMix ?? []
    readonly property var recent: Home.recentlyPlayed ?? []
    // covers of a mix, each once
    function covers(tracks, count) {
        const out = []
        for (const t of tracks) {
            const c = t.cover ?? ""
            if (c !== "" && out.indexOf(c) < 0)
                out.push(c)
            if (out.length >= count)
                break
        }
        return out
    }
    function mmss(ms) {
        const s = Math.max(0, Math.round((ms ?? 0) / 1000))
        return s <= 0 ? "" : Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
    }
    function trackMenu(track, item, x, y) {
        const p = item.mapToItem(null, x, y)
        Catalog.requestTrackMenu(track, p.x, p.y)
    }

    spacing: 12

    GridLayout {
        Layout.fillWidth: true
        Layout.topMargin: 8
        columns: root.compact ? 1 : 2
        columnSpacing: 16
        rowSpacing: 16

        // ---- Your Mix: the whole card plays the mix shuffled ----
        Item {
            id: yourCard
            readonly property var art: root.covers(root.your, 3)
            visible: root.your.length > 0
            Layout.fillWidth: root.compact
            Layout.fillHeight: !root.compact
            Layout.preferredWidth: 248
            Layout.preferredHeight: root.compact ? 220 : -1

            Rectangle { anchors.fill: parent; radius: Theme.shape.extraLarge; color: Theme.surfaceContainerHigh }
            StateLayer {
                id: yourLayer
                anchors.fill: parent
                radius: Theme.shape.extraLarge
                onClicked: Player.shuffleFrom(Home.yourMixTracks)
            }
            Column {
                x: 20
                y: 18
                width: parent.width - 96
                spacing: 2
                Label { width: parent.width; text: I18n.t("Your Mix"); type: "headlineSmall"; weight: 700; fade: true }
                Label { width: parent.width; text: I18n.t("Today's mix for you"); type: "bodySmall"; color: Theme.fgSurfaceVariant; fade: true }
            }
            IconButton {
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 14
                icon: "shuffle"
                style: "filled"
                size: "s"
                onClicked: Player.shuffleFrom(Home.yourMixTracks)
            }
            // collage: one large cover and two round ones over its corners
            Item {
                id: collage
                readonly property real side: Math.max(96, Math.min(parent.width - 96, parent.height - 132))
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 34
                width: side
                height: side
                MorphImage {
                    anchors.fill: parent
                    rotation: -6
                    shape: "softSquare"
                    source: yourCard.art[0] ?? ""
                }
                MorphImage {
                    visible: yourCard.art.length > 1
                    x: -width * 0.45
                    y: -height * 0.35
                    width: Math.round(collage.side * 0.44)
                    height: width
                    shape: "circle"
                    source: yourCard.art[1] ?? ""
                }
                MorphImage {
                    visible: yourCard.art.length > 2
                    x: collage.side - width * 0.55
                    y: collage.side - height * 0.7
                    width: Math.round(collage.side * 0.4)
                    height: width
                    shape: "circle"
                    source: yourCard.art[2] ?? ""
                }
            }
        }

        // ---- Daily Mix: header, the first four songs, "see all" ----
        Item {
            id: dailyCard
            readonly property var rows: root.daily.slice(0, 4)
            readonly property var art: root.covers(root.daily.slice(4).concat(root.daily), 3)
            visible: root.daily.length > 0
            Layout.fillWidth: true
            Layout.preferredHeight: dailyCol.implicitHeight

            Rectangle { anchors.fill: parent; radius: Theme.shape.extraLarge; color: Theme.surfaceContainer }
            Column {
                id: dailyCol
                width: parent.width

                Rectangle {
                    width: parent.width
                    height: 76
                    topLeftRadius: Theme.shape.extraLarge
                    topRightRadius: Theme.shape.extraLarge
                    gradient: Gradient {
                        orientation: Gradient.Horizontal
                        GradientStop { position: 0; color: Theme.primaryContainer }
                        GradientStop { position: 1; color: Theme.tertiaryContainer }
                    }
                    StateLayer {
                        anchors.fill: parent
                        topLeftRadius: Theme.shape.extraLarge
                        topRightRadius: Theme.shape.extraLarge
                        color: Theme.fgPrimaryContainer
                        onClicked: root.seeAll("daily")
                    }
                    Column {
                        anchors.verticalCenter: parent.verticalCenter
                        x: 20
                        width: parent.width - 40 - stack.width - 12
                        spacing: 2
                        Label {
                            width: parent.width
                            text: I18n.t("DAILY MIX")
                            type: "titleLarge"
                            weight: 800
                            color: Theme.fgPrimaryContainer
                            fade: true
                        }
                        Label {
                            width: parent.width
                            text: I18n.t("Based on your history")
                            type: "bodySmall"
                            color: Theme.fgPrimaryContainer
                            opacity: 0.8
                            fade: true
                        }
                    }
                    Row {
                        id: stack
                        anchors.right: parent.right
                        anchors.rightMargin: 16
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: -14
                        Repeater {
                            model: dailyCard.art
                            MorphImage {
                                required property var modelData
                                required property int index
                                width: 44
                                height: 44
                                z: index
                                shape: "softSquare"
                                source: modelData
                            }
                        }
                    }
                }

                Repeater {
                    model: dailyCard.rows
                    Item {
                        id: mixRow
                        required property var modelData
                        required property int index
                        readonly property bool current: (modelData.trackId ?? "") !== "" && modelData.trackId === Player.trackId
                        width: dailyCol.width
                        height: 56
                        StateLayer {
                            anchors.fill: parent
                            onClicked: Player.playFrom(Home.dailyMixTracks, mixRow.index)
                        }
                        TapHandler {
                            acceptedButtons: Qt.RightButton
                            onTapped: point => root.trackMenu(mixRow.modelData, mixRow, point.position.x, point.position.y)
                        }
                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 16
                            anchors.rightMargin: 8
                            spacing: 12
                            MorphImage {
                                Layout.preferredWidth: 40
                                Layout.preferredHeight: 40
                                shape: "softSquare"
                                source: mixRow.modelData.cover ?? ""
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                Label {
                                    Layout.fillWidth: true
                                    text: mixRow.modelData.title ?? ""
                                    type: "titleSmall"
                                    color: mixRow.current ? Theme.primary : Theme.fgSurface
                                    fade: true
                                }
                                Label {
                                    Layout.fillWidth: true
                                    text: mixRow.modelData.artists ?? ""
                                    type: "bodySmall"
                                    color: Theme.fgSurfaceVariant
                                    fade: true
                                }
                            }
                            Label {
                                text: root.mmss(mixRow.modelData.durationMs)
                                type: "bodySmall"
                                color: Theme.fgSurfaceVariant
                            }
                            IconButton {
                                id: more
                                icon: "more_horiz"
                                size: "xs"
                                onClicked: {
                                    const p = more.mapToItem(null, 0, 0)
                                    Catalog.requestTrackMenuAt(mixRow.modelData, p.x, p.y, more.width, more.height)
                                }
                            }
                        }
                    }
                }

                Item {
                    width: dailyCol.width
                    height: 48
                    StateLayer {
                        anchors.fill: parent
                        bottomLeftRadius: Theme.shape.extraLarge
                        bottomRightRadius: Theme.shape.extraLarge
                        onClicked: root.seeAll("daily")
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        x: 20
                        width: parent.width - 64
                        text: I18n.t("See all of Daily Mix")
                        type: "labelLarge"
                        fade: true
                    }
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.right: parent.right
                        anchors.rightMargin: 16
                        name: "arrow_forward"
                        size: 20
                    }
                }
            }
        }
    }

    // ---- Recently played: two rows of pills ----
    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: 8
        spacing: 8
        Label {
            Layout.fillWidth: true
            text: I18n.t("Recently played")
            type: "headlineSmall"
            weight: 600
            fade: true
        }
        IconButton {
            icon: "arrow_forward"
            size: "xs"
            onClicked: root.seeAll("recent")
        }
    }
    GridLayout {
        id: pills
        readonly property int perRow: Math.max(1, Math.min(4, Math.floor((root.width + 12) / 292)))
        // the pills take the tones of the theme in turn (the app does not read colours out of covers)
        readonly property var tones: [
            [Theme.secondaryContainer, Theme.fgSecondaryContainer],
            [Theme.tertiaryContainer, Theme.fgTertiaryContainer],
            [Theme.primaryContainer, Theme.fgPrimaryContainer],
            [Theme.surfaceContainerHighest, Theme.fgSurface]
        ]
        Layout.fillWidth: true
        columns: perRow
        columnSpacing: 12
        rowSpacing: 12
        Repeater {
            model: root.recent.slice(0, pills.perRow * 2)
            Item {
                id: pill
                required property var modelData
                required property int index
                readonly property var tone: pills.tones[index % pills.tones.length]
                Layout.fillWidth: true
                Layout.preferredWidth: 280
                Layout.preferredHeight: 60
                Rectangle { anchors.fill: parent; radius: height / 2; color: pill.tone[0] }
                StateLayer {
                    anchors.fill: parent
                    radius: height / 2
                    color: pill.tone[1]
                    onClicked: Player.playFrom(Home.recentTracks, pill.index)
                }
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: point => root.trackMenu(pill.modelData, pill, point.position.x, point.position.y)
                }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 20
                    spacing: 12
                    MorphImage {
                        Layout.preferredWidth: 44
                        Layout.preferredHeight: 44
                        shape: "circle"
                        source: pill.modelData.cover ?? ""
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Label {
                            Layout.fillWidth: true
                            text: pill.modelData.title ?? ""
                            type: "titleSmall"
                            color: pill.tone[1]
                            fade: true
                        }
                        Label {
                            Layout.fillWidth: true
                            text: pill.modelData.artists ?? ""
                            type: "bodySmall"
                            color: pill.tone[1]
                            opacity: 0.75
                            fade: true
                        }
                    }
                }
            }
        }
    }
}
