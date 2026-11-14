import QtQuick
import QtQuick.Layouts
import Md3

// Группа настроек: заголовок секции + карточка.
// grouped: без общей карточки — строки (SettingsCategoryRow) сами образуют скруглённую группу, как в PixelPlayer.
ColumnLayout {
    id: root

    property string title
    property string icon
    property bool grouped: false
    default property alias rows: body.data

    Layout.fillWidth: true
    spacing: 12

    // PixelPlayer-style section label: small, bold, upper case, in the primary colour
    RowLayout {
        Layout.leftMargin: 4
        spacing: 8
        Icon { name: root.icon; size: 18; color: Theme.primary; visible: root.icon !== "" }
        Label { text: root.title.toLocaleUpperCase(); type: "labelLarge"; weight: 700; font.letterSpacing: 1; color: Theme.primary }
    }

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: body.implicitHeight + (root.grouped ? 0 : 40)
        radius: Theme.shape.extraLarge
        color: root.grouped ? "transparent" : Theme.surfaceContainerHigh

        ColumnLayout {
            id: body
            x: root.grouped ? 0 : 24
            y: root.grouped ? 0 : 20
            width: parent.width - 2 * x
            spacing: root.grouped ? 2 : 20
        }
    }
}
