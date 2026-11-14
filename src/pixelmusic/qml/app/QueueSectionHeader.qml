import QtQuick
import QtQuick.Layouts
import Md3

// A quiet label between queue groups ("Next in queue" · count, "Next from …"): PixelPlayer's queue is one list of
// cards under its "Next Up" header, so the groups only get a small onSurfaceVariant caption.
Item {
    id: root

    property string title
    property int count: -1                 // < 0 — без счётчика
    property string note                   // строка под заголовком («Новые треки встают в конец»)

    implicitHeight: col.implicitHeight + 10
    implicitWidth: 240

    ColumnLayout {
        id: col
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 2
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            Label {
                Layout.fillWidth: true
                Layout.maximumWidth: Math.ceil(implicitWidth)
                text: root.title
                type: "labelLarge"
                weight: 600
                color: Theme.fgSurfaceVariant
            }
            Label {
                visible: root.count >= 0
                text: "· " + root.count
                type: "labelLarge"
                color: Qt.alpha(Theme.fgSurfaceVariant, 0.7)
            }
            Item { Layout.fillWidth: true }
        }
        Label {
            visible: root.note !== ""
            Layout.fillWidth: true
            text: root.note
            type: "bodySmall"
            color: Qt.alpha(Theme.fgSurfaceVariant, 0.7)
        }
    }
}
