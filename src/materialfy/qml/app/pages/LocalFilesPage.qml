import QtQuick
import QtQuick.Dialogs
import Md3
import Materialfy.Core
import ".."

// Music files on this computer (LocalFiles): the folders chosen here or in Settings are scanned for music;
// the tracks play on this computer through the same player as YouTube Music.
TrackListPage {
    id: page
    title: I18n.t("Local files")
    overline: I18n.t("COLLECTION")
    heroIcon: "folder_open"
    heroShape: "cookie9"
    subtitle: LocalFiles.scanning ? I18n.t("Scanning… {0} of {1}", LocalFiles.read, LocalFiles.found)
            : LocalFiles.folders.length === 0 ? I18n.t("Music on this computer")
            : I18n.n(LocalFiles.tracks.count, "{0} track", "{0} tracks")
    model: LocalFiles.tracks
    readonly property bool noFolders: LocalFiles.folders.length === 0
    emptyIcon: LocalFiles.errorText !== "" ? "error" : "folder_open"
    emptyTitle: noFolders ? I18n.t("Add a music folder") : LocalFiles.scanning ? I18n.t("Scanning…") : I18n.t("No music found")
    emptyText: LocalFiles.errorText !== "" ? LocalFiles.errorText
             : noFolders ? I18n.t("Choose folders with music files (MP3, FLAC, M4A, WAV, OGG)")
             : I18n.t("No music files in the chosen folders")
    emptyActionText: I18n.t("Add folder")
    emptyActionIcon: "add"
    onEmptyAction: folderDialog.open()

    headerExtra: Component {
        Row {
            spacing: 4
            IconButton {
                icon: "add"
                size: "m"
                Accessible.name: I18n.t("Add folder")
                onClicked: folderDialog.open()
            }
            IconButton {
                icon: "refresh"
                size: "m"
                enabled: !LocalFiles.scanning && LocalFiles.folders.length > 0
                Accessible.name: I18n.t("Rescan")
                onClicked: LocalFiles.rescan()
            }
        }
    }

    FolderDialog {
        id: folderDialog
        title: I18n.t("Choose a music folder")
        onAccepted: LocalFiles.addFolder(selectedFolder.toString())
    }
}
