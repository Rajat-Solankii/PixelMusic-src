// ThemeEngine: цветовые роли MD3 из seed-цвета или обложки (PlayerSnapshot.cover_file). Пересчёт — в spawn_blocking с номером
// запроса: устаревший результат отбрасывается, ещё не разобранная обложка не теряется.
use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QColor, QString, QStringList, QVariant};
use mf_platform::{SettingKey, SettingValue, SettingsStore};
use sf_player::PlayerEvent;
use mf_theme::{Palette, Scheme, Variant};

use super::{Delivery, forward_events, local_path};
use crate::context::ctx;
use crate::qt::QVariantMap;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qcolor.h");
        type QColor = cxx_qt_lib::QColor;
        include!("cxx-qt-lib/qmap.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qproperty(QMap_QString_QVariant, colors, READ, NOTIFY = scheme_changed)]
        /// The colours of the player surfaces (mini player, full player): the playing cover's scheme when the app
        /// keeps its own accent ("Color from cover" off) and Settings → "Player follows the cover" is on; else the
        /// same as `colors`.
        #[qproperty(QMap_QString_QVariant, player_colors, cxx_name = "playerColors", READ, NOTIFY = player_scheme_changed)]
        /// Whether `playerColors` comes from the cover while the app keeps its own colours.
        #[qproperty(bool, player_separate, cxx_name = "playerSeparate", READ, NOTIFY = player_scheme_changed)]
        #[qproperty(QColor, seed, READ = seed_color, NOTIFY = seed_changed)]
        #[qproperty(bool, dark, READ, WRITE = set_dark, NOTIFY = dark_changed)]
        #[qproperty(QString, variant, READ = variant_name, WRITE = set_variant, NOTIFY = variant_changed)]
        #[qproperty(QStringList, variants, READ = variant_names, CONSTANT)]
        type ThemeEngine = super::ThemeEngineRust;

        fn seed_color(self: &ThemeEngine) -> QColor;
        fn variant_name(self: &ThemeEngine) -> QString;
        fn variant_names(self: &ThemeEngine) -> QStringList;
        fn set_dark(self: Pin<&mut ThemeEngine>, value: bool);
        fn set_variant(self: Pin<&mut ThemeEngine>, value: QString);

        #[qsignal]
        #[cxx_name = "schemeChanged"]
        fn scheme_changed(self: Pin<&mut ThemeEngine>);
        #[qsignal]
        #[cxx_name = "playerSchemeChanged"]
        fn player_scheme_changed(self: Pin<&mut ThemeEngine>);
        #[qsignal]
        #[cxx_name = "seedChanged"]
        fn seed_changed(self: Pin<&mut ThemeEngine>);
        #[qsignal]
        #[cxx_name = "darkChanged"]
        fn dark_changed(self: Pin<&mut ThemeEngine>);
        #[qsignal]
        #[cxx_name = "variantChanged"]
        fn variant_changed(self: Pin<&mut ThemeEngine>);

        /// Обложка текущего трека ('' — ничего не играет); акцент берётся из неё, если так настроено.
        #[qinvokable]
        #[cxx_name = "setCover"]
        fn set_cover(self: Pin<&mut ThemeEngine>, source: &QString);
        #[qinvokable]
        #[cxx_name = "setSeed"]
        fn set_seed(self: Pin<&mut ThemeEngine>, color: &QColor);
        /// Путь или file:// URL.
        #[qinvokable]
        #[cxx_name = "setSeedFromImage"]
        fn set_seed_from_image(self: Pin<&mut ThemeEngine>, source: &QString);
        /// The colours a seed would give in the current light/dark mode and scheme style, for the accent picker:
        /// [primary, secondaryContainer, tertiaryContainer] as "#rrggbb" ([] — not a colour).
        #[qinvokable]
        fn swatch(self: &ThemeEngine, seed: &QString) -> QStringList;
        /// The curated palettes, one "id|Name|#c1 #c2 #c3 #c4 #c5" each (lightest colour first); picking one stores
        /// "palette:<id>" in customSeed.
        #[qinvokable]
        fn palettes(self: &ThemeEngine) -> QStringList;
    }

    impl cxx_qt::Threading for ThemeEngine {}
    impl cxx_qt::Initialize for ThemeEngine {}
}

/// Ultraviolet — фирменный акцент: когда ничего не играет и свой цвет не задан. Светлая и тёмная схемы — от него.
pub(crate) const BRAND_SEED: &str = "#8C6DFF";
const DEFAULT_SEED: u32 = 0xFF8C6DFF;
/// Прежний customSeed по умолчанию (зелёный Spotify).
const LEGACY_SEED: &str = "#1DB954";

/// customSeed с учётом бренда: пока хранилище отдаёт прежний зелёный как умолчание, это значит «не задан».
pub(crate) fn custom_seed(store: &SettingsStore) -> String {
    let raw = store.get_str(SettingKey::CustomSeed);
    let legacy_default = matches!(SettingKey::CustomSeed.default_value(), SettingValue::Str(d) if d.eq_ignore_ascii_case(LEGACY_SEED));
    if raw.trim().is_empty() || (legacy_default && raw.trim().eq_ignore_ascii_case(LEGACY_SEED)) {
        BRAND_SEED.to_owned()
    } else {
        raw
    }
}

pub struct ThemeEngineRust {
    colors: QVariantMap,
    player_colors: QVariantMap,
    player_separate: bool,
    /// the latest player-scheme request; older answers are dropped
    player_request: u64,
    /// the cover's seed, read once per cover for the player scheme
    player_seed: Option<(PathBuf, u32)>,
    seed: u32,
    dark: bool,
    variant: Variant,
    /// a curated palette chosen instead of an accent colour (used while the accent is not from a cover)
    palette: Option<&'static Palette>,
    contrast: f64,
    request: u64,
    pending_image: Option<PathBuf>,
    /// обложка текущего трека (путь), даже если акцент не из неё
    cover: Option<PathBuf>,
}

// colors заполняет initialize (синхронно, по настройкам)
impl Default for ThemeEngineRust {
    fn default() -> Self {
        Self {
            colors: QVariantMap::default(),
            player_colors: QVariantMap::default(),
            player_separate: false,
            player_request: 0,
            player_seed: None,
            seed: DEFAULT_SEED,
            dark: true,
            variant: Variant::TonalSpot,
            palette: None,
            contrast: 0.0,
            request: 0,
            pending_image: None,
            cover: None,
        }
    }
}

fn argb_color(argb: u32) -> QColor {
    let [a, r, g, b] = argb.to_be_bytes();
    QColor::from_rgba(r as i32, g as i32, b as i32, a as i32)
}

fn color_argb(color: &QColor) -> u32 {
    let channel = |v: i32| v.clamp(0, 255) as u32;
    (channel(color.alpha()) << 24) | (channel(color.red()) << 16) | (channel(color.green()) << 8) | channel(color.blue())
}

fn to_qcolors(scheme: &Scheme) -> QVariantMap {
    let mut map = QVariantMap::default();
    for (name, argb) in &scheme.colors {
        map.insert(QString::from(*name), QVariant::from(&argb_color(*argb)));
    }
    map
}

/// (dark, variant, seed, palette) по настройкам themeMode / schemeVariant / customSeed и системной теме.
fn settings_state() -> (bool, Variant, u32, Option<&'static Palette>) {
    let store = &ctx().services.settings;
    let mode = store.get_str(SettingKey::ThemeMode);
    let dark = if mode == "system" { *ctx().system_dark.borrow() } else { mode != "light" };
    let variant = Variant::from_choice(&store.get_str(SettingKey::SchemeVariant));
    let choice = custom_seed(store);
    let palette = mf_theme::palette(&choice).filter(|_| choice.trim_start().starts_with("palette:"));
    let seed = match palette {
        Some(p) => p.accent,
        None => mf_theme::parse_color(&choice).unwrap_or(DEFAULT_SEED),
    };
    (dark, variant, seed, palette)
}

/// The scheme for a seed, or for the palette when one is chosen.
fn scheme_for(seed: u32, palette: Option<&'static Palette>, dark: bool, variant: Variant, contrast: f64) -> Scheme {
    match palette {
        Some(p) => mf_theme::palette_scheme(p, dark, variant, contrast),
        None => mf_theme::app_scheme(seed, dark, variant, contrast),
    }
}

fn non_empty_path(path: String) -> Option<PathBuf> {
    (!path.is_empty()).then(|| PathBuf::from(path))
}

impl qobject::ThemeEngine {
    fn seed_color(&self) -> QColor {
        argb_color(self.seed)
    }

    fn variant_name(&self) -> QString {
        QString::from(self.variant.name())
    }

    fn variant_names(&self) -> QStringList {
        Variant::CHOICES.iter().map(|v| QString::from(v.name())).collect()
    }

    fn set_dark(mut self: Pin<&mut Self>, value: bool) {
        if value != self.dark {
            self.as_mut().rust_mut().dark = value;
            self.as_mut().dark_changed();
            self.rebuild(None);
        }
    }

    fn swatch(&self, seed: &QString) -> QStringList {
        let mut list = QStringList::default();
        let text = seed.to_string();
        let scheme = if let Some(p) = mf_theme::palette(&text).filter(|_| text.starts_with("palette:")) {
            mf_theme::palette_scheme(p, self.dark, self.variant, self.contrast)
        } else {
            let Some(seed) = mf_theme::parse_color(&text) else { return list };
            mf_theme::app_scheme(seed, self.dark, self.variant, self.contrast)
        };
        for role in ["primary", "secondaryContainer", "tertiaryContainer"] {
            if let Some(&argb) = scheme.colors.get(role) {
                list.append(QString::from(mf_theme::format_color(argb).as_str()));
            }
        }
        list
    }

    fn palettes(&self) -> QStringList {
        mf_theme::PALETTES
            .iter()
            .map(|p| {
                let colors: Vec<String> = p.colors.iter().map(|&c| mf_theme::format_color(c)).collect();
                QString::from(format!("{}|{}|{}", p.id, p.name, colors.join(" ")).as_str())
            })
            .collect()
    }

    fn set_variant(mut self: Pin<&mut Self>, value: QString) {
        let variant = Variant::from_choice(&value.to_string());
        if variant != self.variant {
            self.as_mut().rust_mut().variant = variant;
            self.as_mut().variant_changed();
            self.rebuild(None);
        }
    }

    fn set_cover(mut self: Pin<&mut Self>, source: &QString) {
        let cover = non_empty_path(local_path(source));
        self.as_mut().rust_mut().cover = cover.clone();
        if ctx().services.settings.get_bool(SettingKey::AccentFromCover) {
            match cover {
                Some(path) => self.rebuild(Some(path)),
                None => self.apply_settings(),
            }
        } else {
            self.update_player();
        }
    }

    /// The player surfaces' own scheme: from the cover when the app keeps its accent and the player follows the
    /// cover, else the app's colours.
    fn update_player(mut self: Pin<&mut Self>) {
        let settings = &ctx().services.settings;
        let separate = !settings.get_bool(SettingKey::AccentFromCover) && settings.get_bool(SettingKey::PlayerFromCover);
        let cover = self.cover.clone().filter(|_| separate);
        let mut this = self.as_mut().rust_mut();
        this.player_request += 1;
        let Some(path) = cover else {
            this.player_colors = this.colors.clone();
            this.player_separate = false;
            self.player_scheme_changed();
            return;
        };
        let (request, dark, variant, contrast) = (this.player_request, this.dark, this.variant, this.contrast);
        let known = this.player_seed.as_ref().filter(|(p, _)| *p == path).map(|(_, seed)| *seed);
        let thread = self.qt_thread();
        ctx().rt.spawn_blocking(move || {
            let seed = match known {
                Some(seed) => seed,
                None => match mf_theme::artwork_seed_from_image(&path) {
                    Ok(seed) => seed,
                    Err(error) => {
                        tracing::debug!("theme (player): {error}");
                        return;
                    }
                },
            };
            let scheme = scheme_for(seed, None, dark, variant, contrast);
            let _ = thread.queue(move |theme| theme.player_ready(request, path, seed, scheme));
        });
    }

    fn player_ready(mut self: Pin<&mut Self>, request: u64, path: PathBuf, seed: u32, scheme: Scheme) {
        if request != self.player_request {
            return;
        }
        let mut this = self.as_mut().rust_mut();
        this.player_seed = Some((path, seed));
        this.player_colors = to_qcolors(&scheme);
        this.player_separate = true;
        self.player_scheme_changed();
    }

    fn set_seed(mut self: Pin<&mut Self>, color: &QColor) {
        let mut this = self.as_mut().rust_mut();
        this.pending_image = None;
        this.palette = None;
        this.seed = color_argb(color);
        self.as_mut().seed_changed();
        self.rebuild(None);
    }

    fn set_seed_from_image(self: Pin<&mut Self>, source: &QString) {
        if let Some(path) = non_empty_path(local_path(source)) {
            self.rebuild(Some(path));
        }
    }

    /// Изменились themeMode/schemeVariant/accentFromCover/customSeed или системная тема.
    fn apply_settings(mut self: Pin<&mut Self>) {
        let (dark, variant, seed, palette) = settings_state();
        self.as_mut().rust_mut().palette = palette;
        if dark != self.dark {
            self.as_mut().rust_mut().dark = dark;
            self.as_mut().dark_changed();
        }
        if variant != self.variant {
            self.as_mut().rust_mut().variant = variant;
            self.as_mut().variant_changed();
        }
        let cover = self.cover.clone();
        match cover {
            Some(path) if ctx().services.settings.get_bool(SettingKey::AccentFromCover) => self.rebuild(Some(path)),
            _ => {
                let mut this = self.as_mut().rust_mut();
                this.pending_image = None;
                this.seed = seed;
                self.as_mut().seed_changed();
                self.rebuild(None);
            }
        }
    }

    /// Пересчёт в пуле потоков; `image` None — взять ещё не разобранную обложку (если есть).
    fn rebuild(mut self: Pin<&mut Self>, image: Option<PathBuf>) {
        let thread = self.qt_thread();
        let mut this = self.as_mut().rust_mut();
        let image = image.or_else(|| this.pending_image.clone());
        this.pending_image = image.clone();
        this.request += 1;
        let (request, seed, dark, variant, contrast) = (this.request, this.seed, this.dark, this.variant, this.contrast);
        let palette = this.palette;
        ctx().rt.spawn_blocking(move || {
            let (seed, palette) = match &image {
                Some(path) => match mf_theme::artwork_seed_from_image(path) {
                    Ok(seed) => (seed, None),
                    Err(error) => {
                        tracing::debug!("theme: {error}");
                        return;
                    }
                },
                None => (seed, palette),
            };
            // black-and-white artwork (or a grey colour) gets a grey scheme, not an invented hue
            let scheme = scheme_for(seed, palette, dark, variant, contrast);
            let _ = thread.queue(move |theme| theme.scheme_ready(request, scheme));
        });
    }

    fn scheme_ready(mut self: Pin<&mut Self>, request: u64, scheme: Scheme) {
        if request != self.request {
            return;
        }
        self.as_mut().rust_mut().pending_image = None;
        if scheme.seed != self.seed {
            self.as_mut().rust_mut().seed = scheme.seed;
            self.as_mut().seed_changed();
        }
        self.as_mut().rust_mut().colors = to_qcolors(&scheme);
        self.as_mut().scheme_changed();
        // the player's scheme follows: the same colours, or the cover's in the new light/dark mode and style
        self.update_player();
    }

    fn on_settings(self: Pin<&mut Self>, delivery: Delivery<SettingKey>) {
        let relevant = match delivery {
            Delivery::Event(key) => matches!(
                key,
                SettingKey::ThemeMode
                    | SettingKey::SchemeVariant
                    | SettingKey::AccentFromCover
                    | SettingKey::PlayerFromCover
                    | SettingKey::CustomSeed
            ),
            Delivery::Lagged => true,
        };
        if relevant {
            self.apply_settings();
        }
    }

    fn on_player(self: Pin<&mut Self>, delivery: Delivery<PlayerEvent>) {
        if matches!(delivery, Delivery::Event(PlayerEvent::CoverFileChanged) | Delivery::Lagged) {
            let file = ctx().services.player.snapshot().cover_file;
            self.set_cover(&QString::from(file.as_str()));
        }
    }
}

impl cxx_qt::Initialize for qobject::ThemeEngine {
    fn initialize(mut self: Pin<&mut Self>) {
        // стартовая схема синхронно: окно не «перекрашивается» при запуске
        let (dark, variant, seed, palette) = settings_state();
        {
            let mut this = self.as_mut().rust_mut();
            this.dark = dark;
            this.variant = variant;
            this.seed = seed;
            this.palette = palette;
            this.colors = to_qcolors(&scheme_for(seed, palette, dark, variant, this.contrast));
            this.player_colors = this.colors.clone();
        }
        self.as_mut().dark_changed();
        self.as_mut().variant_changed();
        self.as_mut().seed_changed();
        self.as_mut().scheme_changed();

        let context = ctx();
        forward_events(self.qt_thread(), context.services.settings.subscribe(), Self::on_settings);
        forward_events(self.qt_thread(), context.services.player.subscribe().events, Self::on_player);
        let thread = self.qt_thread();
        let mut system_dark = context.system_dark.clone();
        context.rt.spawn(async move {
            while system_dark.changed().await.is_ok() {
                if thread.queue(|theme| theme.apply_settings()).is_err() {
                    break;
                }
            }
        });

        // обложка восстановленной сессии могла скачаться до подписки
        let cover_file = context.services.player.snapshot().cover_file;
        if !cover_file.is_empty() {
            self.as_mut().set_cover(&QString::from(cover_file.as_str()));
        }
        // разовые флаги разработки: в настройки не пишутся
        if context.flags.light {
            self.as_mut().set_dark(false);
        }
        if let Some(path) = &context.flags.seed_image {
            let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
            self.as_mut().set_seed_from_image(&QString::from(path.to_string_lossy().as_ref()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_seed_until_custom_seed_is_chosen() {
        let store = SettingsStore::in_memory();
        assert!(custom_seed(&store).eq_ignore_ascii_case(BRAND_SEED));
        assert_eq!(mf_theme::parse_color(&custom_seed(&store)), Some(DEFAULT_SEED));
        store.set(SettingKey::CustomSeed, SettingValue::Str("#FF6D00".into()));
        assert_eq!(custom_seed(&store), "#FF6D00");
    }
}
