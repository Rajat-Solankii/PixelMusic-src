import QtQuick
import QtQuick.Dialogs
import Md3
import PixelMusic.Core
import ".."

// Downloaded YouTube Music songs (Downloads): they play from their files, without the network. The header shows
// the queue and the space used, and has retry / cancel / clean-up / folder buttons.
TrackListPage {
    id: page
    title: I18n.t("Downloads")
    overline: I18n.t("COLLECTION")
    heroIcon: "download_done"
    heroShape: "cookie9"
    readonly property int pending: Downloads.queued + Downloads.running
    subtitle: page.pending > 0 ? I18n.t("Downloading {0} songs… {1}%", page.pending, Math.round(Downloads.progress * 100))
            : Downloads.failed > 0 ? I18n.t("{0} downloads failed", Downloads.failed)
            : Downloads.tracks.count > 0 ? I18n.n(Downloads.tracks.count, "{0} track", "{0} tracks") + " · " + Downloads.usage
            : I18n.t("Songs saved on this computer")
    model: Downloads.tracks
    emptyIcon: Downloads.errorText !== "" ? "error" : "download"
    emptyTitle: page.pending > 0 ? I18n.t("Downloading…") : I18n.t("No downloads yet")
    emptyText: Downloads.errorText !== "" ? Downloads.errorText
             : I18n.t("Download YouTube Music songs and playlists from their menus to play them offline")
    emptyActionText: Downloads.failed > 0 ? I18n.t("Retry") : ""
    emptyActionIcon: "refresh"
    onEmptyAction: Downloads.retryFailed()

    function notify(text) {
        const w = page.Window.window
        if (w && typeof w.notify === "function")
            w.notify(text, "", null)
    }

    headerExtra: Component {
        Row {
            spacing: 4
            IconButton {
                visible: Downloads.failed > 0
                icon: "refresh"
                size: "m"
                Accessible.name: I18n.t("Retry failed downloads")
                onClicked: Downloads.retryFailed()
            }
            IconButton {
                visible: page.pending > 0 || Downloads.failed > 0
                icon: "close"
                size: "m"
                Accessible.name: I18n.t("Cancel downloads")
                onClicked: Downloads.cancelAll()
            }
            IconButton {
                icon: "delete_sweep"
                size: "m"
                enabled: Downloads.tracks.count > 0
                Accessible.name: I18n.t("Remove songs not played for 30 days")
                onClicked: page.notify(Downloads.removeForgotten(30))
            }
            IconButton {
                icon: "folder_open"
                size: "m"
                Accessible.name: I18n.t("Downloads folder")
                onClicked: folderDialog.open()
            }
        }
    }

    FolderDialog {
        id: folderDialog
        title: I18n.t("Choose the downloads folder")
        onAccepted: {
            Downloads.setFolder(selectedFolder.toString())
            page.notify(I18n.t("New downloads go to the chosen folder"))
        }
    }
}
