import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core

// Equalizer (soundwave1 EqualizerPanel): switch, preset, preamp, ten bands, balance and mono.
// Extras writes the eq* settings; the shared equalizer of both lanes (Spotify and YouTube) follows them at once.
SettingsCard {
    id: root
    title: I18n.t("Equalizer")
    icon: "equalizer"

    property bool compact: false

    function dbText(v) {
        const r = Math.round(v * 10) / 10
        return (r > 0 ? "+" : "") + I18n.t("{0} dB", r)
    }

    SettingRow {
        title: I18n.t("Equalizer")
        description: I18n.t("Shapes the sound of everything played on this computer, Spotify and YouTube alike")
        Switch { checked: Extras.eqEnabled; onToggled: c => Extras.eqEnabled = c }
    }

    SettingRow {
        title: I18n.t("Preset")
        enabled: Extras.eqEnabled
        opacity: enabled ? 1 : 0.5
        MenuField {
            width: root.compact ? 200 : 240
            leadingIcon: "tune"
            placeholderText: I18n.t("Custom")
            model: Extras.eqPresetNames.map(name => ({ id: name, text: I18n.t(name) }))
            currentValue: Extras.eqPreset
            onActivated: v => Extras.applyPreset(v)
        }
    }

    SettingRow {
        title: I18n.t("Preamp")
        description: I18n.t("Lower it if boosted bands make the sound distort")
        enabled: Extras.eqEnabled
        opacity: enabled ? 1 : 0.5
        Slider {
            wheelEnabled: false     // the wheel scrolls the page, not the setting
            width: 220
            from: -12
            to: 12
            stepSize: 0.5
            value: Extras.eqPreamp
            valueText: v => root.dbText(v)
            onMoved: v => Extras.eqPreamp = v
        }
    }

    // ten bands, two columns when there is room
    GridLayout {
        Layout.fillWidth: true
        enabled: Extras.eqEnabled
        opacity: enabled ? 1 : 0.5
        columns: root.compact ? 1 : 2
        columnSpacing: 24
        rowSpacing: 4

        Repeater {
            model: Extras.eqFrequencies
            delegate: RowLayout {
                id: band
                required property string modelData
                required property int index
                Layout.fillWidth: true
                spacing: 12
                Label {
                    Layout.preferredWidth: 56
                    text: I18n.t("{0} Hz", band.modelData)
                    type: "labelLarge"
                    color: Theme.fgSurfaceVariant
                }
                Slider {
                    Layout.fillWidth: true
                    wheelEnabled: false
                    from: -12
                    to: 12
                    stepSize: 0.5
                    value: Extras.eqBands[band.index] ?? 0
                    valueText: v => root.dbText(v)
                    onMoved: v => Extras.setBand(band.index, v)
                }
            }
        }
    }

    SettingRow {
        title: I18n.t("Balance")
        description: I18n.t("Works with the equalizer off too")
        Slider {
            wheelEnabled: false
            width: 220
            from: -1
            to: 1
            stepSize: 0.05
            value: Extras.audioBalance
            valueText: v => Math.abs(v) < 0.03 ? I18n.t("Center")
                          : v < 0 ? I18n.t("L {0}%", Math.round(-v * 100)) : I18n.t("R {0}%", Math.round(v * 100))
            onMoved: v => Extras.audioBalance = v
        }
    }

    SettingRow {
        title: I18n.t("Mono audio")
        description: I18n.t("Both speakers play the same sound")
        Switch { checked: Extras.monoAudio; onToggled: c => Extras.monoAudio = c }
    }

    SettingRow {
        title: I18n.t("Reset")
        description: I18n.t("Flat bands, no preamp, centered balance")
        Button {
            text: I18n.t("Reset")
            icon: "restart_alt"
            style: "outlined"
            size: "xs"
            onClicked: Extras.resetEq()
        }
    }
}
