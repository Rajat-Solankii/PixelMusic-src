import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Sleep timer (soundwave1 SleepTimerButton): pause after N minutes or when the track ends, with a 10 s fade.
// Pauses whatever plays on this computer or a Connect device, Spotify and YouTube alike.
SettingsCard {
    id: root
    title: I18n.t("Sleep timer")
    icon: "bedtime"

    property bool compact: false
    readonly property var minutes: [15, 30, 45, 60, 90]

    function remainingText(s) {
        const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60
        const mm = h > 0 ? String(m).padStart(2, "0") : String(m)
        return (h > 0 ? h + ":" : "") + mm + ":" + String(sec).padStart(2, "0")
    }

    SettingRow {
        title: I18n.t("Pause playback")
        description: !Extras.sleepActive ? I18n.t("The volume fades out for 10 seconds, then playback pauses")
                   : Extras.sleepEndOfTrack ? I18n.t("Pauses when this track ends")
                   : I18n.t("Pauses in {0}", root.remainingText(Extras.sleepRemaining))
        Button {
            visible: Extras.sleepActive
            text: I18n.t("Turn off")
            icon: "close"
            style: "text"
            size: "xs"
            onClicked: Extras.cancelSleep()
        }
    }

    Flow {
        Layout.fillWidth: true
        spacing: 8
        Repeater {
            model: root.minutes
            delegate: Chip {
                required property int modelData
                text: I18n.t("{0} min", modelData)
                onClicked: Extras.setSleepMinutes(modelData)
            }
        }
        Chip {
            text: I18n.t("End of track")
            icon: "music_note"
            selected: Extras.sleepEndOfTrack
            onClicked: Extras.sleepAtEndOfTrack()
        }
    }
}
