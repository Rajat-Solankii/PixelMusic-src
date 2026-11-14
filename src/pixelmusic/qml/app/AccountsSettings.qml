import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Settings → Accounts (PixelPlayer-style, Material 3 Expressive):
//  1. Connected Accounts summary with Active / Available tiles
//  2. YouTube Music — a solid card: account with a status chip and an expand button, the sync options as an
//     M3 list group (icon, text, switch), Sync Library Now (Library.refresh) and Disconnect; signed out —
//     browser sign-in or pasted cookies
//  3. Quick Import — Spotify / YouTube link → Library.importPlaylist (matched to YouTube Music songs)
//  4. Linked Services — the Last.fm card (connect with the user's own API key, or Open Service → the
//     Last.fm Integration page with its options and scrobbling thresholds / Log out)
//  5. Available Services — what is not connected yet, plus services that are not built yet
// Every container is a solid colour from the dynamic theme (Theme.*); only the YouTube logo keeps its brand red.
ColumnLayout {
    id: root

    // SettingsPage.cardFor: section key → card to scroll to
    readonly property var cards: ({ account: youtubeCard })

    readonly property int activeCount: (Auth.youtubeSignedIn ? 1 : 0) + (LastFm.connected ? 1 : 0)
    readonly property int availableCount: 2     // YouTube Music, Last.fm
    readonly property bool narrow: width < 560

    spacing: 24

    // Open Service on the Last.fm card → SettingsPage opens the "lastfm" category
    signal openLastFm()

    // ---------------- building blocks ----------------

    // A full-width pill button: filled (color/content) or outlined in the content colour.
    component PillButton: Rectangle {
        id: pill
        property string text
        property string icon
        property color content: Theme.fgPrimary
        property bool outlined: false
        property bool busy: false
        signal clicked()
        Layout.fillWidth: true
        implicitHeight: 56
        implicitWidth: pillRow.implicitWidth + 48
        radius: pillArea.pressed ? 16 : height / 2
        Behavior on radius { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack } }
        color: outlined ? "transparent" : Theme.primary
        border.width: outlined ? 1.5 : 0
        border.color: pill.content === Theme.error ? Theme.error : Theme.outline
        opacity: enabled ? 1 : 0.5
        Row {
            id: pillRow
            anchors.centerIn: parent
            spacing: 12
            LoadingIndicator { visible: pill.busy; size: 24; anchors.verticalCenter: parent.verticalCenter }
            Icon {
                visible: pill.icon !== "" && !pill.busy
                name: pill.icon
                size: 24
                tight: true
                color: pill.content
                anchors.verticalCenter: parent.verticalCenter
            }
            Label { text: pill.text; type: "titleMedium"; weight: 600; color: pill.content; anchors.verticalCenter: parent.verticalCenter }
        }
        StateLayer {
            id: pillArea
            anchors.fill: parent
            enabled: pill.enabled && !pill.busy
            radius: pill.radius
            color: pill.content
            onClicked: pill.clicked()
        }
    }

    // One item of an M3 list group: the group's rows are separate solid containers 2 px apart, large corners at the
    // ends of the group and small ones between (Settings rows)
    component SyncRow: Rectangle {
        id: row
        property string icon
        property string title
        property string text
        property bool checked: false
        property bool first: false
        property bool last: false
        signal toggled(bool checked)
        Layout.fillWidth: true
        implicitHeight: Math.max(72, rowLayout.implicitHeight + 32)
        topLeftRadius: first ? 20 : 4
        topRightRadius: topLeftRadius
        bottomLeftRadius: last ? 20 : 4
        bottomRightRadius: bottomLeftRadius
        color: Theme.surfaceContainerLowest
        StateLayer {
            anchors.fill: parent
            topLeftRadius: row.topLeftRadius
            topRightRadius: row.topRightRadius
            bottomLeftRadius: row.bottomLeftRadius
            bottomRightRadius: row.bottomRightRadius
            color: Theme.fgSurface
            onClicked: row.toggled(!row.checked)
        }
        RowLayout {
            id: rowLayout
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            spacing: 16
            Rectangle {
                Layout.preferredWidth: 40
                Layout.preferredHeight: 40
                Layout.alignment: Qt.AlignVCenter
                radius: 20
                color: row.checked ? Theme.secondaryContainer : Theme.surfaceContainerHighest
                Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsDefault } }
                Icon {
                    anchors.centerIn: parent
                    name: row.icon
                    size: 22
                    fill: row.checked ? 1 : 0
                    color: row.checked ? Theme.fgSecondaryContainer : Theme.fgSurfaceVariant
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Label { Layout.fillWidth: true; text: row.title; type: "titleMedium"; weight: 600; color: Theme.fgSurface }
                Label {
                    Layout.fillWidth: true
                    text: row.text
                    type: "bodyMedium"
                    color: Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                    elide: Text.ElideNone
                }
            }
            Switch {
                Layout.alignment: Qt.AlignVCenter
                checked: row.checked
                onToggled: c => row.toggled(c)
            }
        }
    }

    // Big section title ("Linked Services")
    component SectionTitle: Label {
        Layout.fillWidth: true
        Layout.topMargin: 8
        Layout.leftMargin: 4
        type: "headlineSmall"
        weight: 600
    }
    // Small primary label above a group ("Options")
    component GroupLabel: Label {
        Layout.fillWidth: true
        Layout.leftMargin: 20
        Layout.bottomMargin: -12
        type: "titleSmall"
        weight: 600
        color: Theme.primary
    }

    // ================= 1. Connected Accounts =================
    Rectangle {
        Layout.fillWidth: true
        implicitHeight: summaryCol.implicitHeight + 48
        radius: Theme.shape.extraLarge
        color: Theme.surfaceContainerHigh
        ColumnLayout {
            id: summaryCol
            x: 24
            y: 24
            width: parent.width - 48
            spacing: 8
            Label { Layout.fillWidth: true; text: I18n.t("Connected Accounts"); type: "headlineSmall"; weight: 600 }
            Label {
                Layout.fillWidth: true
                text: I18n.t("Manage linked providers and keep each integration under your control.")
                type: "bodyLarge"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 8
                spacing: 12
                Repeater {
                    model: [
                        { label: I18n.t("Active"), value: root.activeCount },
                        { label: I18n.t("Available"), value: root.availableCount }
                    ]
                    Rectangle {
                        required property var modelData
                        Layout.fillWidth: true
                        implicitHeight: 88
                        radius: Theme.shape.large + 4
                        color: Theme.surfaceContainerHighest
                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.leftMargin: 20
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 0
                            Label { text: modelData.label; type: "titleMedium"; color: Theme.fgSurfaceVariant }
                            Label { text: String(modelData.value); type: "headlineMedium"; weight: 600 }
                        }
                    }
                }
            }
        }
    }

    // ================= 2. YouTube Music =================
    // Its own account, independent of everything else: sign-in in an Edge/Chrome window, or pasted cookies.
    Rectangle {
        id: youtubeCard
        readonly property string st: Auth.youtubeState
        readonly property bool busy: st === "awaitingBrowser" || st === "verifying" || st === "checking"
        property bool expanded: false
        readonly property color fg: Theme.fgSurface
        readonly property color fgSoft: Theme.fgSurfaceVariant
        readonly property bool syncing: Library.pagesTotal > 0 && Library.pagesLoaded < Library.pagesTotal

        Layout.fillWidth: true
        implicitHeight: ytCol.implicitHeight + 48
        radius: Theme.shape.extraLarge + 4
        color: Theme.surfaceContainerHigh

        ColumnLayout {
            id: ytCol
            x: 24
            y: 24
            width: parent.width - 48
            spacing: 16

            // header: logo, account, Linked/Options pill (the whole row expands when signed in)
            Item {
                Layout.fillWidth: true
                implicitHeight: ytHeader.implicitHeight
                MouseArea {
                    anchors.fill: parent
                    enabled: Auth.youtubeSignedIn
                    cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                    onClicked: youtubeCard.expanded = !youtubeCard.expanded
                }
                RowLayout {
                    id: ytHeader
                    width: parent.width
                    spacing: 16
                    Item {
                        Layout.preferredWidth: 64
                        Layout.preferredHeight: 64
                        Layout.alignment: Qt.AlignTop
                        Rectangle {
                            anchors.centerIn: parent
                            width: 64
                            height: 46
                            radius: 14
                            color: "#FF0000"
                            Icon { anchors.centerIn: parent; name: "play_arrow"; size: 34; fill: 1; color: "white" }
                        }
                        // the account picture, small, on the logo's corner
                        MorphImage {
                            visible: Auth.youtubeSignedIn && (Auth.youtubeAccount.avatar ?? "") !== ""
                            width: 26
                            height: 26
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            anchors.rightMargin: -6
                            anchors.bottomMargin: -2
                            source: Auth.youtubeAccount.avatar ?? ""
                            shape: "circle"
                        }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        Label {
                            Layout.fillWidth: true
                            text: Auth.youtubeSignedIn ? (Auth.youtubeAccount.name || I18n.t("YouTube Music")) : I18n.t("YouTube Music")
                            type: "headlineSmall"
                            weight: 700
                            color: youtubeCard.fg
                        }
                        Label {
                            Layout.fillWidth: true
                            text: Auth.youtubeSignedIn
                                ? (youtubeCard.expanded ? I18n.t("Tap to hide sync & personalization")
                                                        : I18n.t("Connected · Tap to configure sync"))
                                : youtubeCard.st === "awaitingBrowser" ? I18n.t("Sign in in the browser window that opened")
                                : youtubeCard.busy ? I18n.t("Signing in…")
                                : I18n.t("Sign in to see your YouTube Music playlists, likes and library")
                            type: "bodyLarge"
                            color: youtubeCard.fgSoft
                            wrapMode: Text.WordWrap
                            elide: Text.ElideNone
                        }
                    }
                    LoadingIndicator {
                        visible: youtubeCard.busy
                        Layout.alignment: Qt.AlignTop
                        size: 32
                    }
                    // status chip ("Connected") and the tonal expand button of the options
                    Rectangle {
                        visible: Auth.youtubeSignedIn && !root.narrow
                        Layout.alignment: Qt.AlignVCenter
                        implicitWidth: linkedRow.implicitWidth + 28
                        implicitHeight: 32
                        radius: 16
                        color: Theme.primaryContainer
                        Row {
                            id: linkedRow
                            anchors.centerIn: parent
                            spacing: 6
                            Icon { name: "check_circle"; size: 18; fill: 1; color: Theme.fgPrimaryContainer; anchors.verticalCenter: parent.verticalCenter }
                            Label {
                                text: I18n.t("Connected")
                                type: "labelLarge"
                                weight: 600
                                color: Theme.fgPrimaryContainer
                                anchors.verticalCenter: parent.verticalCenter
                            }
                        }
                    }
                    IconButton {
                        visible: Auth.youtubeSignedIn
                        Layout.alignment: Qt.AlignVCenter
                        icon: "expand_more"
                        style: "tonal"
                        checkable: true
                        autoToggle: false
                        checked: youtubeCard.expanded
                        rotation: youtubeCard.expanded ? 180 : 0
                        Behavior on rotation { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutCubic } }
                        onClicked: youtubeCard.expanded = !youtubeCard.expanded
                    }
                }
            }

            SettingsBanner {
                kind: "error"
                shown: Auth.youtubeError !== "" && !youtubeCard.busy
                title: I18n.t("Couldn’t sign in to YouTube Music")
                text: Auth.youtubeError
                actionText: I18n.t("Retry")
                actionIcon: "refresh"
                onAction: Auth.startYouTubeLogin()
            }

            // --- Synchronization & Personalization (signed in, expanded) ---
            Item {
                Layout.fillWidth: true
                implicitHeight: youtubeCard.expanded && Auth.youtubeSignedIn ? syncCol.implicitHeight : 0
                Behavior on implicitHeight { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.motion.emphasized } }
                opacity: youtubeCard.expanded ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsDefault } }
                visible: implicitHeight > 0.5
                clip: true
                ColumnLayout {
                    id: syncCol
                    width: parent.width
                    spacing: 2
                    Label {
                        Layout.fillWidth: true
                        Layout.topMargin: 4
                        Layout.leftMargin: 4
                        text: I18n.t("Synchronization & Personalization")
                        type: "titleSmall"
                        weight: 600
                        color: Theme.primary
                    }
                    // each row is a real setting the services follow (sf-services lib.rs follow_youtube_account_use,
                    // history.rs for listening activity)
                    Repeater {
                        model: [
                            { key: "youtubeLibrarySync", icon: "library_music", title: I18n.t("Sync Playlists & Liked Songs"),
                              text: I18n.t("Sync YouTube Music playlists and likes with your library.") },
                            { key: "youtubeHistorySync", icon: "history", title: I18n.t("Sync Listening Activity"),
                              text: I18n.t("Share played tracks to your YouTube Music account history, and show that history in Recently played.") },
                            { key: "youtubePersonalHome", icon: "travel_explore", title: I18n.t("Personalized Explore Feed"),
                              text: I18n.t("When off, loads Home and Explore anonymously without account history.") },
                            { key: "youtubePersonalRadio", icon: "radio", title: I18n.t("Personalized Queue & Radios"),
                              text: I18n.t("When off, builds auto-play recommendations purely based on the current song.") }
                        ]
                        SyncRow {
                            required property var modelData
                            required property int index
                            first: index === 0
                            last: index === 3
                            icon: modelData.icon
                            title: modelData.title
                            text: modelData.text
                            checked: Settings[modelData.key]
                            onToggled: c => Settings[modelData.key] = c
                        }
                    }
                }
            }


            // --- signed in: Sync Library Now + Disconnect ---
            PillButton {
                visible: Auth.youtubeSignedIn
                text: youtubeCard.syncing ? I18n.t("Syncing library… {0}/{1}", Library.pagesLoaded, Library.pagesTotal)
                                          : I18n.t("Sync Library Now")
                icon: "sync"
                busy: youtubeCard.syncing
                color: Theme.primary
                content: Theme.fgPrimary
                onClicked: Library.refresh()
            }
            PillButton {
                visible: Auth.youtubeSignedIn
                text: I18n.t("Disconnect")
                icon: "logout"
                outlined: true
                content: Theme.error
                onClicked: Auth.youtubeSignOut()
            }

            // --- signed out: browser sign-in, cookie fallback ---
            RowLayout {
                visible: !Auth.youtubeSignedIn
                Layout.fillWidth: true
                spacing: 12
                PillButton {
                    visible: !youtubeCard.busy
                    text: I18n.t("Sign in")
                    icon: "open_in_new"
                    color: Theme.primary
                    content: Theme.fgPrimary
                    onClicked: Auth.startYouTubeLogin()
                }
                PillButton {
                    visible: youtubeCard.busy && youtubeCard.st !== "checking"
                    text: I18n.t("Cancel")
                    outlined: true
                    content: youtubeCard.fg
                    onClicked: Auth.cancelYouTubeLogin()
                }
            }

            // Without Edge/Chrome: the Cookie header of music.youtube.com
            ColumnLayout {
                id: cookieRow
                property bool editing: false
                visible: !Auth.youtubeSignedIn
                Layout.fillWidth: true
                spacing: 12
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        Label { Layout.fillWidth: true; text: I18n.t("Sign in with cookies"); type: "titleMedium"; weight: 700; color: youtubeCard.fg }
                        Label {
                            Layout.fillWidth: true
                            text: I18n.t("For computers without Edge or Chrome: paste the Cookie header of music.youtube.com from your browser’s developer tools")
                            type: "bodyMedium"
                            color: youtubeCard.fgSoft
                            wrapMode: Text.WordWrap
                            elide: Text.ElideNone
                        }
                    }
                    PillButton {
                        visible: !cookieRow.editing
                        Layout.fillWidth: false
                        implicitHeight: 40
                        text: I18n.t("Paste")
                        icon: "edit"
                        outlined: true
                        content: youtubeCard.fg
                        enabled: !youtubeCard.busy
                        onClicked: {
                            cookieField.text = ""
                            cookieField.error = false
                            cookieField.supportingText = ""
                            cookieRow.editing = true
                            cookieField.forceActiveFocus()
                        }
                    }
                }
                // the field sits on a theme surface so the Md3 TextField keeps its own contrast
                Rectangle {
                    visible: cookieRow.editing
                    Layout.fillWidth: true
                    implicitHeight: cookieCol.implicitHeight + 32
                    radius: Theme.shape.large + 4
                    color: Theme.surfaceContainerHigh
                    ColumnLayout {
                        id: cookieCol
                        x: 16
                        y: 16
                        width: parent.width - 32
                        spacing: 12
                        TextField {
                            id: cookieField
                            Layout.fillWidth: true
                            label: "Cookie"
                            placeholderText: I18n.t("SAPISID=…; __Secure-3PAPISID=…")
                            leadingIcon: "cookie"
                            echoMode: TextInput.Password
                            onAccepted: saveCookies.clicked()
                        }
                        Row {
                            spacing: 8
                            Button {
                                id: saveCookies
                                text: I18n.t("Sign in")
                                size: "xs"
                                enabled: cookieField.text.trim() !== ""
                                onClicked: {
                                    const error = Auth.youtubeSignInWithCookies(cookieField.text.trim())
                                    cookieField.error = error !== ""
                                    cookieField.supportingText = error
                                    if (error === "") {
                                        cookieField.text = ""
                                        cookieRow.editing = false
                                    }
                                }
                            }
                            Button {
                                text: I18n.t("Cancel")
                                style: "text"
                                size: "xs"
                                onClicked: cookieRow.editing = false
                            }
                        }
                    }
                }
            }
        }
    }

    // ================= 3. Quick Import =================
    Rectangle {
        id: importCard
        property int source: 0          // 0 Spotify, 1 YouTube
        property bool started: false
        readonly property bool running: Library.importRunning
        readonly property bool finished: started && !running
        readonly property color spotify: Theme.dark ? "#1ED760" : "#1DB954"
        function submit() {
            const text = importField.text.trim()
            if (text === "" || running)
                return
            started = true
            Library.importPlaylist(text, Auth.youtubeSignedIn)
        }

        Layout.fillWidth: true
        implicitHeight: importCol.implicitHeight + 48
        radius: Theme.shape.extraLarge + 4
        color: Theme.surfaceContainerHigh

        ColumnLayout {
            id: importCol
            x: 24
            y: 24
            width: parent.width - 48
            spacing: 16

            RowLayout {
                Layout.fillWidth: true
                spacing: 16
                Rectangle {
                    Layout.preferredWidth: 56
                    Layout.preferredHeight: 56
                    radius: 28
                    color: Theme.tertiaryContainer
                    Icon { anchors.centerIn: parent; name: "cloud_done"; size: 28; fill: 1; color: Theme.fgTertiaryContainer }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label { Layout.fillWidth: true; text: I18n.t("Quick Import"); type: "titleLarge"; weight: 700 }
                    Label {
                        Layout.fillWidth: true
                        text: I18n.t("Sync playlists & songs instantly")
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                        wrapMode: Text.WordWrap
                        elide: Text.ElideNone
                    }
                }
            }

            // Spotify / YouTube segmented selector
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 64
                radius: 32
                color: Theme.surfaceContainerLowest
                RowLayout {
                    anchors.fill: parent
                    anchors.margins: 6
                    spacing: 6
                    Repeater {
                        model: [
                            { text: "Spotify", icon: "music_note" },
                            { text: "YouTube", icon: "play_arrow" }
                        ]
                        Rectangle {
                            id: seg
                            required property var modelData
                            required property int index
                            readonly property bool selected: importCard.source === index
                            readonly property color segContent: selected
                                ? (index === 0 ? "#000000" : "#FFFFFF")
                                : Theme.fgSurface
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            radius: height / 2
                            color: selected ? (index === 0 ? importCard.spotify : "#E00000") : "transparent"
                            Behavior on color { ColorAnimation { duration: Theme.motion.effectsFast } }
                            Row {
                                anchors.centerIn: parent
                                spacing: 10
                                Rectangle {
                                    width: 24
                                    height: seg.index === 0 ? 24 : 17
                                    radius: seg.index === 0 ? 12 : 5
                                    anchors.verticalCenter: parent.verticalCenter
                                    color: seg.index === 0 ? (seg.selected ? "#000000" : importCard.spotify) : (seg.selected ? "#FFFFFF" : "#FF0000")
                                    Icon {
                                        anchors.centerIn: parent
                                        name: seg.modelData.icon
                                        size: 16
                                        fill: 1
                                        color: seg.index === 0 ? (seg.selected ? importCard.spotify : "#000000") : (seg.selected ? "#E00000" : "#FFFFFF")
                                    }
                                }
                                Label {
                                    text: seg.modelData.text
                                    type: "titleMedium"
                                    weight: 700
                                    color: seg.segContent
                                    anchors.verticalCenter: parent.verticalCenter
                                }
                            }
                            StateLayer {
                                anchors.fill: parent
                                radius: parent.radius
                                color: seg.segContent
                                onClicked: importCard.source = seg.index
                            }
                        }
                    }
                }
            }

            Label {
                Layout.fillWidth: true
                Layout.leftMargin: 8
                text: importCard.source === 0
                    ? I18n.t("Import a public playlist from Spotify")
                    : I18n.t("Import a YouTube or YouTube Music playlist")
                type: "bodyLarge"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }

            // pill URL field with a round → button
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 60
                radius: 30
                color: Theme.surfaceContainerLowest
                border.width: importField.activeFocus ? 2 : 0
                border.color: Theme.primary
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 20
                    anchors.rightMargin: 6
                    spacing: 12
                    Icon { name: "link"; size: 24; color: Theme.fgSurfaceVariant }
                    TextInput {
                        id: importField
                        Layout.fillWidth: true
                        clip: true
                        color: Theme.fgSurface
                        selectionColor: Theme.primary
                        selectedTextColor: Theme.fgPrimary
                        font.pixelSize: 16
                        enabled: !importCard.running
                        onAccepted: importCard.submit()
                        Label {
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width
                            visible: importField.text === ""
                            text: importCard.source === 0 ? "https://open.spotify.com/playlist/…" : "https://music.youtube.com/playlist?list=…"
                            type: "bodyLarge"
                            color: Theme.fgSurfaceVariant
                        }
                    }
                    Rectangle {
                        Layout.preferredWidth: 48
                        Layout.preferredHeight: 48
                        radius: 24
                        readonly property bool ready: importField.text.trim() !== "" || importCard.running
                        color: ready ? Theme.primary : Theme.surfaceContainerHigh
                        Behavior on color { ColorAnimation { duration: Theme.motion.effectsFast } }
                        Icon {
                            anchors.centerIn: parent
                            name: importCard.running ? "close" : "arrow_forward"
                            size: 24
                            color: parent.ready ? Theme.fgPrimary : Theme.fgSurfaceVariant
                        }
                        StateLayer {
                            anchors.fill: parent
                            radius: 24
                            enabled: parent.ready
                            color: Theme.fgPrimary
                            onClicked: importCard.running ? Library.cancelImport() : importCard.submit()
                        }
                    }
                }
            }

            // progress / result
            RowLayout {
                visible: importCard.started
                Layout.fillWidth: true
                Layout.leftMargin: 8
                spacing: 12
                LoadingIndicator { visible: importCard.running; size: 24 }
                Icon {
                    visible: importCard.finished
                    name: Library.importError !== "" ? "error" : "check_circle"
                    size: 22
                    fill: 1
                    color: Library.importError !== "" ? Theme.error : Theme.primary
                }
                Label {
                    Layout.fillWidth: true
                    text: importCard.running
                        ? (Library.importTotal > 0 ? I18n.t("Matching songs: {0} of {1}", Library.importDone, Library.importTotal)
                                                   : I18n.t("Reading the playlist…"))
                        : Library.importError !== "" ? Library.importError
                        : I18n.t("Found {0} of {1} songs", Library.importTracks ? Library.importTracks.count : 0, Library.importTotal)
                    type: "bodyMedium"
                    color: Library.importError !== "" && importCard.finished ? Theme.error : Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                    elide: Text.ElideNone
                }
                Button {
                    visible: importCard.finished && Library.importPlaylistUri !== ""
                    text: I18n.t("Open")
                    icon: "open_in_new"
                    style: "tonal"
                    size: "xs"
                    onClicked: Catalog.openUri(Library.importPlaylistUri)
                }
            }
            Label {
                visible: !Auth.youtubeSignedIn
                Layout.fillWidth: true
                Layout.leftMargin: 8
                text: I18n.t("Sign in to YouTube Music to save imports as a playlist in your library")
                type: "bodySmall"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
        }
    }

    // ================= 4. Linked Services: Last.fm =================
    // The user's own API key and secret (OS credential store); sign-in is approved on last.fm in the browser.
    SectionTitle { text: I18n.t("Linked Services") }

    Rectangle {
        id: lastfmCard
        Layout.fillWidth: true
        implicitHeight: lfCol.implicitHeight + 48
        radius: Theme.shape.extraLarge + 4
        color: Theme.surfaceContainerHigh

        ColumnLayout {
            id: lfCol
            x: 24
            y: 24
            width: parent.width - 48
            spacing: 16

            RowLayout {
                Layout.fillWidth: true
                spacing: 16
                Rectangle {
                    Layout.preferredWidth: 60
                    Layout.preferredHeight: 60
                    Layout.alignment: Qt.AlignTop
                    radius: 30
                    color: Theme.primaryContainer
                    Icon { anchors.centerIn: parent; name: "music_note"; size: 30; fill: 1; color: Theme.fgPrimaryContainer }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label { Layout.fillWidth: true; text: "Last.fm"; type: "headlineSmall"; weight: 700 }
                    Label {
                        Layout.fillWidth: true
                        text: LastFm.connected ? (LastFm.username || "Last.fm")
                            : LastFm.signingIn ? I18n.t("Waiting for you in the browser…")
                            : I18n.t("Not connected")
                        type: "titleMedium"
                        color: Theme.fgSurfaceVariant
                        wrapMode: Text.WordWrap
                        elide: Text.ElideNone
                    }
                }
                LoadingIndicator {
                    visible: LastFm.signingIn
                    Layout.alignment: Qt.AlignTop
                    size: 32
                }
                Rectangle {
                    Layout.alignment: Qt.AlignTop
                    implicitWidth: chipText.implicitWidth + 32
                    implicitHeight: 40
                    radius: 20
                    color: LastFm.connected ? Theme.primaryContainer : Theme.surfaceContainerHighest
                    Label {
                        id: chipText
                        anchors.centerIn: parent
                        text: LastFm.connected ? I18n.t("Connected") : I18n.t("Not linked")
                        type: "titleSmall"
                        weight: 700
                        color: LastFm.connected ? Theme.fgPrimaryContainer : Theme.fgSurfaceVariant
                    }
                }
            }

            SettingsBanner {
                kind: "error"
                shown: LastFm.errorText !== "" && !LastFm.signingIn
                title: I18n.t("Last.fm")
                text: LastFm.errorText
            }

            // --- connected ---
            Rectangle {
                visible: LastFm.connected
                Layout.fillWidth: true
                implicitHeight: 56
                radius: 28
                color: Theme.surfaceContainerLowest
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 20
                    anchors.rightMargin: 20
                    spacing: 14
                    Icon {
                        name: Settings.lastfmScrobble ? "sync" : "cloud_off"
                        size: 22
                        color: Settings.lastfmScrobble ? Theme.primary : Theme.fgSurfaceVariant
                    }
                    Label {
                        Layout.fillWidth: true
                        text: Settings.lastfmScrobble ? I18n.t("Scrobbling enabled") : I18n.t("Scrobbling paused")
                        type: "bodyLarge"
                    }
                }
            }
            Rectangle { visible: LastFm.connected; Layout.fillWidth: true; implicitHeight: 1; color: Theme.outlineVariant }
            PillButton {
                visible: LastFm.connected
                text: I18n.t("Open Service")
                icon: "open_in_new"
                color: Theme.primaryContainer
                content: Theme.fgPrimaryContainer
                onClicked: root.openLastFm()     // the in-app Last.fm Integration page (LastFmSettings.qml)
            }
            PillButton {
                visible: LastFm.connected
                text: I18n.t("Log out")
                icon: "logout"
                outlined: true
                content: Theme.fgSurface
                onClicked: LastFm.disconnect()
            }

            // --- not connected: description, API key, Connect ---
            Label {
                visible: !LastFm.connected
                Layout.fillWidth: true
                text: LastFm.signingIn ? I18n.t("Press “Yes, allow access” on the Last.fm page that opened, then come back here")
                    : I18n.t("Connect opens Last.fm in your browser to approve Pixel Music. Your password is never typed into this app")
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
            ColumnLayout {
                visible: !LastFm.connected && !LastFm.signingIn
                Layout.fillWidth: true
                spacing: 12
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Label {
                        Layout.fillWidth: true
                        text: I18n.t("Your own Last.fm API account: create one on Last.fm (any name, no callback URL)")
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                        wrapMode: Text.WordWrap
                        elide: Text.ElideNone
                    }
                    Button {
                        text: I18n.t("Create")
                        icon: "open_in_new"
                        style: "text"
                        size: "xs"
                        onClicked: Qt.openUrlExternally(LastFm.createKeyUrl)
                    }
                }
                TextField {
                    id: lastfmKey
                    Layout.fillWidth: true
                    label: I18n.t("API key")
                    leadingIcon: "key"
                    text: LastFm.apiKey
                }
                TextField {
                    id: lastfmSecret
                    Layout.fillWidth: true
                    label: I18n.t("Shared secret")
                    placeholderText: LastFm.hasKeys ? I18n.t("Saved — type to replace") : ""
                    supportingText: I18n.t("Stored in the system credential store")
                    leadingIcon: "lock"
                    echoMode: TextInput.Password
                    onAccepted: saveLastfmKeys.clicked()
                }
                Row {
                    spacing: 8
                    Button {
                        id: saveLastfmKeys
                        text: I18n.t("Save keys")
                        style: "tonal"
                        size: "xs"
                        enabled: lastfmKey.text.trim() !== "" && (lastfmSecret.text.trim() !== "" || LastFm.hasKeys)
                        onClicked: {
                            LastFm.setKeys(lastfmKey.text.trim(), lastfmSecret.text.trim())
                            lastfmSecret.text = ""
                        }
                    }
                    Button {
                        visible: LastFm.hasKeys
                        text: I18n.t("Remove keys")
                        style: "text"
                        size: "xs"
                        onClicked: {
                            LastFm.forget()
                            lastfmKey.text = ""
                            lastfmSecret.text = ""
                        }
                    }
                }
            }
            PillButton {
                visible: !LastFm.connected && !LastFm.signingIn
                text: I18n.t("Connect")
                icon: "open_in_new"
                enabled: LastFm.hasKeys
                color: Theme.primary
                content: Theme.fgPrimary
                onClicked: LastFm.signIn()
            }
            RowLayout {
                visible: LastFm.signingIn
                Layout.fillWidth: true
                spacing: 12
                PillButton {
                    visible: LastFm.authUrl !== ""
                    text: I18n.t("Open page")
                    icon: "open_in_new"
                    color: Theme.primaryContainer
                    content: Theme.fgPrimaryContainer
                    onClicked: Qt.openUrlExternally(LastFm.authUrl)
                }
                PillButton {
                    text: I18n.t("Cancel")
                    outlined: true
                    content: Theme.fgSurface
                    onClicked: LastFm.cancelSignIn()
                }
            }
        }
    }

    // ================= 5. Available Services =================
    SectionTitle { text: I18n.t("Available Services") }
    Rectangle {
        Layout.fillWidth: true
        implicitHeight: availCol.implicitHeight + 48
        radius: Theme.shape.extraLarge + 4
        color: Theme.surfaceContainerHigh
        ColumnLayout {
            id: availCol
            x: 24
            y: 24
            width: parent.width - 48
            spacing: 12
            Label {
                Layout.fillWidth: true
                text: root.activeCount === root.availableCount ? I18n.t("Everything is linked") : I18n.t("Not linked yet")
                type: "titleLarge"
                weight: 700
            }
            Label {
                Layout.fillWidth: true
                text: root.activeCount === root.availableCount
                    ? I18n.t("More services will show up here as they become available.")
                    : I18n.t("Connect a provider to manage it from this screen.")
                type: "bodyLarge"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
            PillButton {
                visible: !Auth.youtubeSignedIn
                text: I18n.t("Connect YouTube Music")
                icon: "play_arrow"
                color: Theme.secondaryContainer
                content: Theme.fgSecondaryContainer
                enabled: !youtubeCard.busy
                onClicked: Auth.startYouTubeLogin()
            }
            PillButton {
                visible: !LastFm.connected
                text: I18n.t("Connect Last.fm")
                icon: "music_note"
                color: Theme.secondaryContainer
                content: Theme.fgSecondaryContainer
                enabled: LastFm.hasKeys && !LastFm.signingIn
                onClicked: LastFm.signIn()
            }
            // not built yet: shown so the roadmap is visible, never clickable
            Repeater {
                model: [I18n.t("Telegram"), I18n.t("Google Drive")]
                PillButton {
                    required property string modelData
                    text: I18n.t("{0} (Coming soon)", modelData)
                    icon: "schedule"
                    enabled: false
                    color: Theme.surfaceContainerHighest
                    content: Theme.fgSurfaceVariant
                }
            }
        }
    }
}
