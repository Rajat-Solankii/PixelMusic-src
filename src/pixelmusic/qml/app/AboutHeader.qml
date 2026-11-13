import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Settings → About: the header card in PixelPlayer's layout. The logo in the chosen colourway, the app name, one
// line about it, and a version chip (with the build commit as a second chip when the build knows it).
Rectangle {
    id: root

    // same mapping as SidebarLogo (Settings.logoColorway → assets/logo/<file>.svg)
    readonly property var _logoFiles: ({
        bluePurple: "blue_purple", babyPinkPurple: "baby_pink_purple", graphiteGray: "graphite",
        limeGreen: "lime_green", limeLemon: "lime_lemon", orangeYellow: "orange_yellow"
    })

    Layout.fillWidth: true
    implicitHeight: column.implicitHeight + 56
    radius: Theme.shape.extraLarge
    color: Theme.surfaceContainerHigh

    ColumnLayout {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: 24
        anchors.rightMargin: 24
        spacing: 10

        Image {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: 88
            Layout.preferredHeight: 88
            source: Qt.resolvedUrl("../../assets/logo/" + (root._logoFiles[Settings.logoColorway] ?? "blue_purple") + ".svg")
            sourceSize: Qt.size(176, 176)
            fillMode: Image.PreserveAspectFit
            smooth: true
        }
        Label {
            Layout.alignment: Qt.AlignHCenter
            text: "Pixel Music"
            type: "headlineMedium"
            weight: 700
        }
        Label {
            Layout.fillWidth: true
            Layout.maximumWidth: 560
            Layout.alignment: Qt.AlignHCenter
            horizontalAlignment: Text.AlignHCenter
            text: I18n.t("A music player in Material 3 Expressive style: YouTube Music, lossless streams from Monochrome, JioSaavn and your own files.")
            type: "bodyMedium"
            color: Theme.fgSurfaceVariant
            wrapMode: Text.WordWrap
            elide: Text.ElideNone
        }
        Row {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 4
            spacing: 8

            // the version chip: a filled primary-container pill
            Rectangle {
                height: 32
                width: versionRow.implicitWidth + 28
                radius: height / 2
                color: Theme.primaryContainer
                Row {
                    id: versionRow
                    anchors.centerIn: parent
                    spacing: 6
                    Icon { name: "tag"; size: 18; color: Theme.fgPrimaryContainer; anchors.verticalCenter: parent.verticalCenter }
                    Label {
                        text: I18n.t("Version {0}", Updater.appVersion)
                        type: "labelLarge"
                        weight: 600
                        color: Theme.fgPrimaryContainer
                        anchors.verticalCenter: parent.verticalCenter
                    }
                }
            }
            // the build commit, outlined
            Rectangle {
                visible: Updater.buildCommit !== ""
                height: 32
                width: commitLabel.implicitWidth + 28
                radius: height / 2
                color: "transparent"
                border.width: 1
                border.color: Theme.outlineVariant
                Label {
                    id: commitLabel
                    anchors.centerIn: parent
                    text: Updater.buildCommit
                    type: "labelLarge"
                    color: Theme.fgSurfaceVariant
                }
            }
        }
    }
}
