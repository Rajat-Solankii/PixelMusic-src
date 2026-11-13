import QtQuick
import QtQuick.Layouts
import Md3
import PixelMusic.Core
import ".."

TrackListPage {
    id: page

    property string albumId
    property string trackId                   // «перейти к треку»: прокрутить к нему и подсветить
    highlightTrackId: trackId
    readonly property QtObject album: Catalog.album(albumId)
    readonly property var info: album ? album.info : ({})
    readonly property bool saved: Library.savedRevision >= 0 && (info.uri ?? "") !== "" ? Library.isSaved(info.uri) : info.saved === true

    title: info.title ? info.title + (info.version ? " (" + info.version + ")" : "") : ""
    overline: (info.kind ? I18n.any(info.kind) : I18n.t("Album")).toUpperCase()
    heroImage: info.coverLarge || info.cover || ""
    heroIcon: "album"
    heroArtistRefs: info.artistRefs ?? []
    subtitle: {
        if (!info.title)
            return ""
        const parts = []
        if (info.year)
            parts.push(info.year)
        const n = album.tracks.count
        parts.push(I18n.n(n, "{0} track", "{0} tracks"))
        // лейбл — если в шапке есть место (полностью он и в сведениях под списком)
        if (info.label && page.width > 640)
            parts.push(info.label)
        return parts.join("  ·  ")
    }
    model: album ? album.tracks : null
    showPlays: true
    showAlbum: false
    // сведения о выпуске под списком: дата, треки и длительность, лейбл, правообладатели
    footerExtra: album && album.tracks.count > 0 ? releaseInfo : null
    emptyIcon: "album"
    emptyTitle: album && album.errorText !== "" ? I18n.t("Album didn't load") : I18n.t("This album has no tracks")
    emptyText: album ? album.errorText : ""

    function notify(text) {
        const w = page.Window.window
        if (w && typeof w.notify === "function")
            w.notify(text, "", null)
    }

    // сохранить альбом в библиотеку
    headerExtra: Component {
        Row {
            spacing: 8
            IconButton {
                icon: page.saved ? "library_add_check" : "library_add"
                style: page.saved ? "tonal" : "standard"
                size: "m"
                checkable: true
                checked: page.saved
                autoToggle: false
                enabled: (page.info.uri ?? "") !== ""
                Accessible.name: page.saved ? I18n.t("In library") : I18n.t("Save")
                onClicked: Library.toggleSaved(page.info.uri)
            }
            // YouTube Music albums: save every song for playing offline (Downloads)
            IconButton {
                id: albumDownload
                readonly property var rows: {
                    const model = page.album ? page.album.tracks : null
                    const out = []
                    if (!model || Downloads.revision < 0)
                        return out
                    for (let i = 0; i < model.count; ++i) {
                        const row = model.get(i)
                        if ((row.uri ?? "") !== "" && Downloads.canDownload(row.uri))
                            out.push(row)
                    }
                    return out
                }
                readonly property bool downloaded: Downloads.revision >= 0 && rows.length > 0
                    && rows.every(row => Downloads.isDownloaded(row.uri))
                visible: (page.info.uri ?? "").startsWith("youtube:")
                enabled: rows.length > 0
                icon: downloaded ? "download_done" : "download"
                style: downloaded ? "tonal" : "standard"
                size: "m"
                checkable: true
                checked: downloaded
                autoToggle: false
                Accessible.name: downloaded ? I18n.t("Remove download") : I18n.t("Download")
                onClicked: {
                    for (const row of rows) {
                        if (downloaded)
                            Downloads.remove(row.uri)
                        else if (!Downloads.isDownloaded(row.uri) && !Downloads.isPending(row.uri))
                            Downloads.download(row)
                    }
                    page.notify(downloaded ? I18n.t("Removed the download of “{0}”", page.info.title ?? "")
                                           : I18n.t("Downloading “{0}”", page.info.title ?? ""))
                }
            }
        }
    }

    Component {
        id: releaseInfo
        ColumnLayout {
            readonly property var copyrights: page.info.copyrights ?? []
            readonly property var tracks: page.album ? page.album.tracks : null
            spacing: 4
            // отступ сверху и слева — как у текста строк и шапки
            Item { implicitHeight: 20 }
            Rectangle {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                Layout.bottomMargin: 16
                implicitHeight: 1
                color: Theme.outlineVariant
            }
            Label {
                visible: text !== ""
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                text: I18n.releaseDate(page.info.releaseDate ?? "")
                type: "titleSmall"
                weight: 600
            }
            Label {
                visible: parent.tracks !== null && parent.tracks.count > 0
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                text: parent.tracks ? I18n.t("{0}, {1}", I18n.n(parent.tracks.count, "{0} track", "{0} tracks"),
                                             I18n.duration(parent.tracks.totalDurationMs)) : ""
                type: "bodyMedium"
                color: Theme.fgSurfaceVariant
            }
            Label {
                visible: (page.info.label ?? "") !== ""
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                Layout.topMargin: 8
                text: I18n.t("Label: {0}", page.info.label ?? "")
                type: "bodySmall"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.Wrap
            }
            Label {
                visible: (page.info.courtesyLine ?? "") !== ""
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                text: page.info.courtesyLine ?? ""
                type: "bodySmall"
                color: Theme.fgSurfaceVariant
                wrapMode: Text.Wrap
            }
            Repeater {
                model: parent.copyrights
                Label {
                    required property var modelData
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    text: modelData
                    type: "bodySmall"
                    color: Theme.fgSurfaceVariant
                    wrapMode: Text.Wrap
                }
            }
            Item { implicitHeight: 12 }
        }
    }
}
