import QtQuick
import QtQuick.Layouts
import QtQuick.Dialogs
import Md3
import PixelMusic.Core
import ".."

// Настройки: аккаунт и разрешения, воспроизведение, Spotify Connect, сеть, хранилище, внешний вид.
// Каждая строка пишет в Settings сразу; прокси — черновиком формы и кнопкой «Применить».
Page {
    id: page
    title: activeCategory === "" ? I18n.t("Settings") : (categoryInfo(activeCategory).text ?? I18n.t("Settings"))

    property string section           // раздел из маршрута: прокрутить к нему при открытии
    // PixelPlayer-style: the page without a category is the list of categories; a category (a route of its own,
    // so back/forward work) shows only its cards. An old route with a section opens that section's category.
    property string category
    readonly property string activeCategory: category !== "" ? category : section !== "" ? categoryOf(section) : ""
    function shows(key) { return activeCategory === key }
    property string query: ""

    readonly property var categories: [
        { key: "accounts", text: I18n.t("Accounts"), hint: I18n.t("YouTube Music and Last.fm"), icon: "account_circle", tone: "primary", hue: 255,
          words: "account sign in sign out youtube last.fm lastfm scrobble cookie login" },
        { key: "music", text: I18n.t("Music management"), hint: I18n.t("Music folders, storage and cache"), icon: "library_music", tone: "secondary", hue: 200,
          words: "local files folders storage cache downloads disk" },
        { key: "appearance", text: I18n.t("Appearance"), hint: I18n.t("Themes, colors, layout and visual styles"), icon: "palette", tone: "tertiary", hue: 330,
          words: "theme dark light color colour cover accent scale language font blur background island taskbar window animations palette" },
        { key: "playback", text: I18n.t("Playback"), hint: I18n.t("Audio, sources, autoplay and sleep timer"), icon: "music_note", tone: "primary", hue: 0,
          words: "audio output device lossless monochrome jiosaavn autoplay normalize volume sleep timer quality" },
        { key: "equalizer", text: I18n.t("Equalizer"), hint: I18n.t("Bands, preamp and audio presets"), icon: "graphic_eq", tone: "secondary", hue: 38,
          words: "equalizer eq bass treble preamp preset" },
        { key: "lyrics", text: I18n.t("Lyrics"), hint: I18n.t("Synced lyrics, romanization, size and style"), icon: "lyrics", tone: "tertiary", hue: 285,
          words: "lyrics synced romanization romanized translation size reading karaoke" },
        { key: "network", text: I18n.t("Network"), hint: I18n.t("Proxy and connections"), icon: "lan", tone: "primary", hue: 175,
          words: "network proxy connection lan http socks" },
        { key: "stats", text: I18n.t("Listening stats"), hint: I18n.t("Your listening history"), icon: "insights", tone: "secondary", hue: 115,
          words: "stats history listening played" },
        { key: "backup", text: I18n.t("Backup & restore"), hint: I18n.t("Export and recover your personal data"), icon: "settings_backup_restore", tone: "tertiary", hue: 228,
          words: "backup restore export import" },
        { key: "about", text: I18n.t("About"), hint: I18n.t("App info, version, updates and credits"), icon: "info", tone: "primary", hue: 55,
          words: "about version updates update licence license credits" }
    ]
    // categories opened from inside another one (not in the list; back returns to the parent)
    readonly property var subCategories: [
        { key: "lastfm", text: I18n.t("Last.fm Integration"), parent: "accounts" }
    ]
    function categoryInfo(key) { return categories.find(c => c.key === key) ?? subCategories.find(c => c.key === key) ?? ({}) }
    // the old sections (routes from elsewhere in the app) → their category
    function categoryOf(section) {
        switch (section) {
        case "account": return "accounts"
        case "window": return "appearance"
        case "reading": return "lyrics"
        case "sleep": return "playback"
        case "storage": return "music"
        case "updates": return "about"
        }
        return categoryInfo(section).key ? section : ""
    }
    readonly property var shownCategories: {
        const q = query.trim().toLowerCase()
        if (q === "")
            return categories
        return categories.filter(c => (c.text + " " + c.hint + " " + c.words).toLowerCase().indexOf(q) >= 0)
    }
    function openCategory(key) {
        if (page.router)
            page.router.push("settings", { category: key })
        else
            category = key
    }

    readonly property bool signedIn: Auth.state === "signedIn"
    readonly property var account: Auth.account ?? ({})
    readonly property bool compact: width < 760

    readonly property var sections: [
        { key: "account", text: I18n.t("Account"), icon: "account_circle" },
        { key: "appearance", text: I18n.t("Appearance"), icon: "palette" },
        { key: "lyrics", text: I18n.t("Lyrics"), icon: "lyrics" },
        { key: "reading", text: I18n.t("Reading"), icon: "visibility" },
        { key: "playback", text: I18n.t("Playback"), icon: "graphic_eq" },
        { key: "equalizer", text: I18n.t("Equalizer"), icon: "equalizer" },
        { key: "network", text: I18n.t("Network"), icon: "lan" },
        { key: "storage", text: I18n.t("Storage"), icon: "database" },
        { key: "stats", text: I18n.t("Listening stats"), icon: "history" },
        { key: "backup", text: I18n.t("Backup"), icon: "restore" },
        { key: "updates", text: I18n.t("Updates"), icon: "download" }
    ]
    property string currentSection: "account"
    // The scale is read by Qt at launch: after a change, offer a one-click relaunch.
    // The launch scale comes from the process, not the page, which is rebuilt.
    readonly property bool scalePending: Settings.uiScale !== Settings.uiScaleAtLaunch()

    readonly property int screenCount: Qt.application.screens ? Qt.application.screens.length : 1

    function isScreenActive(settingVal, screenIdx, totalScreens) {
        if (!settingVal || settingVal === "" || settingVal === "0")
            return screenIdx === 0
        if (settingVal === "all")
            return true
        const parts = settingVal.split(",")
        return parts.map(p => parseInt(p.trim(), 10)).indexOf(screenIdx) !== -1
    }

    function isAllScreensActive(settingVal, totalScreens) {
        if (settingVal === "all")
            return true
        if (totalScreens <= 1)
            return true
        for (let i = 0; i < totalScreens; ++i) {
            if (!page.isScreenActive(settingVal, i, totalScreens))
                return false
        }
        return true
    }

    function toggleScreenSetting(settingVal, screenIdx, totalScreens) {
        if (totalScreens <= 1)
            return "all"
        if (settingVal === "all")
            return String(screenIdx)
        let active = []
        if (!settingVal || settingVal === "" || settingVal === "0") {
            active = [0]
        } else {
            active = settingVal.split(",").map(p => parseInt(p.trim(), 10)).filter(n => !isNaN(n) && n >= 0 && n < totalScreens)
        }

        const pos = active.indexOf(screenIdx)
        if (pos !== -1) {
            if (active.length > 1)
                active.splice(pos, 1)
        } else {
            active.push(screenIdx)
            active.sort((a, b) => a - b)
        }

        if (active.length === totalScreens)
            return "all"
        return active.join(",")
    }

    function getScreenTaskbarPos(settingMapStr, fallbackPos, screenIdx) {
        if (settingMapStr && settingMapStr !== "") {
            try {
                const map = JSON.parse(settingMapStr)
                if (map && map[String(screenIdx)] !== undefined)
                    return map[String(screenIdx)]
            } catch (e) {}
        }
        return fallbackPos || "tray"
    }

    function setScreenTaskbarPos(screenIdx, pos) {
        let map = {}
        if (Settings.taskbarPositions !== "") {
            try { map = JSON.parse(Settings.taskbarPositions) } catch (e) {}
        }
        map[String(screenIdx)] = pos
        Settings.taskbarPositions = JSON.stringify(map)
        if (screenIdx === 0)
            Settings.taskbarWidgetPos = pos
    }

    function getActiveTaskbarScreenIndices(settingVal, totalScreens) {
        const res = []
        for (let i = 0; i < totalScreens; ++i) {
            if (page.isScreenActive(settingVal, i, totalScreens))
                res.push(i)
        }
        return res.length > 0 ? res : [0]
    }

    readonly property var backdropModes: [
        { mode: "gauss", text: I18n.t("Soft"), hint: I18n.t("Even blur with no blotches or banding: the cover melts smoothly into color") },
        { mode: "glass", text: I18n.t("Frosted glass"), hint: I18n.t("The cover shows through as if behind frosted glass, with fine grain on top") },
        { mode: "palette", text: I18n.t("Gradient"), hint: I18n.t("No cover: a smooth gradient of theme colors") },
        { mode: "blobs", text: I18n.t("Blobs"), hint: I18n.t("Large color blobs") }
    ]

    readonly property var transitions: [
        { mode: "wipe", text: I18n.t("Wipe"), hint: I18n.t("New colors and background slide in from the side: left to right for the next track, right to left for the previous one") },
        { mode: "ripple", text: I18n.t("Ripple from cover"), hint: I18n.t("New colors and background spread out from the cover in a wavy-edged shape") },
        { mode: "dissolve", text: I18n.t("Dissolve"), hint: I18n.t("Old colors and background gently fade away, revealing the new ones") },
        { mode: "liquid", text: I18n.t("Liquid"), hint: I18n.t("The old image flows and melts away in an uneven wave toward the transition") }
    ]

    readonly property var variantNames: ({
        tonalSpot: I18n.t("Calm"), neutral: I18n.t("Neutral"), vibrant: I18n.t("Vibrant"),
        expressive: I18n.t("Expressive"), monochrome: I18n.t("Monochrome")
    })

    // ---- Прокрутка к разделам ----
    // The cards of a category live in that category's Loader (built only while it is shown): the section → the
    // loaded card, or null while its category is not on screen
    function cardFor(key) {
        const loader = ({ accounts: accountsLoader, music: musicLoader, appearance: appearanceLoader, lyrics: lyricsLoader,
                          playback: playbackLoader, equalizer: equalizerLoader, network: networkLoader, stats: statsLoader,
                          backup: backupLoader, about: aboutLoader })[categoryOf(key)]
        return loader && loader.item ? (loader.item.cards[key] ?? null) : null
    }
    function _cardTop(card) {
        return card.mapToItem(page.flickable.contentItem, 0, 0).y
    }
    function _cardY(card) {
        return _cardTop(card) - nav.height - 16
    }
    function scrollToSection(key, animated) {
        const card = cardFor(key)
        if (!card)
            return
        currentSection = key
        _following = false
        page.flickable.scrollTo(Math.max(0, _cardY(card)), animated && !Theme.reducedMotion)
        followTimer.restart()
    }
    // во время программной прокрутки выбранный чип не прыгает по промежуточным карточкам
    property bool _following: true
    Timer { id: followTimer; interval: Theme.motion.spatialDefault + 200; onTriggered: page._following = true }
    function _followScroll() {
        if (!_following)
            return
        // строка разделов прилипает сверху и закрывает полосу под собой — как в _cardY
        const viewTop = page.flickable.contentY + nav.height + 16
        const viewBottom = page.flickable.contentY + page.flickable.height
        let cur = currentSection
        let best = 0
        // смотрим на видимую площадь, а не на верх карточки: следующая может начинаться
        // ниже экрана, но уже занимать его почти целиком, пока предыдущая уехала наверх
        for (const s of sections) {
            const card = cardFor(s.key)
            if (!card)
                continue
            const top = _cardTop(card)
            const visible = Math.min(top + card.height, viewBottom) - Math.max(top, viewTop)
            // доля того, что карточка вообще может показать: короткая целиком на экране не проигрывает высокой соседке
            const score = visible / Math.max(1, Math.min(card.height, viewBottom - viewTop))
            if (score > best) {
                best = score
                cur = s.key
            }
        }
        if (page.flickable.contentY <= page.flickable.originY + 2)
            cur = sections[0].key
        if (page.flickable.contentY >= page.flickable.maxY - 2)
            cur = sections[sections.length - 1].key
        currentSection = cur
    }
    Connections {
        target: page.flickable
        function onContentYChanged() { page._followScroll() }
    }

    Component.onCompleted: {
        Settings.refreshAudioDevices()
        Settings.refreshCacheUsage()
        if (signedIn)
            Devices.refresh()
        if (section !== "")
            openSection.start()
    }
    // раскладка карточек готова не сразу после их загрузки (Loader категории — асинхронный)
    Timer { id: openSection; interval: 60; onTriggered: page.scrollToSection(page.section, false) }
    function _cardsLoaded() {
        if (section !== "")
            openSection.restart()
    }

    // ---- Липкий ряд разделов ----
    // ---- Category list (the page without a category) ----
    SearchField {
        id: settingsSearch
        visible: page.activeCategory === ""
        Layout.fillWidth: true
        Layout.maximumWidth: 520
        placeholder: I18n.t("Search settings…")
        onTextChanged: page.query = text
        onAccepted: if (page.shownCategories.length > 0) page.openCategory(page.shownCategories[0].key)
    }
    ColumnLayout {
        visible: page.activeCategory === ""
        Layout.fillWidth: true
        spacing: 3
        Repeater {
            model: page.shownCategories
            SettingsCategoryRow {
                required property var modelData
                required property int index
                title: modelData.text
                subtitle: modelData.hint
                icon: modelData.icon
                tone: modelData.tone
                hue: modelData.hue ?? -1
                first: index === 0
                last: index === page.shownCategories.length - 1
                onClicked: page.openCategory(modelData.key)
            }
        }
        Label {
            visible: page.shownCategories.length === 0
            Layout.fillWidth: true
            Layout.topMargin: 8
            text: I18n.t("No settings match “{0}”", page.query.trim())
            color: Theme.fgSurfaceVariant
        }
    }

    Item {
        id: navHost
        visible: false
        Layout.fillWidth: true
        Layout.topMargin: -8
        implicitHeight: nav.height
        z: 10
        readonly property real stick: Math.max(0, page.flickable.contentY - y - 8)
        transform: Translate { y: navHost.stick }

        Rectangle {
            x: -page.sideMargin
            width: page.width
            y: -8
            height: nav.height + 16
            color: Theme.surfaceContainer     // цвет панели контента (Surface level "container")
            opacity: navHost.stick > 0 ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsFast } }
        }
        SettingsNav {
            id: nav
            width: parent.width
            sections: page.sections
            current: page.currentSection
            onPicked: key => page.scrollToSection(key, true)
        }
    }

    Loader {
        id: accountsLoader
        Layout.fillWidth: true
        active: page.shows("accounts")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        // the redesigned Accounts category (AccountsSettings.qml): YouTube Music, Quick Import, Last.fm
        sourceComponent: AccountsSettings { onOpenLastFm: page.openCategory("lastfm") }
    }

    Loader {
        id: lastfmLoader
        Layout.fillWidth: true
        active: page.shows("lastfm")
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        // Last.fm Integration (LastFmSettings.qml): options, Smart Mix playlists, scrobbling thresholds
        sourceComponent: LastFmSettings { onBack: page.router ? page.router.back() : page.openCategory("accounts") }
    }

    Loader {
        id: musicLoader
        Layout.fillWidth: true
        active: page.shows("music")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ storage: storageCard })
            spacing: 24
            // ================= Local files =================
            // Music folders on this computer (LocalFiles): scanned for music, played on this computer
            SettingsCard {
                id: localFilesCard
                visible: page.shows("music")
                title: I18n.t("Local files")
                icon: "folder_open"

                Repeater {
                    model: LocalFiles.folders
                    delegate: SettingRow {
                        required property string modelData
                        title: modelData
                        IconButton {
                            icon: "delete"
                            size: "s"
                            Accessible.name: I18n.t("Remove folder")
                            onClicked: LocalFiles.removeFolder(modelData)
                        }
                    }
                }

                SettingRow {
                    title: I18n.t("Music folders")
                    description: LocalFiles.scanning ? I18n.t("Scanning… {0} of {1}", LocalFiles.read, LocalFiles.found)
                               : LocalFiles.errorText !== "" ? LocalFiles.errorText
                               : I18n.t("MP3, FLAC, M4A, WAV and OGG files in these folders appear under Local files")
                    // the row's slot is a plain Item: the two buttons need their own Row, or they overlap
                    Row {
                        spacing: 8
                        Button {
                            anchors.verticalCenter: parent.verticalCenter
                            text: I18n.t("Rescan")
                            icon: "refresh"
                            style: "text"
                            size: "xs"
                            visible: LocalFiles.folders.length > 0
                            enabled: !LocalFiles.scanning
                            onClicked: LocalFiles.rescan()
                        }
                        Button {
                            anchors.verticalCenter: parent.verticalCenter
                            text: I18n.t("Add folder")
                            icon: "add"
                            style: "outlined"
                            size: "xs"
                            onClicked: localFolderDialog.open()
                        }
                    }
                }

                FolderDialog {
                    id: localFolderDialog
                    title: I18n.t("Choose a music folder")
                    onAccepted: LocalFiles.addFolder(selectedFolder.toString())
                }
            }

            // ================= Хранилище =================
            SettingsCard {
                id: storageCard
                visible: page.shows("music")
                title: I18n.t("Storage")
                icon: "database"

                SettingRow {
                    title: I18n.t("Audio cache")
                    description: I18n.t("Played tracks are saved to disk and won’t be downloaded again")
                    Switch { checked: Settings.audioCache; onToggled: c => Settings.audioCache = c }
                }
                SettingRow {
                    title: I18n.t("Cache size")
                    enabled: Settings.audioCache
                    opacity: enabled ? 1 : 0.5
                    description: I18n.t("Older tracks are removed when the cache is full")
                    Slider {
                        wheelEnabled: false     // колесо прокручивает страницу, а не меняет настройку
                        width: 220
                        from: 256
                        to: 10240
                        stepSize: 256
                        value: Settings.audioCacheMb
                        valueText: v => v >= 1024 ? I18n.t("{0} GB", I18n.decimal(v / 1024, v % 1024 ? 1 : 0)) : I18n.t("{0} MB", Math.round(v))
                        onMoved: v => Settings.audioCacheMb = Math.round(v / 256) * 256
                    }
                }
                StorageBar {
                    Layout.fillWidth: true
                    usage: Settings.cacheUsage ?? ({})
                    limit: Settings.audioCache ? Settings.audioCacheMb * 1024 * 1024 : 0
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: 8
                    Button {
                        text: I18n.t("Clear cache")
                        icon: "delete_sweep"
                        style: "outlined"
                        onClicked: {
                            Settings.clearCache()
                            // занятые файлы (звук играющего трека) остаются — об этом не сообщаем, объём обновит мост
                            const w = page.Window.window
                            if (w && typeof w.notify === "function")
                                w.notify(I18n.t("Cache cleared"))
                        }
                    }
                    Button {
                        text: I18n.t("Open folder")
                        icon: "folder_open"
                        style: "text"
                        onClicked: Settings.openCacheFolder()
                    }
                }
                Label {
                    Layout.fillWidth: true
                    text: I18n.t("Clearing doesn’t sign you out or touch your settings.")
                    type: "bodySmall"
                    color: Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                }
            }
        }
    }

    Loader {
        id: appearanceLoader
        Layout.fillWidth: true
        active: page.shows("appearance")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ appearance: appearanceCard, window: appearanceCard })
            spacing: 24
            // ================= Внешний вид =================
            SettingsCard {
                id: appearanceCard
                visible: page.shows("appearance")
                title: I18n.t("Appearance")
                icon: "palette"

                SettingsBanner {
                    kind: "info"
                    icon: "restart_alt"
                    shown: page.scalePending
                    title: I18n.t("Restart to apply the scale")
                    text: I18n.t("The interface scale is read when the app starts. Apply now reopens the app at the new size.")
                    actionText: I18n.t("Apply now")
                    onAction: Settings.relaunch()
                }

                SettingRow {
                    title: I18n.t("Language")
                    // "system" (default) follows the OS UI language; the language names stay in their own language
                    ButtonGroup {
                        model: [I18n.t("System default"), "English", "Русский", "Português"]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, ["system", "en", "ru", "pt"].indexOf(Settings.language))
                        onActivated: i => Settings.language = ["system", "en", "ru", "pt"][i]
                    }
                }

                SettingRow {
                    title: I18n.t("Theme")
                    ButtonGroup {
                        model: [{ text: I18n.t("System"), icon: "contrast" }, { text: I18n.t("Light"), icon: "light_mode" }, { text: I18n.t("Dark"), icon: "dark_mode" }]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: ["system", "light", "dark"].indexOf(Settings.themeMode)
                        onActivated: i => Settings.themeMode = ["system", "light", "dark"][i]
                    }
                }

                SettingRow {
                    title: I18n.t("Interface scale")
                    description: I18n.t("Size of text and controls. Takes effect after restarting the app")
                    ButtonGroup {
                        model: ["75%", "100%", "125%", "150%", "200%"]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, [75, 100, 125, 150, 200].indexOf(Settings.uiScale))
                        onActivated: i => Settings.uiScale = [75, 100, 125, 150, 200][i]
                    }
                }

                SettingRow {
                    title: I18n.t("Color from cover")
                    description: I18n.t("The app recolors to match the playing track’s cover")
                    Switch { checked: Settings.accentFromCover; onToggled: c => Settings.accentFromCover = c }
                }

                SettingRow {
                    title: I18n.t("Accent color")
                    description: Settings.accentFromCover ? I18n.t("When nothing is playing") : I18n.t("The app’s main color")
                    Flow {
                        width: Math.min(8 * 40, Math.max(160, page.width - 2 * page.sideMargin - 48))
                        spacing: 8
                        Repeater {
                            // seeds of the Material 3 tonal palettes (baseline purple, blue, teal, green, amber, red, pink);
                            // the scheme is built from the seed, so each one gives a full harmonious set of roles
                            model: [String(Theme.defaultSeed).toUpperCase(), "#6750A4", "#0B57D0", "#006A6A", "#386A20",
                                    "#7D5700", "#9C4146", "#984061"]
                            MorphShape {
                                required property string modelData
                                readonly property bool current: Settings.customSeed.toUpperCase() === modelData
                                width: 32
                                height: 32
                                shape: current || swatchArea.containsMouse ? "cookie6" : "circle"
                                color: modelData
                                scale: swatchArea.pressed ? 0.9 : 1
                                Behavior on scale { NumberAnimation { duration: Theme.motion.effectsFast } }
                                MorphShape {
                                    anchors.centerIn: parent
                                    width: 12; height: 12
                                    visible: parent.current
                                    shape: "circle"
                                    color: "white"
                                }
                                MouseArea {
                                    id: swatchArea
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: Settings.customSeed = parent.modelData
                                }
                            }
                        }
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 12
                    Label { text: I18n.t("Color scheme"); type: "bodyLarge" }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: ThemeEngine.variants
                            Chip {
                                required property string modelData
                                text: page.variantNames[modelData] ?? modelData
                                selected: Settings.schemeVariant === modelData
                                onClicked: Settings.schemeVariant = modelData
                            }
                        }
                    }
                }

                // ---- Logo colorway ----
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 12
                    Label { text: I18n.t("Logo color"); type: "bodyLarge" }
                    Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: I18n.t("Choose the app logo’s colorway")
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 12
                        Repeater {
                            model: [
                                // the logo itself in each colour (assets/logo, tools/pixelmusic_logo.py)
                                { key: "bluePurple", file: "blue_purple" },
                                { key: "babyPinkPurple", file: "baby_pink_purple" },
                                { key: "graphiteGray", file: "graphite" },
                                { key: "limeGreen", file: "lime_green" },
                                { key: "limeLemon", file: "lime_lemon" },
                                { key: "orangeYellow", file: "orange_yellow" }
                            ]
                            Rectangle {
                                required property var modelData
                                width: 56
                                height: 56
                                radius: Theme.shape.full
                                color: Theme.surfaceContainerHigh
                                border.width: Settings.logoColorway === modelData.key ? 3 : 1
                                border.color: Settings.logoColorway === modelData.key ? Theme.primary : Theme.outlineVariant

                                Behavior on border.width { NumberAnimation { duration: Theme.motion.effectsFast } }

                                Image {
                                    anchors.centerIn: parent
                                    width: 40
                                    height: 40
                                    source: Qt.resolvedUrl("../../../assets/logo/" + modelData.file + ".svg")
                                    sourceSize: Qt.size(80, 80)
                                    smooth: true
                                }

                                Icon {
                                    anchors.bottom: parent.bottom
                                    anchors.right: parent.right
                                    anchors.margins: 2
                                    visible: Settings.logoColorway === modelData.key
                                    name: "check_circle"
                                    size: 18
                                    color: Theme.primary
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: Settings.logoColorway = modelData.key
                                }
                            }
                        }
                    }
                }

                // ---- Фон и обложка ----
                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: 8
                    spacing: 10
                    Label { text: I18n.t("Cover background"); type: "titleSmall"; color: Theme.primary }
                    WavyDivider {
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                        color: Theme.outlineVariant
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 12
                    Label { text: I18n.t("Cover background"); type: "bodyLarge" }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: page.backdropModes
                            Chip {
                                required property var modelData
                                text: modelData.text
                                selected: Settings.backdropMode === modelData.mode
                                onClicked: Settings.backdropMode = modelData.mode
                            }
                        }
                    }
                    Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: (page.backdropModes.find(m => m.mode === Settings.backdropMode) ?? page.backdropModes[0]).hint
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 12
                    Label { text: I18n.t("Color and background transition"); type: "bodyLarge" }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: page.transitions
                            Chip {
                                required property var modelData
                                text: modelData.text
                                selected: Settings.trackTransition === modelData.mode
                                onClicked: Settings.trackTransition = modelData.mode
                            }
                        }
                    }
                    Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: (page.transitions.find(m => m.mode === Settings.trackTransition) ?? page.transitions[0]).hint
                        type: "bodyMedium"
                        color: Theme.fgSurfaceVariant
                    }
                }

                SettingRow {
                    title: I18n.t("Player background blur")
                    enabled: Settings.backdropMode !== "palette"
                    opacity: enabled ? 1 : 0.5
                    description: Settings.backdropMode === "palette" ? I18n.t("The gradient doesn’t depend on the cover")
                        : I18n.t("{0}% · 0 is a sharp cover, 100 is color only", Math.round(Settings.nowPlayingBlur * 100))
                    Slider {
                        wheelEnabled: false     // колесо прокручивает страницу, а не меняет настройку
                        width: 220
                        from: 0
                        to: 1
                        stepSize: 0.05
                        value: Settings.nowPlayingBlur
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => Settings.nowPlayingBlur = Math.round(v * 20) / 20
                    }
                }

                SettingRow {
                    title: I18n.t("Blur drift")
                    description: Settings.nowPlayingDrift > 0
                        ? I18n.t("Background colors slowly shift while music plays")
                        : I18n.t("Off — the blur stays still")
                    Slider {
                        wheelEnabled: false     // колесо прокручивает страницу, а не меняет настройку
                        width: 220
                        from: 0
                        to: 1
                        stepSize: 0.05
                        value: Settings.nowPlayingDrift
                        valueText: v => v > 0 ? Math.round(v * 100) + " %" : I18n.t("off")
                        onMoved: v => Settings.nowPlayingDrift = Math.round(v * 20) / 20
                    }
                }

                SettingRow {
                    title: I18n.t("Ambient background")
                    description: I18n.t("The playing track’s cover, blurred behind the whole window as a soft colored glow")
                    Switch { checked: Settings.ambientBackground; onToggled: c => Settings.ambientBackground = c }
                }

                SettingRow {
                    title: I18n.t("Spinning cover frame")
                    description: I18n.t("While music plays, the shaped cover frame slowly rotates; the image itself stays still")
                    Switch { checked: Settings.coverSpin; onToggled: c => Settings.coverSpin = c }
                }

                SettingRow {
                    title: I18n.t("Cover pulses to the beat")
                    description: I18n.t("In the full-screen player, the cover gently pulses with the music’s loudness")
                    Switch { checked: Settings.coverPulse; onToggled: c => Settings.coverPulse = c }
                }

                // ---- Окно и плеер вне окна ----
                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: 8
                    spacing: 10
                    Label { text: I18n.t("Window"); type: "titleSmall"; color: Theme.primary }
                    WavyDivider {
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                        color: Theme.outlineVariant
                    }
                }

                // Плеер вне окна: островок — Windows, macOS и X11 (на Wayland окно не встанет у края экрана само),
                // панель задач — только Windows; где нет ни того ни другого, раздела нет
                FloatingPlayerPicker {
                    id: floatingPicker
                    modes: ["off"]
                        .concat(Qt.platform.os === "windows" || Qt.platform.os === "osx" || (Qt.platform.os === "linux" && (!Qt.platform.pluginName.startsWith("wayland") || WindowShell.kwinAvailable())) ? ["island"] : [])
                        .concat(Qt.platform.os === "windows" ? ["taskbar", "both"] : [])
                    visible: modes.length > 1
                }

                SettingRow {
                    visible: floatingPicker.visible && (Settings.floatingPlayer === "island" || Settings.floatingPlayer === "both") && page.screenCount > 1 && Qt.platform.os !== "osx"
                    title: Settings.floatingPlayer === "both" ? I18n.t("Island displays") : I18n.t("Displays")
                    description: I18n.t("Choose which monitors display the island player")
                    Flow {
                        width: Math.min(420, Math.max(160, page.width - 2 * page.sideMargin - 48))
                        spacing: 8
                        Chip {
                            text: I18n.t("All monitors")
                            selected: page.isAllScreensActive(Settings.floatingIslandScreens, page.screenCount)
                            onClicked: Settings.floatingIslandScreens = "all"
                        }
                        Repeater {
                            model: page.screenCount
                            delegate: Chip {
                                required property int modelData
                                text: I18n.t("Monitor") + " " + (modelData + 1)
                                selected: page.isScreenActive(Settings.floatingIslandScreens, modelData, page.screenCount)
                                onClicked: Settings.floatingIslandScreens = page.toggleScreenSetting(Settings.floatingIslandScreens, modelData, page.screenCount)
                            }
                        }
                    }
                }

                SettingRow {
                    visible: floatingPicker.visible && (Settings.floatingPlayer === "island" || Settings.floatingPlayer === "both")
                        && (Qt.platform.os === "windows" || (Qt.platform.os === "linux" && WindowShell.kwinAvailable()))
                    title: I18n.t("Hide over fullscreen apps")
                    description: I18n.t("The island steps aside while a game, video or browser is fullscreen on its monitor")
                    Switch { checked: Settings.floatingHideFullscreen; onToggled: c => Settings.floatingHideFullscreen = c }
                }

                SettingRow {
                    visible: floatingPicker.visible && (Settings.floatingPlayer === "island" || Settings.floatingPlayer === "both")
                        && (Qt.platform.os === "windows" || (Qt.platform.os === "linux" && WindowShell.kwinAvailable()))
                    title: I18n.t("Hide over maximized windows")
                    description: I18n.t("The island steps aside while a maximized window fills its monitor")
                    Switch { checked: Settings.floatingHideMaximized; onToggled: c => Settings.floatingHideMaximized = c }
                }

                SettingRow {
                    visible: floatingPicker.visible && (Settings.floatingPlayer === "island" || Settings.floatingPlayer === "both") && Qt.platform.os !== "osx"
                    title: I18n.t("Island edge")
                    description: Settings.floatingEdge === "free" ? I18n.t("Drag the island anywhere on the screen")
                        : I18n.t("The screen edge the island sticks to")
                    ButtonGroup {
                        readonly property var keys: ["top", "bottom", "left", "right", "free"]
                        model: [I18n.t("Top"), I18n.t("Bottom"), I18n.t("Left"), I18n.t("Right"), I18n.t("Free")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, keys.indexOf(Settings.floatingEdge))
                        onActivated: i => Settings.floatingEdge = keys[i]
                    }
                }

                SettingRow {
                    visible: floatingPicker.visible && (Settings.floatingPlayer === "island" || Settings.floatingPlayer === "both") && Qt.platform.os !== "osx"
                        && Settings.floatingEdge !== "free"
                    title: I18n.t("Island position")
                    description: I18n.t("Or drag the island along the edge. 0 is the center of the edge")
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: -1000
                        to: 1000
                        stepSize: 10
                        value: Settings.floatingOffsetX
                        valueText: v => Math.round(v) + " px"
                        onMoved: v => Settings.floatingOffsetX = Math.round(v / 10) * 10
                    }
                }

                SettingRow {
                    visible: floatingPicker.visible && (Settings.floatingPlayer === "taskbar" || Settings.floatingPlayer === "both") && page.screenCount > 1
                    title: Settings.floatingPlayer === "both" ? I18n.t("Taskbar displays") : I18n.t("Displays")
                    description: I18n.t("Choose which monitors display the taskbar player")
                    Flow {
                        width: Math.min(420, Math.max(160, page.width - 2 * page.sideMargin - 48))
                        spacing: 8
                        Chip {
                            text: I18n.t("All monitors")
                            selected: page.isAllScreensActive(Settings.taskbarScreens, page.screenCount)
                            onClicked: Settings.taskbarScreens = "all"
                        }
                        Repeater {
                            model: page.screenCount
                            delegate: Chip {
                                required property int modelData
                                text: I18n.t("Monitor") + " " + (modelData + 1)
                                selected: page.isScreenActive(Settings.taskbarScreens, modelData, page.screenCount)
                                onClicked: Settings.taskbarScreens = page.toggleScreenSetting(Settings.taskbarScreens, modelData, page.screenCount)
                            }
                        }
                    }
                }

                // Место виджета в панели задач (на каждом мониторе своё); перетаскивание виджета на панели ставит «своё»
                Repeater {
                    model: floatingPicker.visible && (Settings.floatingPlayer === "taskbar" || Settings.floatingPlayer === "both")
                           ? page.getActiveTaskbarScreenIndices(Settings.taskbarScreens, page.screenCount)
                           : []
                    delegate: SettingRow {
                        id: screenPosRow
                        required property int modelData
                        readonly property string pos: page.getScreenTaskbarPos(Settings.taskbarPositions, Settings.taskbarWidgetPos, modelData)
                        readonly property var keys: ["left", "appsLeft", "appsRight", "tray"]
                        title: page.getActiveTaskbarScreenIndices(Settings.taskbarScreens, page.screenCount).length > 1
                               ? I18n.t("Position on monitor {0}", modelData + 1)
                               : I18n.t("Position in the taskbar")
                        description: keys.indexOf(pos) >= 0
                                     ? I18n.t("Or hold the player in the taskbar and drag it; it snaps to these spots")
                                     : I18n.t("Your own spot. Hold the player in the taskbar and drag it to move it again")
                        ButtonGroup {
                            model: [I18n.t("Left"), I18n.t("Before apps"), I18n.t("After apps"), I18n.t("By the tray")]
                            autoSelect: false
                            size: page.compact ? "xs" : "s"
                            currentIndex: screenPosRow.keys.indexOf(screenPosRow.pos)
                            onActivated: i => page.setScreenTaskbarPos(screenPosRow.modelData, screenPosRow.keys[i])
                        }
                    }
                }

                SettingRow {
                    title: I18n.t("Compact sidebar")
                    description: I18n.t("Icons only. Turns on automatically in a narrow window")
                    Switch { checked: Settings.sidebarCollapsed; onToggled: c => Settings.sidebarCollapsed = c }
                }

                SettingRow {
                    title: I18n.t("Radio on Home")
                    description: I18n.t("The station scene at the top of Home. The Radio page and “Radio from this track” keep working")
                    Switch { checked: Settings.homeRadioHidden !== true; onToggled: c => Settings.homeRadioHidden = !c }
                }

                SettingRow {
                    title: I18n.t("Sidebar list")
                    description: Settings.sidebarLibrary === true
                                 ? I18n.t("What you added or played last: playlists, albums, artists, podcasts and Liked Songs")
                                 : I18n.t("Your playlist tree, folders included")
                    ButtonGroup {
                        model: [I18n.t("Playlists"), I18n.t("Library")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Settings.sidebarLibrary === true ? 1 : 0
                        onActivated: i => Settings.sidebarLibrary = i === 1
                    }
                }

                SettingRow {
                    title: I18n.t("Minimize to tray on close")
                    description: I18n.t("Closing the window hides it in the tray and the music keeps playing. Quit from the tray menu or with Ctrl+Q")
                    Switch { checked: Settings.closeToTray; onToggled: c => Settings.closeToTray = c }
                }

                SettingRow {
                    title: I18n.t("Mouse wheel scroll step")
                    description: I18n.t("{0} px per notch; grows up to ×2 when scrolling fast", Settings.wheelStep)
                    Slider {
                        wheelEnabled: false     // колесо прокручивает страницу, а не меняет настройку
                        width: 220
                        from: 60
                        to: 480
                        stepSize: 20
                        value: Settings.wheelStep
                        valueText: v => Math.round(v) + " px"
                        onMoved: v => Settings.wheelStep = Math.round(v)
                    }
                }

                // ---- Анимации и производительность ----
                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: 8
                    spacing: 10
                    Label { text: I18n.t("Animations"); type: "titleSmall"; color: Theme.primary }
                    WavyDivider {
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                        color: Theme.outlineVariant
                    }
                }

                SettingRow {
                    title: I18n.t("Animations")
                    description: Settings.animations === "reduced" ? I18n.t("No motion or morphing: only short fades")
                                 : Settings.animations === "full" ? I18n.t("All transitions, waves and morphs")
                                 : Settings.systemReducedMotion === true ? I18n.t("Follows Windows: animation effects are currently off")
                                 : I18n.t("Follows Windows “Animation effects”")
                    ButtonGroup {
                        model: [I18n.t("System default"), I18n.t("Full"), I18n.t("Minimal")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: ["system", "full", "reduced"].indexOf(Settings.animations)
                        onActivated: i => Settings.animations = ["system", "full", "reduced"][i]
                    }
                }

                SettingRow {
                    title: I18n.t("Weak computer mode")
                    description: Settings.lowPowerMode === "on" ? I18n.t("No background motion, shimmer, theme wipe or cover fades, minimal transitions")
                                 : Settings.lowPowerMode === "off" ? I18n.t("All effects, even when graphics are drawn by the processor")
                                 : Theme.lowPower ? I18n.t("On by itself: graphics are drawn by the processor, without the video card")
                                 : I18n.t("Turns on by itself when graphics are drawn by the processor")
                    ButtonGroup {
                        model: [I18n.t("Auto"), I18n.t("On"), I18n.t("Off")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: ["auto", "on", "off"].indexOf(Settings.lowPowerMode)
                        onActivated: i => Settings.lowPowerMode = ["auto", "on", "off"][i]
                    }
                }
                // Видеокарта (Linux, гибридная графика): по умолчанию — как решит система; «Дискретная» включает PRIME offload
                SettingRow {
                    visible: Qt.platform.os === "linux"
                    title: I18n.t("Graphics processor")
                    description: (Settings.gpuPreference === "integrated"
                                 ? I18n.t("Forces integrated graphics")
                                 : Settings.gpuPreference === "discrete"
                                 ? I18n.t("Forces dedicated graphics card")
                                 : I18n.t("The system picks the graphics card"))
                                 + " · " + I18n.t("Takes effect after restarting the app")
                    ButtonGroup {
                        model: [I18n.t("System"), I18n.t("Integrated"), I18n.t("Discrete")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, ["auto", "integrated", "discrete"].indexOf(Settings.gpuPreference))
                        onActivated: i => Settings.gpuPreference = ["auto", "integrated", "discrete"][i]
                    }
                }

                SettingRow {
                    title: I18n.t("Save power in background")
                    description: I18n.t("While the window is out of focus, decorative animations (progress wave, shapes, background shimmer) pause to reduce CPU load. Lyrics keep going")
                    Switch { checked: Settings.calmWhenInactive; onToggled: c => Settings.calmWhenInactive = c }
                }
            }
        }
    }

    Loader {
        id: lyricsLoader
        Layout.fillWidth: true
        active: page.shows("lyrics")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ lyrics: lyricsCard, reading: readingCard })
            spacing: 24
            // ================= Текст песни =================
            SettingsCard {
                id: lyricsCard
                visible: page.shows("lyrics")
                title: I18n.t("Lyrics")
                icon: "lyrics"

                // Живое превью: заливка по слову, свечение и размытие как в LyricsView.
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: preview.implicitHeight + 28
                    radius: Theme.shape.large
                    // The mounted view paints the scrim itself; tinting the card too would double it.
                    color: "transparent"
                    border.width: 1
                    border.color: Theme.outlineVariant

                    LyricsStylePreview {
                        id: preview
                        anchors.centerIn: parent
                        width: parent.width - 32
                        baseFontSize: 20
                    }
                }

                SettingRow {
                    title: I18n.t("Style")
                    description: Settings.lyricsMode === "spicy" ? I18n.t("Spicy style: the singing line is larger, with a soft gradient sweep and a glow")
                                 : Settings.lyricsMode === "karaoke" ? I18n.t("Karaoke: larger singing line with sharp word-by-word fill")
                                 : I18n.t("As it was: cascade scroll, blurred neighbouring lines")
                    ButtonGroup {
                        model: [I18n.t("Classic"), I18n.t("Spicy"), I18n.t("Karaoke")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, ["normal", "spicy", "karaoke"].indexOf(Settings.lyricsMode))
                        onActivated: i => Settings.lyricsMode = ["normal", "spicy", "karaoke"][i]
                    }
                }

                SettingRow {
                    id: activeScaleRow
                    // Настройка 0 = по стилю: отображается значение текущего стиля
                    readonly property real styleScale: Settings.lyricsMode === "spicy" ? 1.12 : (Settings.lyricsMode === "karaoke" ? 1.16 : 1.0)
                    readonly property real activeScaleVal: Settings.lyricsActiveScale > 0 ? Settings.lyricsActiveScale : styleScale
                    title: I18n.t("Active line scale")
                    description: Math.round(activeScaleVal * 100) <= 100
                                 ? I18n.t("Standard size: the singing line is not enlarged")
                                 : I18n.t("{0}%: enlarges the currently singing line", Math.round(activeScaleVal * 100))
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 1.0
                        to: 1.35
                        stepSize: 0.05
                        value: activeScaleRow.activeScaleVal
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => { if (Settings.lyricsActiveScale !== undefined) Settings.lyricsActiveScale = Math.round(v * 20) / 20 }
                    }
                }

                SettingRow {
                    title: I18n.t("Text alignment")
                    description: Settings.lyricsTextAlign === "center" ? I18n.t("Centered text")
                                 : Settings.lyricsTextAlign === "right" ? I18n.t("Right aligned")
                                 : I18n.t("Left aligned")
                    ButtonGroup {
                        model: [I18n.t("Left"), I18n.t("Center"), I18n.t("Right")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, ["left", "center", "right"].indexOf(Settings.lyricsTextAlign))
                        onActivated: i => Settings.lyricsTextAlign = ["left", "center", "right"][i]
                    }
                }

                SettingRow {
                    title: I18n.t("Fill mode")
                    description: Settings.lyricsFillMode === "line" ? I18n.t("Whole line: the active line lights up all at once")
                                 : Settings.lyricsFillMode === "off" ? I18n.t("Off: lines stay fully lit without progress fill")
                                 : I18n.t("Word by word: words fill as they are sung")
                    ButtonGroup {
                        model: [I18n.t("Word by word"), I18n.t("Whole line"), I18n.t("Off")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, ["word", "line", "off"].indexOf(Settings.lyricsFillMode))
                        onActivated: i => Settings.lyricsFillMode = ["word", "line", "off"][i]
                    }
                }

                SettingRow {
                    title: I18n.t("Highlight intensity")
                    enabled: Settings.lyricsFillMode !== "off"
                    opacity: enabled ? 1 : 0.5
                    description: Settings.lyricsHighlight <= 0.01 ? I18n.t("Sharp cut with no gradient or glow")
                                 : I18n.t("{0}%: scales the gradient width and glow", Math.round(Settings.lyricsHighlight * 100))
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 0
                        to: 2.0
                        stepSize: 0.05
                        value: Settings.lyricsHighlight
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => Settings.lyricsHighlight = Math.round(v * 20) / 20
                    }
                }

                SettingRow {
                    id: bounceRow
                    readonly property real bounceVal: Settings.lyricsWordBounce ?? 1.0
                    title: I18n.t("Word wave motion")
                    description: bounceVal <= 0.01 ? I18n.t("Off: words stay still while sung")
                                 : I18n.t("{0}%: vertical lift and pulse of sung words", Math.round(bounceVal * 100))
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 0
                        to: 2.0
                        stepSize: 0.1
                        value: bounceRow.bounceVal
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => { if (Settings.lyricsWordBounce !== undefined) Settings.lyricsWordBounce = Math.round(v * 10) / 10 }
                    }
                }

                SettingRow {
                    title: I18n.t("Background scrim")
                    description: Settings.lyricsScrim === "soft" ? I18n.t("Soft tinted backing behind the lyrics")
                                 : Settings.lyricsScrim === "strong" ? I18n.t("Prominent tinted backing behind the lyrics")
                                 : I18n.t("Transparent: lyrics sit directly over the backdrop")
                    ButtonGroup {
                        model: [I18n.t("None"), I18n.t("Soft"), I18n.t("Strong")]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: Math.max(0, ["none", "soft", "strong"].indexOf(Settings.lyricsScrim))
                        onActivated: i => Settings.lyricsScrim = ["none", "soft", "strong"][i]
                    }
                }

            }

            // ================= Чтение =================
            SettingsCard {
                id: readingCard
                visible: page.shows("lyrics")
                title: I18n.t("Reading")
                icon: "visibility"

                SettingRow {
                    title: I18n.t("Lyrics size")
                    description: I18n.t("{0}% of the default size", Math.round(Settings.lyricsFontScale * 100))
                    Slider {
                        wheelEnabled: false     // колесо прокручивает страницу, а не меняет настройку
                        width: 220
                        from: 0.7
                        to: 1.6
                        stepSize: 0.05
                        value: Settings.lyricsFontScale
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => Settings.lyricsFontScale = Math.round(v * 20) / 20
                    }
                }

                SettingRow {
                    id: linesRow
                    readonly property int linesVal: {
                        const s = String(Settings.lyricsVisibleLines ?? "all").trim()
                        if (s === "all" || s === "") return 15
                        if (s === "one") return 1
                        if (s === "three") return 3
                        const n = parseInt(s, 10)
                        return (!isNaN(n) && n >= 1 && n <= 15) ? n : 15
                    }
                    title: I18n.t("Visible lines")
                    description: linesVal >= 15 ? I18n.t("All lines: the full song text is visible")
                                 : linesVal === 1 ? I18n.t("One line: only the active line is visible")
                                 : I18n.t("{0} lines around the singing line", linesVal)
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 1
                        to: 15
                        stepSize: 2
                        value: linesRow.linesVal
                        valueText: v => Math.round(v) >= 15 ? I18n.t("All lines") : String(Math.round(v))
                        onMoved: v => {
                            const n = Math.round(v)
                            Settings.lyricsVisibleLines = n >= 15 ? "all" : (n === 1 ? "one" : (n === 3 ? "three" : String(n)))
                        }
                    }
                }

                SettingRow {
                    id: idleScaleRow
                    // Настройка 0 = по стилю: отображается значение текущего стиля
                    readonly property real styleScale: Settings.lyricsMode === "spicy" ? 0.88 : (Settings.lyricsMode === "karaoke" ? 0.90 : 0.94)
                    readonly property real effective: Settings.lyricsIdleScale > 0 ? Settings.lyricsIdleScale : styleScale
                    title: I18n.t("Inactive lines size")
                    description: I18n.t("{0}%: how much the lines around the singing one stay smaller", Math.round(effective * 100))
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 0.8
                        to: 1.0
                        stepSize: 0.01
                        value: idleScaleRow.effective
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => { if (Settings.lyricsIdleScale !== undefined) Settings.lyricsIdleScale = Math.round(v * 100) / 100 }
                    }
                }

                SettingRow {
                    id: scrollMsRow
                    // -1 = стиль решает сам (spicy 420, karaoke 440, обычный 600)
                    readonly property int styleMs: Settings.lyricsMode === "spicy" ? 420 : (Settings.lyricsMode === "karaoke" ? 440 : 600)
                    readonly property int effectiveMs: Settings.lyricsScrollMs >= 0 ? Settings.lyricsScrollMs : styleMs
                    title: I18n.t("Scroll speed")
                    description: I18n.t("{0} ms: how long the text takes to move to the next line", effectiveMs)
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 200
                        to: 800
                        stepSize: 20
                        value: scrollMsRow.effectiveMs
                        valueText: v => Math.round(v) + " ms"
                        onMoved: v => { if (Settings.lyricsScrollMs !== undefined) Settings.lyricsScrollMs = Math.round(v / 20) * 20 }
                    }
                }

                SettingRow {
                    id: staggerRow
                    // -1 = стиль решает сам (spicy 28, karaoke 26, обычный 42); 0 = весь блок вместе
                    readonly property int styleMs: Settings.lyricsMode === "spicy" ? 28 : (Settings.lyricsMode === "karaoke" ? 26 : 42)
                    readonly property int effectiveMs: Settings.lyricsStaggerMs >= 0 ? Settings.lyricsStaggerMs : styleMs
                    title: I18n.t("Cascade delay")
                    description: I18n.t("{0} ms between neighbouring lines; 0 moves the whole block together", effectiveMs)
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 0
                        to: 80
                        stepSize: 4
                        value: staggerRow.effectiveMs
                        valueText: v => Math.round(v) + " ms"
                        onMoved: v => { if (Settings.lyricsStaggerMs !== undefined) Settings.lyricsStaggerMs = Math.round(v / 4) * 4 }
                    }
                }

                SettingRow {
                    id: dimRow
                    readonly property real dimVal: Settings.lyricsDimOpacity ?? 0.35
                    title: I18n.t("Inactive lines opacity")
                    description: I18n.t("{0}% brightness on unsung and neighbouring lines", Math.round(dimVal * 100))
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 0.15
                        to: 0.75
                        stepSize: 0.05
                        value: dimRow.dimVal
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => { if (Settings.lyricsDimOpacity !== undefined) Settings.lyricsDimOpacity = Math.round(v * 20) / 20 }
                    }
                }

                SettingRow {
                    title: I18n.t("Blur other lines")
                    description: Settings.lyricsBlurAmount <= 0.01 ? I18n.t("No blur on inactive lines")
                                 : I18n.t("{0}% blur on inactive lines", Math.round(Settings.lyricsBlurAmount * 100))
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: 0
                        to: 1.0
                        stepSize: 0.05
                        value: Settings.lyricsBlurAmount
                        valueText: v => v > 0.01 ? Math.round(v * 100) + " %" : I18n.t("off")
                        onMoved: v => Settings.lyricsBlurAmount = Math.round(v * 20) / 20
                    }
                }

                SettingRow {
                    title: I18n.t("Line spacing")
                    description: I18n.t("{0}% of the style spacing", Math.round(Settings.lyricsLineSpacing * 100))
                    Slider {
                        wheelEnabled: false     // колесо прокручивает страницу, а не меняет настройку
                        width: 220
                        from: 0.8
                        to: 1.6
                        stepSize: 0.05
                        value: Settings.lyricsLineSpacing
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => Settings.lyricsLineSpacing = Math.round(v * 20) / 20
                    }
                }

                SettingRow {
                    title: I18n.t("Animations")
                    description: I18n.t("Smooth scrolling, cascade transitions and line shifts. Turn off for instant updates")
                    Switch { checked: Settings.lyricsAnimate; onToggled: c => Settings.lyricsAnimate = c }
                }

                SettingRow {
                    title: I18n.t("Lyrics source label")
                    description: I18n.t("Shows the source badge (e.g. “LRCLIB · line by line”)")
                    Switch { checked: Settings.lyricsShowSource; onToggled: c => Settings.lyricsShowSource = c }
                }

                SettingRow {
                    title: I18n.t("Show romanization")
                    description: I18n.t("A line in Latin letters under Hindi, Punjabi, Korean, Japanese kana, Cyrillic and Greek lyrics")
                    Switch { checked: Settings.lyricsRomanize; onToggled: c => Settings.lyricsRomanize = c }
                }

                SettingRow {
                    title: I18n.t("Sync adjustment")
                    description: Settings.lyricsSyncMs === 0 ? I18n.t("Exact sync with playback (0 ms)")
                                 : (Settings.lyricsSyncMs > 0 ? I18n.t("Lyrics lead playback by {0} ms", Settings.lyricsSyncMs)
                                 : I18n.t("Lyrics lag playback by {0} ms", Math.abs(Settings.lyricsSyncMs)))
                    Slider {
                        wheelEnabled: false
                        width: 220
                        from: -3000
                        to: 3000
                        stepSize: 50
                        value: Settings.lyricsSyncMs
                        valueText: v => (v > 0 ? "+" : "") + Math.round(v) + " ms"
                        onMoved: v => Settings.lyricsSyncMs = Math.round(v / 50) * 50
                    }
                }

            }
        }
    }

    Loader {
        id: playbackLoader
        Layout.fillWidth: true
        active: page.shows("playback")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ playback: playbackCard, sleep: sleepTimerCard })
            spacing: 24
            // ================= Воспроизведение =================
            SettingsCard {
                id: playbackCard
                visible: page.shows("playback")
                title: I18n.t("Playback")
                icon: "graphic_eq"

                SettingRow {
                    title: I18n.t("Volume normalization")
                    description: I18n.t("Tracks play at the same loudness")
                    Switch { checked: Settings.normalisation; onToggled: c => Settings.normalisation = c }
                }
                SettingRow {
                    title: I18n.t("Autoplay")
                    description: I18n.t("When the queue runs out, similar songs keep playing")
                    Switch { checked: Settings.autoplay; onToggled: c => Settings.autoplay = c }
                }
                SettingRow {
                    title: I18n.t("YouTube Music data saver")
                    description: I18n.t("YouTube Music songs stream at a lower bitrate (about 64 kbps). Applies from the next song")
                    Switch { checked: Settings.youtubeDataSaver; onToggled: c => Settings.youtubeDataSaver = c }
                }
                SettingRow {
                    title: I18n.t("Lossless streams")
                    description: I18n.t("YouTube Music songs play in FLAC from the free Monochrome server when it has the same recording. Not with the data saver")
                    Switch { checked: Settings.losslessStreams; onToggled: c => Settings.losslessStreams = c }
                }
                SettingRow {
                    visible: Settings.losslessStreams
                    title: I18n.t("Monochrome server")
                    description: Settings.monochromeCheck === "ok" ? I18n.t("Answering. Lossless streams are available")
                        : Settings.monochromeCheck === "down" ? I18n.t("Not answering. Songs play from YouTube Music for now")
                        : Settings.monochromeCheck === "checking" ? I18n.t("Checking…")
                        : I18n.t("Check whether the server answers")
                    Button {
                        text: I18n.t("Check")
                        icon: "refresh"
                        style: "outlined"
                        size: "xs"
                        enabled: Settings.monochromeCheck !== "checking"
                        onClicked: Settings.checkMonochrome()
                    }
                }
                SettingRow {
                    title: I18n.t("JioSaavn streams")
                    description: I18n.t("YouTube Music songs play from JioSaavn (320 kbps) when it has them, and JioSaavn songs appear in search")
                    Switch { checked: Settings.jioSaavnStreams; onToggled: c => Settings.jioSaavnStreams = c }
                }
                SettingRow {
                    title: I18n.t("Output device")
                    description: I18n.t("Where this computer’s audio goes")
                    MenuField {
                        width: page.compact ? 220 : 280
                        leadingIcon: "speaker"
                        model: [{ id: "", text: I18n.t("System default") }].concat((Settings.audioDevices ?? []).map(d => ({ id: d, text: d })))
                        currentValue: Settings.audioDevice
                        onActivated: v => Settings.audioDevice = v
                    }
                }
                SettingRow {
                    title: I18n.t("Volume")
                    enabled: !Player.remote
                    opacity: enabled ? 1 : 0.5
                    description: Player.remote ? I18n.t("Another device is playing — its volume is in the mini player") : ""
                    Slider {
                        wheelEnabled: false     // колесо прокручивает страницу, а не меняет настройку
                        width: 220
                        from: 0
                        to: 1
                        stepSize: 0.01
                        value: Settings.volume
                        valueText: v => Math.round(v * 100) + " %"
                        onMoved: v => Player.volume = v
                    }
                }
            }

            SleepTimerCard {
                id: sleepTimerCard
                visible: page.shows("playback")
                compact: page.compact
            }
        }
    }

    Loader {
        id: equalizerLoader
        Layout.fillWidth: true
        active: page.shows("equalizer")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ equalizer: equalizerCard })
            spacing: 24
            // ================= Equalizer and sleep timer (app/EqualizerCard.qml, app/SleepTimerCard.qml) =================
            EqualizerCard {
                id: equalizerCard
                visible: page.shows("equalizer")
                compact: page.compact
            }
        }
    }

    Loader {
        id: networkLoader
        Layout.fillWidth: true
        active: page.shows("network")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ network: networkCard })
            spacing: 24
            // ================= Сеть =================
            SettingsCard {
                id: networkCard
                visible: page.shows("network")
                title: I18n.t("Network")
                icon: "lan"

                // черновик формы: начальные значения — применённые настройки
                QtObject {
                    id: proxy
                    property string mode: Settings.proxyMode || "off"
                    property string host: Settings.proxyHost
                    property string port: Settings.proxyPort
                    property string username: Settings.proxyUsername
                    property string password: ""
                    readonly property bool manual: mode === "http" || mode === "socks"
                    readonly property bool dirty: mode !== (Settings.proxyMode || "off")
                        || (manual && (host !== Settings.proxyHost || port !== Settings.proxyPort
                                       || username !== Settings.proxyUsername || password !== ""))
                    readonly property bool valid: !manual || (host.trim() !== "" && /^\d{1,5}$/.test(port.trim()) && Number(port) <= 65535)
                    function reset() {
                        mode = Settings.proxyMode || "off"
                        host = Settings.proxyHost
                        port = Settings.proxyPort
                        username = Settings.proxyUsername
                        password = ""
                        hostField.text = host
                        portField.text = port
                        userField.text = username
                        passField.text = ""
                    }
                    function apply() {
                        Settings.applyProxy(mode, host.trim(), port.trim(), username.trim(), password)
                    }
                }
                Connections {
                    target: Settings
                    function onProxyStateChanged() {
                        if (Settings.proxyState === "ok")
                            proxy.reset()
                    }
                }

                readonly property var modes: ["off", "system", "http", "socks"]
                SettingRow {
                    title: I18n.t("Proxy")
                    description: proxy.mode === "off" ? I18n.t("Direct, no proxy")
                                 : proxy.mode === "system" ? I18n.t("Same as Windows settings")
                                 : proxy.mode === "http" ? I18n.t("HTTP proxy (CONNECT)") : I18n.t("SOCKS5 proxy")
                    ButtonGroup {
                        model: [I18n.t("Off"), I18n.t("System proxy"), "HTTP", "SOCKS5"]
                        autoSelect: false
                        size: page.compact ? "xs" : "s"
                        currentIndex: networkCard.modes.indexOf(proxy.mode)
                        onActivated: i => proxy.mode = networkCard.modes[i]
                    }
                }

                Item {
                    Layout.fillWidth: true
                    implicitHeight: proxy.manual ? fields.implicitHeight : 0
                    Behavior on implicitHeight { NumberAnimation { duration: Theme.motion.spatialDefault; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.motion.emphasized } }
                    opacity: proxy.manual ? 1 : 0
                    Behavior on opacity { NumberAnimation { duration: Theme.motion.effectsDefault } }
                    visible: implicitHeight > 0.5
                    clip: true

                    GridLayout {
                        id: fields
                        width: parent.width
                        columns: page.compact ? 1 : 2
                        columnSpacing: 16
                        rowSpacing: 12
                        TextField {
                            id: hostField
                            Layout.fillWidth: true
                            label: I18n.t("Host")
                            placeholderText: "proxy.example.com"
                            text: Settings.proxyHost
                            onEdited: proxy.host = text
                        }
                        TextField {
                            id: portField
                            Layout.fillWidth: true
                            label: I18n.t("Port")
                            placeholderText: proxy.mode === "socks" ? "1080" : "8080"
                            text: Settings.proxyPort
                            inputMethodHints: Qt.ImhDigitsOnly
                            maximumLength: 5
                            error: text !== "" && !/^\d{1,5}$/.test(text)
                            onEdited: proxy.port = text
                        }
                        TextField {
                            id: userField
                            Layout.fillWidth: true
                            label: I18n.t("Username")
                            placeholderText: I18n.t("optional")
                            text: Settings.proxyUsername
                            onEdited: proxy.username = text
                        }
                        TextField {
                            id: passField
                            Layout.fillWidth: true
                            label: I18n.t("Password")
                            placeholderText: Settings.proxyUsername !== "" ? I18n.t("empty — unchanged") : I18n.t("optional")
                            echoMode: TextInput.Password
                            onEdited: proxy.password = text
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 12
                    visible: proxy.dirty || Settings.proxyState !== ""
                    StatusChip {
                        visible: Settings.proxyState === "ok" && !proxy.dirty
                        kind: "ok"
                        text: I18n.t("Applied")
                    }
                    LoadingIndicator { visible: Settings.proxyState === "applying"; size: 28 }
                    Label {
                        Layout.fillWidth: true
                        visible: text !== ""
                        text: Settings.proxyState === "applying" ? I18n.t("Checking the connection through the proxy…")
                              : Settings.proxyState === "error" ? (Settings.proxyError || I18n.t("Proxy isn’t responding"))
                              : ""
                        type: "bodyMedium"
                        color: Settings.proxyState === "error" ? Theme.error : Theme.fgSurfaceVariant
                        wrapMode: Text.WordWrap
                        elide: Text.ElideNone
                    }
                    Item { Layout.fillWidth: true; visible: Settings.proxyState !== "applying" && Settings.proxyState !== "error" }
                    Button {
                        visible: proxy.dirty
                        text: I18n.t("Revert")
                        style: "text"
                        onClicked: proxy.reset()
                    }
                    Button {
                        visible: proxy.dirty
                        enabled: proxy.valid && Settings.proxyState !== "applying"
                        text: I18n.t("Apply")
                        icon: "check"
                        onClicked: proxy.apply()
                    }
                }

                // обложки по умолчанию идут через прокси, как всё остальное; CDN картинок часто открыт и напрямую
                SettingRow {
                    visible: (Settings.proxyMode || "off") !== "off"
                    title: I18n.t("Covers bypass the proxy")
                    description: I18n.t("Load covers and pictures directly: faster on a slow proxy and saves its traffic. the image servers see your real address")
                    Switch { checked: Settings.coversBypassProxy; onToggled: c => Settings.coversBypassProxy = c }
                }

                Label {
                    Layout.fillWidth: true
                    text: I18n.t("The proxy password is stored in Credential Manager. Local network device discovery bypasses the proxy.")
                    type: "bodySmall"
                    color: Theme.fgSurfaceVariant
                    wrapMode: Text.WordWrap
                    elide: Text.ElideNone
                }
            }
        }
    }

    Loader {
        id: statsLoader
        Layout.fillWidth: true
        active: page.shows("stats")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ stats: statsCard })
            spacing: 24
            // ================= Listening stats and backup (app/ListeningStatsCard.qml, app/BackupCard.qml) =================
            ListeningStatsCard {
                id: statsCard
                visible: page.shows("stats")
                compact: page.compact
            }
        }
    }

    Loader {
        id: backupLoader
        Layout.fillWidth: true
        active: page.shows("backup")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ backup: backupCard })
            spacing: 24
            BackupCard {
                id: backupCard
                visible: page.shows("backup")
            }
        }
    }

    Loader {
        id: aboutLoader
        Layout.fillWidth: true
        active: page.shows("about")
        // built after the page is on screen: the router shows the category at once, its cards stream in
        asynchronous: true
        visible: status === Loader.Ready
        onLoaded: page._cardsLoaded()
        sourceComponent: ColumnLayout {
            readonly property var cards: ({ updates: updatesCard })
            spacing: 24
            // ================= О программе: шапка с версией, обновления, проект (как About в PixelPlayer) =================
            AboutHeader {
                visible: page.shows("about")
            }

            // ================= Обновления (app/UpdatesCard.qml) =================
            UpdatesCard {
                id: updatesCard
                visible: page.shows("about")
                compact: page.compact
            }

            SettingsCard {
                id: aboutCard
                // the project the app's updates and crash reports come from (sf-services update::API, CrashReport.issueUrl)
                readonly property string repoUrl: "https://github.com/Rajat-Solankii/PixelMusic"
                visible: page.shows("about")
                title: I18n.t("Project")
                icon: "info"
                grouped: true

                SettingsCategoryRow {
                    first: true
                    title: I18n.t("Source code")
                    subtitle: aboutCard.repoUrl.replace("https://", "")
                    icon: "code"
                    tone: "primary"
                    trailingIcon: "open_in_new"
                    onClicked: Qt.openUrlExternally(aboutCard.repoUrl)
                }
                SettingsCategoryRow {
                    title: I18n.t("Release notes")
                    subtitle: I18n.t("What changed in each version")
                    icon: "new_releases"
                    tone: "tertiary"
                    trailingIcon: "open_in_new"
                    onClicked: Updater.openReleasePage()
                }
                SettingsCategoryRow {
                    title: I18n.t("Report a problem")
                    subtitle: I18n.t("Open an issue on GitHub")
                    icon: "flag"
                    tone: "error"
                    trailingIcon: "open_in_new"
                    onClicked: Qt.openUrlExternally(aboutCard.repoUrl + "/issues/new")
                }
                SettingsCategoryRow {
                    last: true
                    title: I18n.t("Licences and credits")
                    subtitle: I18n.t("Free software under GPL-3.0-or-later. Based on spotifast (crmne, MIT) and yamusic-md3 (yaskorinov, MIT); not affiliated with Google or YouTube. Fonts: Google Sans Flex and Google Sans (OFL), Material Symbols (Apache 2.0).")
                    icon: "copyright"
                    tone: "secondary"
                    trailingIcon: ""
                    clickable: false
                }
            }
        }
    }
}
