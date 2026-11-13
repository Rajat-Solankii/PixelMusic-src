// Регистрация типов QML-модуля PixelMusic.Core.
// Владелец — «bridges-player». Синглтоны создаются здесь, ДО движка, и живут до конца процесса.
// Имена типов — контракт с QML.
#include "register.h"

#include <QtQml/QQmlEngine>
#include <QtQml/qqml.h>

#include "pixelmusic/src/bridges/auth.cxxqt.h"
#include "pixelmusic/src/bridges/catalog.cxxqt.h"
#include "pixelmusic/src/bridges/cover_tones.cxxqt.h"
#include "pixelmusic/src/bridges/covers.cxxqt.h"
#include "pixelmusic/src/bridges/crash_report.cxxqt.h"
#include "pixelmusic/src/bridges/data.cxxqt.h"
#include "pixelmusic/src/bridges/devices.cxxqt.h"
#include "pixelmusic/src/bridges/downloads.cxxqt.h"
#include "pixelmusic/src/bridges/extras.cxxqt.h"
#include "pixelmusic/src/bridges/home.cxxqt.h"
#include "pixelmusic/src/bridges/lanjam.cxxqt.h"
#include "pixelmusic/src/bridges/lastfm.cxxqt.h"
#include "pixelmusic/src/bridges/library.cxxqt.h"
#include "pixelmusic/src/bridges/local_files.cxxqt.h"
#include "pixelmusic/src/bridges/lyrics.cxxqt.h"
#include "pixelmusic/src/bridges/player.cxxqt.h"
#include "pixelmusic/src/bridges/queue_history.cxxqt.h"
#include "pixelmusic/src/bridges/radio.cxxqt.h"
#include "pixelmusic/src/bridges/releases.cxxqt.h"
#include "pixelmusic/src/bridges/settings.cxxqt.h"
#include "pixelmusic/src/bridges/smart_mix.cxxqt.h"
#include "pixelmusic/src/bridges/social.cxxqt.h"
#include "pixelmusic/src/bridges/theme.cxxqt.h"
#include "pixelmusic/src/bridges/translations.cxxqt.h"
#include "pixelmusic/src/bridges/updater.cxxqt.h"
#include "pixelmusic/src/icons.cxxqt.h"
#include "pixelmusic/src/models/list_model.cxxqt.h"
#include "pixelmusic/src/models/track_filter.cxxqt.h"
#include "pixelmusic/src/winframe.cxxqt.h"

namespace mf {

namespace {
constexpr const char* URI = "PixelMusic.Core";

template<typename T>
void
singleton(const char* name)
{
  auto* object = new T();
  QQmlEngine::setObjectOwnership(object, QQmlEngine::CppOwnership);
  qmlRegisterSingletonInstance<T>(URI, 1, 0, name, object);
}
} // namespace

void
register_qml_types()
{
  // Порядок важен: Settings и ThemeEngine первыми (их читают остальные при создании)
  singleton<Settings>("Settings");
  singleton<Translations>("Translations");
  singleton<ThemeEngine>("ThemeEngine");
  singleton<IconMetrics>("IconMetrics");
  singleton<Auth>("Auth");
  singleton<Library>("Library");
  singleton<LocalFiles>("LocalFiles");
  singleton<Downloads>("Downloads");
  singleton<Player>("Player");
  singleton<Devices>("Devices");
  singleton<Radio>("Radio");
  singleton<Home>("Home");
  singleton<Releases>("Releases");
  singleton<Social>("Social");
  singleton<Catalog>("Catalog");
  singleton<Covers>("Covers");
  singleton<CoverTones>("CoverTones");
  singleton<CrashReport>("CrashReport");
  singleton<Lyrics>("Lyrics");
  singleton<QueueHistory>("QueueHistory");
  singleton<Updater>("Updater");
  singleton<LastFm>("LastFm");
  singleton<SmartMix>("SmartMix");
  singleton<LanJam>("LanJam");
  singleton<Extras>("Extras");
  singleton<WindowShell>("WindowShell");

  qmlRegisterType<TrackFilter>(URI, 1, 0, "TrackFilter");
  qmlRegisterUncreatableType<DictListModel>(URI, 1, 0, "DictListModel", QStringLiteral("the model is created in Rust"));
  qmlRegisterUncreatableType<AlbumData>(URI, 1, 0, "AlbumData", QStringLiteral("Catalog.album(id)"));
  qmlRegisterUncreatableType<ArtistData>(URI, 1, 0, "ArtistData", QStringLiteral("Catalog.artist(id)"));
  qmlRegisterUncreatableType<ShowData>(URI, 1, 0, "ShowData", QStringLiteral("Catalog.show(id)"));
  qmlRegisterUncreatableType<EpisodeData>(URI, 1, 0, "EpisodeData", QStringLiteral("Catalog.episode(id)"));
}

} // namespace mf
