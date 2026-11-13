import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Same-Wi-Fi Jam card under Spotify's Jam card in the Friends tab. No account: the host's room code is found on
// this network (UDP beacon), or the invite link carries its address. Works with SoundWave on Android and Windows.
Rectangle {
    id: root

    readonly property var jam: LanJam.state ?? {}
    readonly property bool inJam: jam.status === "joined"
    readonly property bool busy: jam.busy === true
    readonly property var members: jam.members ?? []
    readonly property var nearby: jam.nearby ?? []
    property bool joining: false

    onInJamChanged: if (inJam) joining = false
    // list nearby rooms only while the card is on screen and not in a Jam
    readonly property bool wantsDiscovery: visible && !inJam
    onWantsDiscoveryChanged: wantsDiscovery ? LanJam.startDiscovery() : LanJam.stopDiscovery()
    Component.onCompleted: if (wantsDiscovery) LanJam.startDiscovery()
    Component.onDestruction: LanJam.stopDiscovery()

    implicitHeight: body.implicitHeight + 32
    radius: Theme.shape.large
    color: Theme.surfaceContainerHigh

    ColumnLayout {
        id: body
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 16
        spacing: 12

        // Header: icon, title, LIVE, subtitle
        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            MorphShape {
                Layout.preferredWidth: 44
                Layout.preferredHeight: 44
                shape: root.inJam ? "sunny" : "cookie9"
                color: root.inJam ? Theme.primary : Theme.secondaryContainer
                Icon {
                    anchors.centerIn: parent
                    name: "wifi"
                    size: 24
                    fill: 1
                    color: root.inJam ? Theme.fgPrimary : Theme.fgSecondaryContainer
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                RowLayout {
                    spacing: 8
                    Label { text: I18n.t("Same Wi-Fi Jam"); type: "titleMedium" }
                    Rectangle {
                        visible: root.inJam
                        implicitWidth: live.implicitWidth + 12
                        implicitHeight: 18
                        radius: 9
                        color: Theme.primaryContainer
                        Label {
                            id: live
                            anchors.centerIn: parent
                            text: I18n.t("LIVE")
                            type: "labelSmall"
                            weight: 700
                            color: Theme.fgPrimaryContainer
                        }
                    }
                }
                Label {
                    Layout.fillWidth: true
                    text: !root.inJam ? I18n.t("Listen together on this network, no account needed")
                        : root.jam.isHost ? I18n.n(root.members.length, "You're hosting · {0} person", "You're hosting · {0} people")
                        : root.jam.pendingApproval ? I18n.t("Waiting for the host to let you in")
                        : I18n.t("Hosted by {0}", root.jam.hostName ?? "")
                    type: "bodySmall"
                    color: Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                    elide: Text.ElideNone
                }
            }
            LoadingIndicator {
                visible: root.busy
                running: visible && !Theme.calm
                Layout.preferredWidth: 28
                Layout.preferredHeight: 28
                size: 28
            }
        }

        // ---- Not in a Jam ----
        Label {
            visible: !root.inJam
            Layout.fillWidth: true
            text: I18n.t("Friends on the same Wi-Fi join with the room code. YouTube Music songs play on every device; playback follows the host.")
            type: "bodySmall"
            color: Theme.fgSurfaceVariant
            wrapMode: Text.WordWrap
            elide: Text.ElideNone
        }
        Flow {
            visible: !root.inJam
            Layout.fillWidth: true
            spacing: 8
            Button {
                text: I18n.t("Host on this Wi-Fi")
                icon: "wifi_tethering"
                enabled: !root.busy
                onClicked: LanJam.host("")
            }
            Button {
                text: I18n.t("Join with a code")
                icon: "pin"
                style: root.joining ? "tonal" : "outlined"
                enabled: !root.busy
                onClicked: {
                    root.joining = !root.joining
                    if (root.joining)
                        codeField.inputItem.forceActiveFocus()
                }
            }
        }
        RowLayout {
            visible: !root.inJam && root.joining
            Layout.fillWidth: true
            spacing: 8
            TextField {
                id: codeField
                Layout.fillWidth: true
                placeholderText: I18n.t("Room code or invite link")
                leadingIcon: "pin"
                onAccepted: if (text.trim() !== "") LanJam.join(text, "")
            }
            Button {
                text: I18n.t("Join")
                style: "tonal"
                enabled: codeField.text.trim() !== "" && !root.busy
                onClicked: LanJam.join(codeField.text, "")
            }
        }
        // rooms heard on this network
        Label {
            visible: !root.inJam && root.nearby.length > 0
            text: I18n.t("Nearby")
            type: "labelMedium"
            color: Theme.fgSurfaceVariant
        }
        Repeater {
            model: !root.inJam ? root.nearby : []
            RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 12
                Avatar {
                    size: 32
                    name: modelData.hostName ?? ""
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label { Layout.fillWidth: true; text: modelData.hostName ?? ""; type: "bodyMedium" }
                    Label { Layout.fillWidth: true; text: "#" + (modelData.roomId ?? ""); type: "bodySmall"; color: Theme.fgSurfaceVariant }
                }
                Button {
                    text: I18n.t("Join")
                    style: "text"
                    size: "xs"
                    enabled: !root.busy
                    onClicked: LanJam.join(modelData.roomId, "")
                }
            }
        }

        // ---- Host: the room code ----
        ColumnLayout {
            visible: root.inJam && root.jam.isHost === true
            Layout.fillWidth: true
            spacing: 0
            Label {
                text: I18n.t("Room code")
                type: "labelMedium"
                color: Theme.fgSurfaceVariant
            }
            Label {
                text: root.jam.roomCode ?? ""
                type: "headlineSmall"
                weight: 700
                color: Theme.primary
            }
            Label {
                visible: (root.jam.address ?? "") !== ""
                Layout.fillWidth: true
                text: I18n.t("On this network at {0}", root.jam.address ?? "")
                type: "bodySmall"
                color: Theme.fgSurfaceVariant
            }
        }

        // ---- In a Jam: people ----
        Label {
            visible: root.inJam
            text: I18n.t("Listening")
            type: "labelMedium"
            color: Theme.fgSurfaceVariant
        }
        Repeater {
            model: root.inJam ? root.members : []
            RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 12
                Avatar {
                    size: 32
                    name: modelData.name ?? ""
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label {
                        Layout.fillWidth: true
                        text: modelData.isYou ? I18n.t("{0} (you)", modelData.name ?? "") : (modelData.name ?? "")
                        type: "bodyMedium"
                    }
                    Label {
                        visible: (modelData.device ?? "") !== "" || modelData.isPending === true
                        Layout.fillWidth: true
                        text: modelData.isPending === true ? I18n.t("Wants to join") : (modelData.device ?? "")
                        type: "bodySmall"
                        color: Theme.fgSurfaceVariant
                    }
                }
                Label {
                    visible: modelData.isHost === true
                    text: I18n.t("Jam host")
                    type: "labelSmall"
                    color: Theme.primary
                }
                IconButton {
                    visible: root.jam.isHost === true && modelData.isPending === true
                    icon: "check"
                    onClicked: LanJam.approve(modelData.userId)
                }
                IconButton {
                    visible: root.jam.isHost === true && modelData.isHost !== true
                    icon: "person_off"
                    onClicked: LanJam.remove(modelData.userId)
                }
            }
        }

        // ---- Guest: ask the host to play, pause, skip ----
        RowLayout {
            visible: root.inJam && root.jam.isHost !== true && root.jam.canControl === true && root.jam.pendingApproval !== true
            Layout.alignment: Qt.AlignHCenter
            spacing: 8
            IconButton {
                icon: "skip_previous"
                onClicked: LanJam.previous()
            }
            IconButton {
                icon: Player.playing ? "pause" : "play_arrow"
                style: "filled"
                onClicked: LanJam.togglePlay()
            }
            IconButton {
                icon: "skip_next"
                onClicked: LanJam.next()
            }
        }

        // ---- In a Jam: actions ----
        Flow {
            visible: root.inJam
            Layout.fillWidth: true
            Layout.topMargin: 4
            spacing: 8
            Button {
                visible: root.jam.isHost === true
                text: copiedHint.running ? I18n.t("Copied") : I18n.t("Copy invite link")
                icon: copiedHint.running ? "check" : "content_copy"
                style: "tonal"
                enabled: (root.jam.inviteUrl ?? "") !== ""
                onClicked: if (LanJam.copyInvite()) copiedHint.restart()
                Timer { id: copiedHint; interval: 1500 }
            }
            Button {
                text: root.jam.isHost ? I18n.t("End Jam") : I18n.t("Leave Jam")
                icon: "logout"
                style: "outlined"
                onClicked: LanJam.leave()
            }
        }

        // ---- Error ----
        RowLayout {
            visible: (root.jam.error ?? "") !== ""
            Layout.fillWidth: true
            spacing: 8
            Icon { name: "error"; size: 18; color: Theme.error }
            Label {
                Layout.fillWidth: true
                text: root.jam.error ?? ""
                type: "bodySmall"
                color: Theme.error
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
        }
    }
}
