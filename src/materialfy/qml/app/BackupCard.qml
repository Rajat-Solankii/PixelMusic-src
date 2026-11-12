import QtQuick
import QtQuick.Layouts
import QtQuick.Dialogs
import Md3
import Materialfy.Core

// Backup (soundwave1 BackupPanel): settings, listening history and stats in one JSON file. Sign-ins, cookies and
// keys stay in the credential store and are never written to it. The result comes as a toast (Main.qml).
SettingsCard {
    id: root
    title: I18n.t("Backup")
    icon: "restore"

    SettingRow {
        title: I18n.t("Back up settings")
        description: I18n.t("Settings, listening history and stats in one file. Sign-ins, cookies and keys are not included")
        Button {
            text: I18n.t("Export")
            icon: "save"
            style: "outlined"
            size: "xs"
            enabled: !Extras.busy
            onClicked: {
                const folder = exportDialog.currentFolder.toString()
                if (folder !== "")
                    exportDialog.selectedFile = folder + "/" + Extras.backupFileName()
                exportDialog.open()
            }
        }
    }

    SettingRow {
        title: I18n.t("Restore from a backup")
        description: I18n.t("Replaces these settings and the history on this computer")
        Button {
            text: I18n.t("Import")
            icon: "download"
            style: "outlined"
            size: "xs"
            enabled: !Extras.busy
            onClicked: importDialog.open()
        }
    }

    FileDialog {
        id: exportDialog
        title: I18n.t("Save backup")
        fileMode: FileDialog.SaveFile
        defaultSuffix: "json"
        nameFilters: [I18n.t("Backup ({0})", "*.json")]
        onAccepted: Extras.exportBackup(selectedFile.toString())
    }

    FileDialog {
        id: importDialog
        title: I18n.t("Open backup")
        fileMode: FileDialog.OpenFile
        nameFilters: [I18n.t("Backup ({0})", "*.json")]
        onAccepted: Extras.importBackup(selectedFile.toString())
    }
}
