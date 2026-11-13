import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Quick Picks shelf - 2-column grid of tracks with cover, title, artist, and duration
// Similar to the SoundWave Android app's Quick Picks section
ColumnLayout {
    id: root
    
    property var tracks: []
    property bool compact: false
    property string title: I18n.t("Quick picks")
    property bool showMore: false
    // "Show more" opens the rest of the shelf in place (the first 8 songs otherwise)
    property bool expanded: false
    
    signal activated(var track, int index)
    signal showMoreClicked()
    signal trackMenuRequested(var track, point position)
    
    spacing: 12
    
    // Section header with optional "Show more" action
    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: 8
        spacing: 8
        
        Label {
            Layout.fillWidth: true
            text: root.title
            type: "headlineSmall"
            weight: 600
            fade: true
        }
        
        Label {
            visible: root.showMore
            Layout.alignment: Qt.AlignRight
            text: root.expanded ? I18n.t("Show less") : I18n.t("Show more")
            type: "labelLarge"
            color: Theme.primary
            
            HoverHandler { id: moreHover; cursorShape: Qt.PointingHandCursor }
            
            Rectangle {
                anchors.fill: parent
                anchors.margins: -4
                radius: Theme.shape.small
                color: Theme.primary
                opacity: moreHover.hovered ? 0.1 : 0
            }
            
            TapHandler {
                onTapped: { root.expanded = !root.expanded; root.showMoreClicked() }
            }
        }
    }
    
    // 2-column grid of tracks
    GridLayout {
        id: grid
        Layout.fillWidth: true
        columns: root.compact ? 1 : 2
        columnSpacing: 16
        rowSpacing: 8
        
        Repeater {
            model: root.showMore && !root.expanded ? root.tracks.slice(0, 8) : root.tracks
            
            Item {
                id: trackItem
                required property var modelData
                required property int index
                
                readonly property string trackId: modelData.trackId ?? ""
                readonly property string title: modelData.title ?? ""
                readonly property string artists: modelData.artists ?? ""
                readonly property string cover: modelData.cover ?? ""
                readonly property int durationMs: modelData.durationMs ?? 0
                readonly property bool current: trackId !== "" && trackId === Player.trackId
                readonly property bool playing: current && Player.playing
                
                Layout.fillWidth: true
                Layout.preferredHeight: 56
                
                // Background state layer for hover/click
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.shape.large
                    color: trackItem.current ? Qt.alpha(Theme.primary, 0.12) : "transparent"
                    
                    Behavior on color { ColorAnimation { duration: Theme.motion.effectsFast } }
                }
                
                // Hover effect
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.shape.large
                    color: Theme.fgSurface
                    opacity: area.pressed ? Theme.stateLayer.pressed : area.containsMouse ? Theme.stateLayer.hover : 0
                    
                    Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
                }
                
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    spacing: 12
                    
                    // Track number or current indicator
                    Item {
                        Layout.preferredWidth: 28
                        Layout.preferredHeight: 28
                        
                        Label {
                            visible: !trackItem.current
                            anchors.centerIn: parent
                            text: trackItem.index + 1
                            type: "bodyMedium"
                            color: Theme.fgSurfaceVariant
                            horizontalAlignment: Text.AlignHCenter
                        }
                        
                        // Current track indicator (animated bars)
                        Item {
                            visible: trackItem.current
                            anchors.centerIn: parent
                            width: 16
                            height: 16
                            
                            Row {
                                anchors.centerIn: parent
                                spacing: 2
                                
                                Repeater {
                                    model: 3
                                    
                                    Rectangle {
                                        width: 3
                                        height: parent.height
                                        radius: 1
                                        color: Theme.primary
                                        
                                        // Animate bars when playing
                                        SequentialAnimation on height {
                                            running: trackItem.playing && trackItem.visible
                                            loops: Animation.Infinite
                                            NumberAnimation { to: 8 + Math.random() * 8; duration: 400 + Math.random() * 200 }
                                            NumberAnimation { to: 4; duration: 400 + Math.random() * 200 }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Track cover
                    Item {
                        Layout.preferredWidth: 44
                        Layout.preferredHeight: 44
                        
                        MorphImage {
                            anchors.fill: parent
                            source: trackItem.cover
                            shape: "softSquare"
                            fallbackKind: "track"
                            fallbackArtist: trackItem.artists.split(", ")[0]
                            fallbackTitle: trackItem.title
                        }
                    }
                    
                    // Track info (title and artist)
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        
                        Label {
                            Layout.fillWidth: true
                            text: trackItem.title
                            type: "titleSmall"
                            color: trackItem.current ? Theme.primary : Theme.fgSurface
                            fade: true
                            elide: Text.ElideRight
                        }
                        
                        Label {
                            Layout.fillWidth: true
                            text: trackItem.artists
                            type: "bodySmall"
                            color: Theme.fgSurfaceVariant
                            fade: true
                            elide: Text.ElideRight
                        }
                    }
                    
                    // the backend fills lengths in; unknown — nothing
                    Label {
                        id: durationLabel
                        text: trackItem.durationMs > 0 ? root._formatTime(trackItem.durationMs) : ""
                        type: "bodySmall"
                        color: Theme.fgSurfaceVariant
                        horizontalAlignment: Text.AlignRight
                        Layout.preferredWidth: 40
                    }
                }
                
                MouseArea {
                    id: area
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    
                    onClicked: mouse => {
                        if (mouse.button === Qt.RightButton) {
                            const p = mapToItem(null, mouse.x, mouse.y)
                            root.trackMenuRequested(trackItem.modelData, p)
                        } else {
                            root.activated(trackItem.modelData, trackItem.index)
                        }
                    }
                }
            }
        }
    }
    
    function _formatTime(ms) {
        const s = Math.round(ms / 1000)
        return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
    }
}
