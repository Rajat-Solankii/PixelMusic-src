import QtQuick
import QtQuick.Layouts
import Md3

// Logo colorway picker: displays 6 logo variants for user selection
Flow {
    id: root
    spacing: 16

    property string currentColorway: Settings.logoColorway
    
    // Use LogoPaths to ensure consistency with actual logo rendering
    LogoPaths { id: logoPaths }
    
    // Build colorways array from LogoPaths.colorways for consistency
    property var colorways: [
        { key: "bluePurple", name: I18n.t("Blue Purple"), gradient: logoPaths.colorways.bluePurple.gradient },
        { key: "babyPinkPurple", name: I18n.t("Baby Pink Purple"), gradient: logoPaths.colorways.babyPinkPurple.gradient },
        { key: "graphiteGray", name: I18n.t("Graphite Gray"), gradient: logoPaths.colorways.graphiteGray.gradient },
        { key: "limeGreen", name: I18n.t("Lime Green"), gradient: logoPaths.colorways.limeGreen.gradient },
        { key: "limeLemon", name: I18n.t("Lime Lemon"), gradient: logoPaths.colorways.limeLemon.gradient },
        { key: "orangeYellow", name: I18n.t("Orange Yellow"), gradient: logoPaths.colorways.orangeYellow.gradient }
    ]

    signal selected(string key)

    Repeater {
        model: root.colorways

        Rectangle {
            required property var modelData
            required property int index

            width: 80
            height: 80
            radius: Theme.shape.full
            color: Theme.surfaceContainerHigh
            border.width: root.currentColorway === modelData.key ? 3 : 1
            border.color: root.currentColorway === modelData.key ? Theme.primary : Theme.outlineVariant

            Behavior on border.width { NumberAnimation { duration: Theme.motion.effectsFast } }

            // Logo preview
            Rectangle {
                anchors.centerIn: parent
                width: 56
                height: 56
                radius: width / 2
                gradient: Gradient {
                    orientation: Gradient.Horizontal
                    GradientStop { position: 0; color: modelData.gradient[0] }
                    GradientStop { position: 1; color: modelData.gradient[1] }
                }
            }

            // Selection checkmark
            Icon {
                anchors.bottom: parent.bottom
                anchors.right: parent.right
                anchors.margins: 4
                visible: root.currentColorway === modelData.key
                name: "check_circle"
                size: 20
                color: Theme.primary
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                hoverEnabled: true
                onClicked: root.selected(modelData.key)
            }

            // Tooltip on hover
            ToolTip.visible: ma.containsMouse
            ToolTip.text: modelData.name
            ToolTip.delay: 500

            MouseArea {
                id: ma
                anchors.fill: parent
                hoverEnabled: true
                acceptedButtons: Qt.NoButton
            }
        }
    }
}
