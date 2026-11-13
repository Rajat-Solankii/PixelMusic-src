import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Настройки → «О программе» → «Обновления», строки в стиле PixelPlayer (круглая пастельная иконка, подпись,
// контрол справа): проверка (итог + «Проверить сейчас» с индикатором), автоматическая проверка, канал
// (стабильные / бета). Само обновление — в UpdateSheet. Текущая версия — в чипе AboutHeader.
SettingsCard {
    id: root

    property bool compact: false

    readonly property string st: Updater.state
    readonly property bool hasOffer: Updater.version !== ""

    readonly property string status: {
        switch (st) {
        case "checking": return I18n.t("Checking for updates…")
        case "upToDate": return I18n.t("You’re up to date")
        case "available": return Updater.skipped ? I18n.t("Version {0} is available (skipped)", Updater.version)
                                                 : I18n.t("Version {0} is available", Updater.version)
        case "downloading": return I18n.t("Downloading version {0} · {1} %", Updater.version, Math.floor(Updater.progress * 100))
        case "verifying": return I18n.t("Verifying version {0}…", Updater.version)
        case "readyToInstall": return I18n.t("Version {0} is ready to install", Updater.version)
        case "installing": return I18n.t("Installing version {0}…", Updater.version)
        case "error": return Updater.errorText
        default: return Updater.autoCheck ? I18n.t("Not checked yet") : I18n.t("Automatic checks are off")
        }
    }
    readonly property bool busy: st === "checking" || st === "downloading" || st === "verifying"
                                 || st === "installing" || st === "readyToInstall"

    function _lower(text) { return text.charAt(0).toLowerCase() + text.slice(1) }

    title: I18n.t("Updates")
    icon: "download"
    grouped: true

    SettingsCategoryRow {
        first: true
        title: I18n.t("Pixel Music {0}", Updater.currentVersion)
        subtitle: root.status
                  + (Updater.lastChecked > 0 && root.st !== "checking" && root.st !== "error"
                     ? " · " + I18n.t("checked {0}", root._lower(I18n.relative(Updater.lastChecked))) : "")
        icon: root.hasOffer ? "download" : "sync"
        tone: root.hasOffer ? "tertiary" : "primary"
        trailingIcon: ""
        // the whole row checks too, as in PixelPlayer; the buttons stay for the state they show
        clickable: !root.busy
        onClicked: root.hasOffer ? Updater.showSheet() : Updater.check()

        Row {
            spacing: 8

            LoadingIndicator {
                visible: root.st === "checking"
                size: root.compact ? 32 : 40
                anchors.verticalCenter: parent.verticalCenter
            }
            Button {
                visible: !root.compact || !root.hasOffer
                enabled: !root.busy
                text: root.st === "checking" ? I18n.t("Checking…") : I18n.t("Check now")
                icon: root.st === "checking" ? "" : "refresh"
                style: root.hasOffer ? "outlined" : "tonal"
                size: root.compact ? "xs" : "s"
                anchors.verticalCenter: parent.verticalCenter
                onClicked: Updater.check()
            }
            Button {
                visible: root.hasOffer && root.st !== "checking"
                text: root.st === "readyToInstall" ? I18n.t("Install") : I18n.t("View update")
                size: root.compact ? "xs" : "s"
                anchors.verticalCenter: parent.verticalCenter
                onClicked: Updater.showSheet()
            }
        }
    }

    SettingsCategoryRow {
        title: I18n.t("Check for updates automatically")
        subtitle: I18n.t("At startup and every 6 hours. Only the public release list on GitHub is requested")
        icon: "schedule"
        tone: "secondary"
        trailingIcon: ""
        onClicked: Updater.autoCheck = !Updater.autoCheck
        Switch { checked: Updater.autoCheck; onToggled: c => Updater.autoCheck = c }
    }

    SettingsCategoryRow {
        last: true
        title: I18n.t("Update channel")
        subtitle: Updater.channel === "beta"
                  ? I18n.t("Beta: pre-release versions too. New features arrive earlier, but may be less stable")
                  : I18n.t("Stable: only finished releases")
        icon: "new_releases"
        tone: "tertiary"
        trailingIcon: ""
        clickable: false
        ButtonGroup {
            model: [I18n.t("Stable"), I18n.t("Beta")]
            autoSelect: false
            size: root.compact ? "xs" : "s"
            currentIndex: Updater.channel === "beta" ? 1 : 0
            onActivated: i => Updater.channel = ["stable", "beta"][i]
        }
    }
}
