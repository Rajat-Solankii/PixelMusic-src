import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// A confirmation as PixelPlayer Desktop's AlertDialog: an icon over a bold titleMedium title, the question in
// bodyMedium onSurfaceVariant, "Cancel" as a text button and the action as a filled button. It covers the whole
// window (it re-parents to the window's content item), Esc or a click outside cancels.
// Usage: dialog.ask({ icon, title, text, action, danger }, () => doIt())
Item {
    id: root

    property bool opened: false
    property string icon: "help"
    property string title
    property string text
    property string actionText: I18n.t("OK")
    property bool danger: false
    property var _onConfirm: null

    function ask(spec, onConfirm) {
        icon = spec.icon ?? "help"
        title = spec.title ?? ""
        text = spec.text ?? ""
        actionText = spec.action ?? I18n.t("OK")
        danger = spec.danger === true
        _onConfirm = onConfirm
        opened = true
        card.forceActiveFocus()
    }
    function close() { opened = false }
    function confirm() {
        const f = _onConfirm
        close()
        if (f)
            f()
    }

    parent: root.Window.contentItem
    anchors.fill: parent
    z: 2100
    visible: opened || scrim.opacity > 0

    Rectangle {
        id: scrim
        anchors.fill: parent
        color: Qt.alpha(Theme.scrim, 0.32)
        opacity: root.opened ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onPressed: root.close()
            onWheel: wheel => {}
        }
    }

    Rectangle {
        id: card
        anchors.centerIn: parent
        width: Math.min(400, parent.width - 48)
        height: content.implicitHeight + 48
        radius: Theme.shape.extraLarge
        color: Theme.surfaceContainerHigh
        scale: root.opened ? 1 : 0.92
        opacity: root.opened ? 1 : 0
        Behavior on scale { NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack } }
        Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
        Keys.onEscapePressed: root.close()
        Keys.onReturnPressed: root.confirm()

        Elevation { target: card; level: 3 }
        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }

        ColumnLayout {
            id: content
            anchors.fill: parent
            anchors.margins: 24
            spacing: 16

            Icon {
                Layout.alignment: Qt.AlignHCenter
                name: root.icon
                size: 28
                color: root.danger ? Theme.error : Theme.secondary
            }
            Label {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: root.title
                type: "titleMedium"
                weight: 700
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
            Label {
                Layout.fillWidth: true
                text: root.text
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
            RowLayout {
                Layout.alignment: Qt.AlignRight
                spacing: 8
                Button { text: I18n.t("Cancel"); style: "text"; onClicked: root.close() }
                Button { text: root.actionText; onClicked: root.confirm() }
            }
        }
    }
}
