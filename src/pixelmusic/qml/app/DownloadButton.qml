import QtQuick
import Md3
import PixelMusic.Core

// Download a YouTube Music song for playing offline, or remove its download (dict from a model or Player.track).
// While it downloads the icon pulses and a click cancels.
IconButton {
    id: root

    property var track: ({})
    readonly property string _uri: track && track.uri ? String(track.uri) : ""
    readonly property bool _can: Downloads.revision >= 0 && _uri !== "" && Downloads.canDownload(_uri)
    readonly property bool downloaded: _can && Downloads.isDownloaded(_uri)
    readonly property bool pending: _can && !downloaded && Downloads.isPending(_uri)

    visible: _can
    icon: downloaded ? "download_done" : pending ? "downloading" : "download"
    checkable: true
    autoToggle: false
    checked: downloaded
    Accessible.name: downloaded ? I18n.t("Remove download") : pending ? I18n.t("Cancel download") : I18n.t("Download")
    onClicked: {
        if (downloaded)
            Downloads.remove(_uri)
        else if (pending)
            Downloads.cancel(_uri)
        else
            Downloads.download(track)
    }

    SequentialAnimation on opacity {
        running: root.pending && !Theme.reducedMotion
        loops: Animation.Infinite
        onStopped: root.opacity = 1
        NumberAnimation { to: 0.45; duration: 700; easing.type: Easing.InOutSine }
        NumberAnimation { to: 1; duration: 700; easing.type: Easing.InOutSine }
    }
}
