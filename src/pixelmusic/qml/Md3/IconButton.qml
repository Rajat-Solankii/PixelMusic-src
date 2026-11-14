import QtQuick

// Иконка-кнопка MD3 Expressive: style standard | filled | tonal | outlined | pastel | accent | accentSoft | accentGlass,
// (accent* — PixelPlayer's player roles: accent = primary/onPrimary, so deep in light mode and bright in dark mode;
// accentSoft = onPrimary/primary; accentGlass = onPrimary at 70 % with a primary icon, the player's top-bar buttons)
// size xs | s | m | l | xl, width narrow | default | wide. Поддерживает toggle.
Item {
    id: root
    // the colour scheme it draws with: Theme, or PlayerTheme on the player surfaces
    property var scheme: Theme

    property string icon
    property string style: "standard"
    property string size: "s"
    property string widthMode: "default"
    property bool checkable: false
    property bool checked: false
    // false — состояние задаётся только снаружи (checked: binding), клик лишь шлёт clicked()
    property bool autoToggle: true
    property color iconColor: _contentColor
    // Мелким глифам (три точки more_horiz) стандартного кегля и веса мало: в кнопке они теряются
    property real iconSize: _spec.icon
    property real iconWeight: 400
    // FILL axis of the icon: filled when selected; transport buttons ask for filled always
    property real iconFill: _selected ? 1 : 0
    readonly property alias pressed: area.pressed
    readonly property alias hovered: area.containsMouse

    signal clicked()

    readonly property var _spec: ({
        xs: { h: 32,  icon: 20, narrow: 28,  wide: 40,  pressedR: 8,  squareR: 12 },
        s:  { h: 40,  icon: 24, narrow: 32,  wide: 52,  pressedR: 8,  squareR: 12 },
        m:  { h: 56,  icon: 24, narrow: 48,  wide: 72,  pressedR: 12, squareR: 16 },
        l:  { h: 96,  icon: 32, narrow: 64,  wide: 128, pressedR: 16, squareR: 28 },
        xl: { h: 136, icon: 40, narrow: 104, wide: 184, pressedR: 16, squareR: 28 }
    })[size]

    readonly property bool _selected: checkable && checked

    // the style's container; a place that needs other roles (the player pill over primaryContainer) sets its own
    property color containerColor: _styleContainer
    readonly property color _styleContainer: {
        if (!enabled && style === "pastel")
            return Qt.alpha(root.scheme.tertiaryContainer, 0.5)
        if (!enabled && style === "accent")
            return Qt.alpha(root.scheme.primary, 0.38)
        if (!enabled && (style === "accentSoft" || style === "accentGlass"))
            return Qt.alpha(root.scheme.fgPrimary, 0.38)
        if (!enabled)
            return style === "standard" || style === "outlined" ? "transparent" : Qt.alpha(root.scheme.fgSurface, Theme.stateLayer.disabledContainer)
        switch (style) {
        // a solid pastel (PixelPlayer's transport): tertiary container; unchecked toggles — the highest surface
        case "pastel": return checkable && !checked ? root.scheme.surfaceContainerHighest : root.scheme.tertiaryContainer
        case "accent": return root.scheme.primary
        case "accentSoft": return root.scheme.fgPrimary
        case "accentGlass": return Qt.alpha(root.scheme.fgPrimary, 0.7)
        case "filled": return checkable && !checked ? root.scheme.surfaceContainer : root.scheme.primary
        // Compose's FilledTonalIconButton (PixelPlayer Desktop): secondaryContainer
        case "tonal": return checkable && !checked ? root.scheme.surfaceContainerHighest : root.scheme.secondaryContainer
        case "outlined": return _selected ? root.scheme.inverseSurface : "transparent"
        // Выбранная standard-кнопка — с подложкой: одного цвета иконки мало. PixelPlayer Desktop's toggles
        // (shuffle, repeat) are primary / onPrimary when on.
        default: return _selected ? root.scheme.primary : "transparent"
        }
    }
    readonly property color _contentColor: {
        if (!enabled && style === "pastel")
            return Qt.alpha(root.scheme.fgTertiaryContainer, 0.5)
        if (!enabled && style === "accent")
            return Qt.alpha(root.scheme.fgPrimary, 0.6)
        if (!enabled && (style === "accentSoft" || style === "accentGlass"))
            return Qt.alpha(root.scheme.primary, 0.5)
        if (!enabled)
            return Qt.alpha(root.scheme.fgSurface, Theme.stateLayer.disabledContent)
        switch (style) {
        case "pastel": return checkable && !checked ? root.scheme.fgSurface : root.scheme.fgTertiaryContainer
        case "accent": return root.scheme.fgPrimary
        case "accentSoft":
        case "accentGlass": return root.scheme.primary
        case "filled": return checkable && !checked ? root.scheme.fgSurfaceVariant : root.scheme.fgPrimary
        case "tonal": return checkable && !checked ? root.scheme.fgSurfaceVariant : root.scheme.fgSecondaryContainer
        case "outlined": return _selected ? root.scheme.fgInverseSurface : root.scheme.fgSurfaceVariant
        default: return _selected ? root.scheme.fgPrimary : root.scheme.fgSurfaceVariant
        }
    }

    property real cornerRadius: area.pressed ? _spec.pressedR : _selected ? _spec.squareR : height / 2
    Behavior on cornerRadius {
        NumberAnimation { duration: Theme.motion.spatialFast; easing.type: Easing.OutBack; easing.overshoot: Theme.motion.overshoot }
    }

    implicitHeight: _spec.h
    implicitWidth: widthMode === "narrow" ? _spec.narrow : widthMode === "wide" ? _spec.wide : _spec.h

    Rectangle {
        id: container
        anchors.fill: parent
        radius: Math.min(root.cornerRadius, height / 2)
        color: root.containerColor
        border.width: root.style === "outlined" && !root._selected ? 1 : 0
        border.color: root.scheme.outlineVariant
        Behavior on color { enabled: !Theme.transitioning; ColorAnimation { duration: Theme.motion.effectsDefault } }
    }

    Icon {
        anchors.centerIn: parent
        name: root.icon
        size: root.iconSize
        weight: root.iconWeight
        fill: root.iconFill
        color: root.iconColor
    }

    StateLayer {
        id: area
        anchors.fill: parent
        enabled: root.enabled
        radius: container.radius
        color: root.iconColor
        onClicked: {
            if (root.checkable && root.autoToggle)
                root.checked = !root.checked
            root.clicked()
        }
    }
}
