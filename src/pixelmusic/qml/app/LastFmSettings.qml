import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Settings → Accounts → Last.fm → "Last.fm Integration" (category "lastfm", opened by the Last.fm card's
// Open Service; PixelPlayer-style grouped rows, Material 3 Expressive, colours from Theme.*):
//  Accounts — logged in as <user>, click to log out (asks first), the profile on last.fm
//  Options — scrobbling, now playing, Last.fm shelves on Home
//  Smart Mix Playlists — Home mixes from Last.fm, saved mixes in Library, local playlist sync, retention
//  Scrobbling Configuration — when a listen counts (ScrobbleRule in sf-services lastfm.rs)
ColumnLayout {
    id: root
    spacing: 24

    signal back()

    // Small primary label above a group ("Options")
    component GroupLabel: Label {
        Layout.fillWidth: true
        Layout.leftMargin: 20
        Layout.bottomMargin: -12
        type: "titleSmall"
        weight: 600
        color: Theme.primary
    }

    // A switch row bound to a boolean Settings key
    component SwitchRow: SettingsCategoryRow {
        id: swr
        property string key
        tone: "secondary"
        trailingIcon: ""
        onClicked: Settings[swr.key] = !Settings[swr.key]
        Switch { checked: Settings[swr.key]; onToggled: c => Settings[swr.key] = c }
    }

    // One row of a slider group: "Title: value" with a value chip and a full-width slider below.
    component SliderRow: Rectangle {
        id: sr
        property string title
        property string valueLabel
        property real from: 0
        property real to: 1
        property real step: 1
        property real value: 0
        property bool first: false
        property bool last: false
        signal moved(real value)
        Layout.fillWidth: true
        implicitHeight: srCol.implicitHeight + 40
        topLeftRadius: first ? Theme.shape.extraLarge : Theme.shape.extraSmall
        topRightRadius: topLeftRadius
        bottomLeftRadius: last ? Theme.shape.extraLarge : Theme.shape.extraSmall
        bottomRightRadius: bottomLeftRadius
        color: Theme.surfaceContainerHigh
        opacity: enabled ? 1 : 0.6
        ColumnLayout {
            id: srCol
            x: 24
            y: 20
            width: parent.width - 48
            spacing: 16
            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                Label {
                    Layout.fillWidth: true
                    text: sr.title + ": " + sr.valueLabel
                    type: "titleMedium"
                    weight: 600
                    wrapMode: Text.WordWrap
                    elide: Text.ElideNone
                }
                Rectangle {
                    implicitWidth: chipLabel.implicitWidth + 28
                    implicitHeight: 36
                    radius: 18
                    color: Theme.surfaceContainerLowest
                    Label {
                        id: chipLabel
                        anchors.centerIn: parent
                        text: sr.valueLabel
                        type: "labelLarge"
                        weight: 700
                        color: Theme.primary
                    }
                }
            }
            Slider {
                Layout.fillWidth: true
                wheelEnabled: false     // the wheel scrolls the page
                showValue: false
                from: sr.from
                to: sr.to
                stepSize: sr.step
                value: sr.value
                onMoved: v => sr.moved(v)
            }
        }
    }

    // ---------------- Accounts ----------------
    GroupLabel { text: I18n.t("Accounts") }
    ColumnLayout {
        Layout.fillWidth: true
        spacing: 3
        SettingsCategoryRow {
            id: accountRow
            property bool confirming: false
            first: true
            last: !confirming
            title: LastFm.connected ? I18n.t("Logged in as {0}", LastFm.username || "Last.fm") : I18n.t("Not connected")
            subtitle: LastFm.connected ? (confirming ? I18n.t("Log out of Last.fm on this PC?") : I18n.t("Click to Log Out"))
                                       : I18n.t("Connect Last.fm in Settings → Accounts")
            icon: "person"
            tone: "secondary"
            onClicked: {
                if (LastFm.connected)
                    confirming = !confirming
                else
                    root.back()
            }
        }
        // asks before logging out: scrobbling stops and the saved session is removed
        Rectangle {
            visible: accountRow.confirming && LastFm.connected
            Layout.fillWidth: true
            implicitHeight: confirmRow.implicitHeight + 32
            topLeftRadius: Theme.shape.extraSmall
            topRightRadius: Theme.shape.extraSmall
            bottomLeftRadius: Theme.shape.extraLarge
            bottomRightRadius: Theme.shape.extraLarge
            color: Theme.surfaceContainerHigh
            RowLayout {
                id: confirmRow
                x: 24
                y: 16
                width: parent.width - 48
                spacing: 8
                Label {
                    Layout.fillWidth: true
                    text: I18n.t("Scrobbling stops until you connect again.")
                    type: "bodyMedium"
                    color: Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                    elide: Text.ElideNone
                }
                Button {
                    text: I18n.t("Cancel")
                    style: "text"
                    size: "s"
                    onClicked: accountRow.confirming = false
                }
                Button {
                    text: I18n.t("Log out")
                    icon: "logout"
                    style: "filled"
                    size: "s"
                    onClicked: {
                        accountRow.confirming = false
                        LastFm.disconnect()
                        root.back()
                    }
                }
            }
        }
        Button {
            visible: LastFm.connected
            Layout.leftMargin: 12
            Layout.topMargin: 4
            text: I18n.t("Open profile on Last.fm")
            icon: "open_in_new"
            style: "text"
            size: "s"
            onClicked: Qt.openUrlExternally("https://www.last.fm/user/" + encodeURIComponent(LastFm.username))
        }
    }

    // ---------------- Options ----------------
    GroupLabel { text: I18n.t("Options") }
    ColumnLayout {
        Layout.fillWidth: true
        spacing: 3
        SwitchRow {
            first: true
            key: "lastfmScrobble"
            title: I18n.t("Enable Scrobbling")
            subtitle: I18n.t("Scrobble played songs to your Last.fm profile")
            icon: "music_note"
        }
        SwitchRow {
            key: "lastfmNowPlaying"
            title: I18n.t("Update Now Playing")
            subtitle: I18n.t("Update your Last.fm status with the current track")
            icon: "touch_app"
        }
        SwitchRow {
            last: true
            key: "lastfmExplore"
            title: I18n.t("Explore recommendations with Last.fm")
            subtitle: I18n.t("Use your Last.fm listening history, top albums, and friends to personalize the Home page.")
            icon: "travel_explore"
        }
    }

    // ---------------- Smart Mix Playlists ----------------
    GroupLabel { text: I18n.t("Smart Mix Playlists") }
    ColumnLayout {
        Layout.fillWidth: true
        spacing: 3
        SwitchRow {
            first: true
            key: "lastfmSmartMix"
            title: I18n.t("Last.fm Smart Mix")
            subtitle: I18n.t("Use Last.fm recommendations for the Home mixes instead of your local listening history.")
            icon: "auto_awesome"
        }
        SwitchRow {
            key: "lastfmShowMixes"
            title: I18n.t("Show Smart Mix playlists in Library")
            subtitle: I18n.t("Display or hide saved Last.fm Smart Mix playlists on the Library playlists page.")
            icon: "queue_music"
        }
        SwitchRow {
            // local playlists (library/local.rs) mirror their edits to a linked YouTube Music playlist
            key: "localPlaylistSync"
            title: I18n.t("Sync local playlists to YouTube Music")
            subtitle: I18n.t("Sync local playlist edits back to your YouTube Music account.")
            icon: "public"
        }
        // retention: a radio group of chips under the row text
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: retCol.implicitHeight + 40
            topLeftRadius: Theme.shape.extraSmall
            topRightRadius: Theme.shape.extraSmall
            bottomLeftRadius: Theme.shape.extraLarge
            bottomRightRadius: Theme.shape.extraLarge
            color: Theme.surfaceContainerHigh
            RowLayout {
                id: retRow
                x: 20
                y: 20
                width: parent.width - 44
                spacing: 16
                Rectangle {
                    Layout.alignment: Qt.AlignTop
                    implicitWidth: 48
                    implicitHeight: 48
                    radius: 24
                    color: Theme.secondaryContainer
                    Icon { anchors.centerIn: parent; name: "timer"; size: 24; color: Theme.fgSecondaryContainer }
                }
                ColumnLayout {
                    id: retCol
                    Layout.fillWidth: true
                    spacing: 4
                    Label { Layout.fillWidth: true; text: I18n.t("Generated Playlists Retention"); type: "titleMedium"; weight: 600 }
                    Label {
                        Layout.fillWidth: true
                        text: I18n.t("Choose how long to keep saved Last.fm mix playlists in your library.")
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                        wrapMode: Text.WordWrap
                        elide: Text.ElideNone
                    }
                    Flow {
                        Layout.fillWidth: true
                        Layout.topMargin: 8
                        spacing: 8
                        Repeater {
                            model: [
                                { days: 0, text: I18n.t("Permanently Store") },
                                { days: 7, text: I18n.t("7 days") },
                                { days: 30, text: I18n.t("30 days") },
                                { days: 90, text: I18n.t("90 days") }
                            ]
                            Chip {
                                required property var modelData
                                text: modelData.text
                                selected: Settings.lastfmMixRetentionDays === modelData.days
                                onClicked: Settings.lastfmMixRetentionDays = modelData.days
                            }
                        }
                    }
                }
            }
        }
    }

    // ---------------- Scrobbling Configuration ----------------
    GroupLabel { text: I18n.t("Scrobbling Configuration") }
    ColumnLayout {
        enabled: Settings.lastfmScrobble
        Layout.fillWidth: true
        spacing: 3
        SliderRow {
            first: true
            title: I18n.t("Minimum Track Duration")
            valueLabel: I18n.t("{0}s", Settings.lastfmMinTrackS)
            from: 0
            to: 120
            step: 5
            value: Settings.lastfmMinTrackS
            onMoved: v => Settings.lastfmMinTrackS = Math.round(v)
        }
        SliderRow {
            title: I18n.t("Scrobble Threshold Percentage")
            valueLabel: Settings.lastfmThresholdPct + "%"
            from: 25
            to: 100
            step: 5
            value: Settings.lastfmThresholdPct
            onMoved: v => Settings.lastfmThresholdPct = Math.round(v)
        }
        SliderRow {
            last: true
            title: I18n.t("Max Scrobble Threshold Duration")
            valueLabel: I18n.t("{0}s", Settings.lastfmMaxThresholdS)
            from: 30
            to: 600
            step: 10
            value: Settings.lastfmMaxThresholdS
            onMoved: v => Settings.lastfmMaxThresholdS = Math.round(v)
        }
    }
}
