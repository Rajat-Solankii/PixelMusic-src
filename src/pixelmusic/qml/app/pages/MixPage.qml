import QtQuick
import Md3
import PixelMusic.Core
import ".."

// "See all" of a Home mix (Home.dailyMixTracks / recentTracks / yourMixTracks — built from the local
// listening history, home/mixes.rs). mix: "daily" | "recent" | "your".
TrackListPage {
    id: page

    property string mix: "daily"

    title: mix === "recent" ? I18n.t("Recently played") : mix === "your" ? I18n.t("Your Mix") : I18n.t("Daily Mix")
    overline: mix === "recent" ? I18n.t("HISTORY") : I18n.t("MIX")
    heroIcon: mix === "recent" ? "history" : "auto_awesome"
    heroShape: mix === "recent" ? "cookie9" : "flower8"
    heroImage: model && model.count > 0 ? (model.get(0).coverLarge || model.get(0).cover || "") : ""
    subtitle: !model || model.count === 0 ? ""
            : mix === "recent" ? I18n.n(model.count, "{0} track", "{0} tracks")
            : mix === "your" ? I18n.t("Today's mix for you") : I18n.t("Based on your history")
    showAlbum: false
    model: mix === "recent" ? Home.recentTracks : mix === "your" ? Home.yourMixTracks : Home.dailyMixTracks
    emptyIcon: "history"
    emptyTitle: I18n.t("Nothing here yet")
    emptyText: I18n.t("Play a few songs and your mixes will appear here")
}
