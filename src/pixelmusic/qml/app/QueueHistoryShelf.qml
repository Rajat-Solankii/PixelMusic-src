import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Home → "Queue history" (PixelPlayer): queues that were replaced by something else, newest first. A card: the
// song count on a pastel tile, the covers stacked, "Starts with" the first song, when it was replaced, and
// "Replay queue" + delete. QueueHistory (bridge) keeps them in queue_history.json.
ColumnLayout {
    id: root
    visible: QueueHistory.count > 0
    spacing: 12

    RowLayout {
        Layout.fillWidth: true
        spacing: 12
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Label { text: I18n.t("Queue history"); type: "headlineSmall"; weight: 700 }
            Label {
                text: I18n.n(QueueHistory.count, "{0} saved queue", "{0} saved queues")
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
            }
        }
        Item { Layout.fillWidth: true }
        Button {
            text: I18n.t("Clear all")
            style: "text"
            onClicked: QueueHistory.clear()
        }
    }

    ListView {
        id: list
        Layout.fillWidth: true
        Layout.preferredHeight: 392
        orientation: ListView.Horizontal
        SideScroll { flickable: list }
        spacing: 16
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        model: QueueHistory.queues
        // a mouse wheel scrolls the page, Shift + wheel (or a touchpad's sideways swipe) this row
        WheelHandler {
            acceptedModifiers: Qt.ShiftModifier
            onWheel: e => list.contentX = Math.max(0, Math.min(list.contentWidth - list.width,
                                                                list.contentX - (e.angleDelta.y || e.angleDelta.x)))
        }
        delegate: Rectangle {
            id: card
            required property var modelData
            readonly property var first: modelData.first ?? ({})
            readonly property var covers: modelData.covers ?? []
            width: Math.min(340, list.width - 24)
            height: list.height
            radius: 28
            color: Theme.surfaceContainer

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 6

                RowLayout {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 168
                    spacing: 8

                    // the count on a pastel tile
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        radius: 20
                        color: Theme.primaryContainer
                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 16
                            spacing: 0
                            MorphShape {
                                Layout.preferredWidth: 36
                                Layout.preferredHeight: 36
                                shape: "cookie9"
                                color: Qt.alpha(Theme.fgPrimaryContainer, 0.16)
                                Icon {
                                    anchors.centerIn: parent
                                    name: "queue_music"
                                    size: 20
                                    fill: 1
                                    color: Theme.fgPrimaryContainer
                                }
                            }
                            Item { Layout.fillHeight: true }
                            Label {
                                text: card.modelData.count ?? 0
                                type: "displayMedium"
                                weight: 700
                                color: Theme.fgPrimaryContainer
                            }
                            Label {
                                text: I18n.n(card.modelData.count ?? 0, "track", "tracks")
                                type: "titleMedium"
                                weight: 600
                                color: Theme.fgPrimaryContainer
                            }
                        }
                    }
                    // the covers, fanned
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        radius: 20
                        color: Theme.surfaceContainerHighest
                        Repeater {
                            model: card.covers.slice().reverse()
                            MorphImage {
                                required property string modelData
                                required property int index
                                readonly property int depth: card.covers.length - 1 - index   // 0 — on top
                                // fanned left to right: the top cover in front on the left, the others peek out
                                width: 84
                                height: 84
                                anchors.centerIn: parent
                                anchors.horizontalCenterOffset: (depth - (card.covers.length - 1) / 2) * 22
                                anchors.verticalCenterOffset: -depth * 4
                                rotation: [-6, 6, 14][depth] ?? 0
                                shape: "softSquare"
                                source: modelData
                            }
                        }
                        Icon {
                            anchors.centerIn: parent
                            visible: card.covers.length === 0
                            name: "music_note"
                            size: 40
                        }
                    }
                }

                Label {
                    Layout.topMargin: 10
                    Layout.leftMargin: 8
                    text: I18n.t("Starts with")
                    type: "labelLarge"
                    color: Theme.fgSurfaceVariant
                }
                Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: 8
                    Layout.rightMargin: 8
                    text: card.first.title ?? ""
                    type: "titleLarge"
                    weight: 700
                    elide: Text.ElideRight
                }
                Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: 8
                    Layout.rightMargin: 8
                    text: card.first.artists ?? ""
                    type: "bodyMedium"
                    color: Theme.fgSurfaceVariant
                    elide: Text.ElideRight
                }
                RowLayout {
                    Layout.leftMargin: 8
                    Layout.topMargin: 4
                    spacing: 6
                    Icon { name: "history"; size: 18 }
                    Label {
                        text: I18n.relative(new Date(card.modelData.savedAt ?? 0))
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                    }
                    Label {
                        visible: (card.modelData.title ?? "") !== ""
                        Layout.fillWidth: true
                        text: "· " + (card.modelData.title ?? "")
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                        elide: Text.ElideRight
                    }
                }
                Item { Layout.fillHeight: true }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Button {
                        Layout.fillWidth: true
                        text: I18n.t("Replay queue")
                        icon: "play_arrow"
                        size: "m"
                        style: "tonal"
                        onClicked: QueueHistory.replay(card.modelData.id)
                    }
                    IconButton {
                        icon: "delete"
                        style: "tonal"
                        size: "m"
                        onClicked: QueueHistory.remove(card.modelData.id)
                    }
                }
            }
        }
    }
}
