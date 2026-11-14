pragma Singleton
import QtQuick
import PixelMusic.Core
import "ColorMix.js" as Mix

// The colours of the player surfaces (mini player, full player). Same names as Theme. While the app keeps its own
// accent ("Color from cover" off) and Settings → Appearance → "Player follows the cover" is on, they are the
// playing cover's scheme (ThemeEngine.playerColors), changing with one OKLab blend like Theme's; otherwise each one
// is Theme's own colour, so the app's transitions and wipes carry the player along.
QtObject {
    id: root

    readonly property bool separate: ThemeEngine.playerSeparate
    readonly property bool dark: Theme.dark

    readonly property var _roles: ({
        background: "background",
        fgBackground: "onBackground",
        surface: "surface",
        surfaceDim: "surfaceDim",
        surfaceBright: "surfaceBright",
        surfaceContainerLowest: "surfaceContainerLowest",
        surfaceContainerLow: "surfaceContainerLow",
        surfaceContainer: "surfaceContainer",
        surfaceContainerHigh: "surfaceContainerHigh",
        surfaceContainerHighest: "surfaceContainerHighest",
        fgSurface: "onSurface",
        surfaceVariant: "surfaceVariant",
        fgSurfaceVariant: "onSurfaceVariant",
        outline: "outline",
        outlineVariant: "outlineVariant",
        inverseSurface: "inverseSurface",
        fgInverseSurface: "inverseOnSurface",
        shadow: "shadow",
        scrim: "scrim",
        surfaceTint: "surfaceTint",
        primary: "primary",
        primaryDim: "primaryDim",
        fgPrimary: "onPrimary",
        primaryContainer: "primaryContainer",
        fgPrimaryContainer: "onPrimaryContainer",
        inversePrimary: "inversePrimary",
        primaryFixed: "primaryFixed",
        primaryFixedDim: "primaryFixedDim",
        fgPrimaryFixed: "onPrimaryFixed",
        fgPrimaryFixedVariant: "onPrimaryFixedVariant",
        secondary: "secondary",
        secondaryDim: "secondaryDim",
        fgSecondary: "onSecondary",
        secondaryContainer: "secondaryContainer",
        fgSecondaryContainer: "onSecondaryContainer",
        secondaryFixed: "secondaryFixed",
        secondaryFixedDim: "secondaryFixedDim",
        fgSecondaryFixed: "onSecondaryFixed",
        fgSecondaryFixedVariant: "onSecondaryFixedVariant",
        tertiary: "tertiary",
        tertiaryDim: "tertiaryDim",
        fgTertiary: "onTertiary",
        tertiaryContainer: "tertiaryContainer",
        fgTertiaryContainer: "onTertiaryContainer",
        tertiaryFixed: "tertiaryFixed",
        tertiaryFixedDim: "tertiaryFixedDim",
        fgTertiaryFixed: "onTertiaryFixed",
        fgTertiaryFixedVariant: "onTertiaryFixedVariant",
        error: "error",
        errorDim: "errorDim",
        fgError: "onError",
        errorContainer: "errorContainer",
        fgErrorContainer: "onErrorContainer",
        primaryPaletteKeyColor: "primaryPaletteKeyColor",
        secondaryPaletteKeyColor: "secondaryPaletteKeyColor",
        tertiaryPaletteKeyColor: "tertiaryPaletteKeyColor",
        neutralPaletteKeyColor: "neutralPaletteKeyColor",
        neutralVariantPaletteKeyColor: "neutralVariantPaletteKeyColor",
        errorPaletteKeyColor: "errorPaletteKeyColor"
    })

    property var _fromLab: ({})
    property var _toLab: ({})
    property real _p: 1
    // the colours shown last: a change of scheme blends from them, also from Theme's while it was not separate
    property bool _wasSeparate: false

    function _c(name) {
        if (!separate)
            return Theme[name]
        const key = _roles[name]
        const b = _toLab[key]
        if (!b)
            return ThemeEngine.playerColors[key] ?? Theme[name]
        return Mix.mix(_fromLab[key] ?? b, b, _p)
    }

    function _changed() {
        const from = {}
        for (const name in _roles)
            from[_roles[name]] = Mix.toLab(root[name])
        _fromLab = from
        _toLab = Mix.labMap(ThemeEngine.playerColors)
        _p = 0
        if (Theme.lowPower || Theme.reducedMotion)
            _p = 1
        else
            _anim.restart()
    }

    readonly property Connections _watch: Connections {
        target: ThemeEngine
        function onPlayerSchemeChanged() { root._changed() }
    }

    readonly property NumberAnimation _anim: NumberAnimation {
        target: root
        property: "_p"
        from: 0
        to: 1
        duration: Theme.colorDuration
        easing.type: Easing.InOutCubic
    }

    Component.onCompleted: {
        const lab = Mix.labMap(ThemeEngine.playerColors)
        _fromLab = lab
        _toLab = lab
    }

    readonly property color background: _c("background")
    readonly property color fgBackground: _c("fgBackground")
    readonly property color surface: _c("surface")
    readonly property color surfaceDim: _c("surfaceDim")
    readonly property color surfaceBright: _c("surfaceBright")
    readonly property color surfaceContainerLowest: _c("surfaceContainerLowest")
    readonly property color surfaceContainerLow: _c("surfaceContainerLow")
    readonly property color surfaceContainer: _c("surfaceContainer")
    readonly property color surfaceContainerHigh: _c("surfaceContainerHigh")
    readonly property color surfaceContainerHighest: _c("surfaceContainerHighest")
    readonly property color fgSurface: _c("fgSurface")
    readonly property color surfaceVariant: _c("surfaceVariant")
    readonly property color fgSurfaceVariant: _c("fgSurfaceVariant")
    readonly property color outline: _c("outline")
    readonly property color outlineVariant: _c("outlineVariant")
    readonly property color inverseSurface: _c("inverseSurface")
    readonly property color fgInverseSurface: _c("fgInverseSurface")
    readonly property color shadow: _c("shadow")
    readonly property color scrim: _c("scrim")
    readonly property color surfaceTint: _c("surfaceTint")
    readonly property color primary: _c("primary")
    readonly property color primaryDim: _c("primaryDim")
    readonly property color fgPrimary: _c("fgPrimary")
    readonly property color primaryContainer: _c("primaryContainer")
    readonly property color fgPrimaryContainer: _c("fgPrimaryContainer")
    readonly property color inversePrimary: _c("inversePrimary")
    readonly property color primaryFixed: _c("primaryFixed")
    readonly property color primaryFixedDim: _c("primaryFixedDim")
    readonly property color fgPrimaryFixed: _c("fgPrimaryFixed")
    readonly property color fgPrimaryFixedVariant: _c("fgPrimaryFixedVariant")
    readonly property color secondary: _c("secondary")
    readonly property color secondaryDim: _c("secondaryDim")
    readonly property color fgSecondary: _c("fgSecondary")
    readonly property color secondaryContainer: _c("secondaryContainer")
    readonly property color fgSecondaryContainer: _c("fgSecondaryContainer")
    readonly property color secondaryFixed: _c("secondaryFixed")
    readonly property color secondaryFixedDim: _c("secondaryFixedDim")
    readonly property color fgSecondaryFixed: _c("fgSecondaryFixed")
    readonly property color fgSecondaryFixedVariant: _c("fgSecondaryFixedVariant")
    readonly property color tertiary: _c("tertiary")
    readonly property color tertiaryDim: _c("tertiaryDim")
    readonly property color fgTertiary: _c("fgTertiary")
    readonly property color tertiaryContainer: _c("tertiaryContainer")
    readonly property color fgTertiaryContainer: _c("fgTertiaryContainer")
    readonly property color tertiaryFixed: _c("tertiaryFixed")
    readonly property color tertiaryFixedDim: _c("tertiaryFixedDim")
    readonly property color fgTertiaryFixed: _c("fgTertiaryFixed")
    readonly property color fgTertiaryFixedVariant: _c("fgTertiaryFixedVariant")
    readonly property color error: _c("error")
    readonly property color errorDim: _c("errorDim")
    readonly property color fgError: _c("fgError")
    readonly property color errorContainer: _c("errorContainer")
    readonly property color fgErrorContainer: _c("fgErrorContainer")
    readonly property color primaryPaletteKeyColor: _c("primaryPaletteKeyColor")
    readonly property color secondaryPaletteKeyColor: _c("secondaryPaletteKeyColor")
    readonly property color tertiaryPaletteKeyColor: _c("tertiaryPaletteKeyColor")
    readonly property color neutralPaletteKeyColor: _c("neutralPaletteKeyColor")
    readonly property color neutralVariantPaletteKeyColor: _c("neutralVariantPaletteKeyColor")
    readonly property color errorPaletteKeyColor: _c("errorPaletteKeyColor")
    // the players' play/pause, as Theme.playContainer
    readonly property color playContainer: dark ? tertiaryFixedDim : Qt.darker(tertiaryFixedDim, 1.08)

    // Light mode transport (mini and full player): onPrimary pills with a primary icon around a solid primary
    // play button; dark mode keeps primary pills around the tertiary-fixed play button
    readonly property string transportStyle: dark ? "accent" : "accentSoft"
    readonly property color playButtonContainer: dark ? playContainer : primary
    readonly property color playButtonIcon: dark ? fgTertiaryFixed : fgPrimary
    // shuffle / repeat when off: a deeper tone of the sheet's own colour in light mode (onSurfaceVariant greys out)
    readonly property color toggleIdle: dark ? fgSurfaceVariant : primary
}
