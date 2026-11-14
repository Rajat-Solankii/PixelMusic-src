pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

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

            Rectangle { anchors.fill: parent; radius: Theme.shape.extraLarge; color: Theme.surfaceContainerHigh }
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

    // ---- Recently played: PixelPlayer Desktop's RecentlyPlayedSection ----
    // Three rows of 68 px pills, each as wide as its text needs (190…310 px in five steps), filled column by column
    // and scrolled sideways together. Each pill is its cover's primaryContainer (CoverTones: Tonal Spot, light or
    // dark) with the title in onPrimaryContainer and the artist at 80 %; the playing song's pill squares its corners
    // (34 → 18 px, 280 ms). The header is the target's: a large title and a 72 × 44 arrow button.
    ColumnLayout {
        id: recentSection
        readonly property real pillHeight: 68
        readonly property real gap: 12
        readonly property int rowCount: 3
        readonly property var widthSteps: [190, 220, 250, 280, 310]
        // until a cover's own colours arrive (CoverTones) — or when the cover cannot be read — the theme's container
        readonly property var fallback: [Theme.primaryContainer, Theme.fgPrimaryContainer, Qt.alpha(Theme.fgPrimaryContainer, 0.8)]

        function pillWidth(t) {
            const w = (t.title ?? "").trim().length + (t.artists ?? "").trim().length * 0.55
            return widthSteps[w < 18 ? 0 : w < 28 ? 1 : w < 40 ? 2 : w < 54 ? 3 : 4]
        }
        // as many songs as fill the width plus two columns to scroll to (at least five columns), dealt column by
        // column over the three rows like the target (the first rows take the remainder)
        readonly property var rows: {
            const songs = root.recent
            if (songs.length === 0 || width <= 0)
                return []
            const columns = Math.max(5, Math.ceil((width + gap) / (widthSteps[0] + gap)) + 2)
            const shown = songs.slice(0, columns * rowCount)
            const base = Math.floor(shown.length / rowCount), rest = shown.length % rowCount
            const targets = [base + (rest > 0 ? 1 : 0), base + (rest > 1 ? 1 : 0), base]
            const out = [[], [], []]
            let i = 0
            for (let c = 0; c < targets[0]; c++)
                for (let r = 0; r < rowCount && i < shown.length; r++)
                    if (c < targets[r])
                        out[r].push({ index: i, track: shown[i++] })
            return out
        }

        visible: root.recent.length > 0
        Layout.fillWidth: true
        Layout.topMargin: 12
        spacing: 14

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 4
            Layout.rightMargin: 4
            spacing: 8
            Label {
                Layout.fillWidth: true
                text: I18n.t("Recently played")
                type: "headlineSmall"
                font.pixelSize: root.compact ? 22 : 26
                weight: 600
                fade: true
            }
            Item {
                Layout.preferredWidth: root.compact ? 60 : 72
                Layout.preferredHeight: root.compact ? 38 : 44
                Rectangle { anchors.fill: parent; radius: height / 2; color: Theme.surfaceContainerHigh }
                Icon { anchors.centerIn: parent; name: "arrow_forward"; size: root.compact ? 20 : 22; color: Theme.fgSurface }
                StateLayer { anchors.fill: parent; radius: height / 2; onClicked: root.seeAll("recent") }
            }
        }

        Flickable {
            id: pillScroll
            Layout.fillWidth: true
            Layout.preferredHeight: recentSection.rowCount * recentSection.pillHeight + (recentSection.rowCount - 1) * recentSection.gap
            contentWidth: pillRows.width
            contentHeight: height
            flickableDirection: Flickable.HorizontalFlick
            SideScroll { flickable: pillScroll }
            boundsBehavior: Flickable.StopAtBounds
            clip: true

            Column {
                id: pillRows
                spacing: recentSection.gap
                Repeater {
                    model: recentSection.rows
                    Row {
                        id: pillRow
                        required property var modelData
                        spacing: recentSection.gap
                        height: recentSection.pillHeight
                        Repeater {
                            model: pillRow.modelData
                            Item {
                                id: pill
                                required property var modelData
                                readonly property var track: modelData.track
                                readonly property bool current: (track.trackId ?? "") !== "" && track.trackId === Player.trackId
                                // the cover's container tone and the text colours made for it, light or dark
                                readonly property var coverTones: CoverTones.revision, CoverTones.tones(track.cover ?? "", Theme.dark)
                                readonly property color container: coverTones.container ?? recentSection.fallback[0]
                                readonly property color fgTitle: coverTones.onContainer ?? recentSection.fallback[1]
                                readonly property color fgArtist: coverTones.onContainerVariant ?? recentSection.fallback[2]
                                width: recentSection.pillWidth(track)
                                height: recentSection.pillHeight
                                property real corner: current ? 18 : height / 2
                                Behavior on corner { enabled: !Theme.reducedMotion; NumberAnimation { duration: 280; easing.type: Easing.InOutQuad } }

                                Rectangle {
                                    anchors.fill: parent
                                    radius: pill.corner
                                    color: pill.container
                                    Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: 280 } }
                                }
                                StateLayer {
                                    anchors.fill: parent
                                    radius: pill.corner
                                    color: pill.fgTitle
                                    onClicked: Player.playFrom(Home.recentTracks, pill.modelData.index)
                                }
                                TapHandler {
                                    acceptedButtons: Qt.RightButton
                                    onTapped: point => root.trackMenu(pill.track, pill, point.position.x, point.position.y)
                                }
                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 9
                                    anchors.rightMargin: 16
                                    spacing: 12
                                    MorphImage {
                                        Layout.preferredWidth: 50
                                        Layout.preferredHeight: 50
                                        shape: "circle"
                                        source: pill.track.cover ?? ""
                                    }
                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 0
                                        Label {
                                            Layout.fillWidth: true
                                            text: pill.track.title ?? ""
                                            type: "titleMedium"
                                            font.pixelSize: 17
                                            weight: 600
                                            color: pill.fgTitle
                                            Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: 280 } }
                                        }
                                        Label {
                                            Layout.fillWidth: true
                                            text: pill.track.artists ?? ""
                                            type: "bodySmall"
                                            font.pixelSize: 14
                                            color: pill.fgArtist
                                            Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: 280 } }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
