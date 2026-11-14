//! Цветовая схема MD3 (src/pixelmusic/theme.py): materialyoucolor 3.0.4, spec "2025", platform "phone".
//! Эталон — .venv/Lib/site-packages/materialyoucolor; значения должны совпадать с Python до бита
//! (см. тест-векторы в tests/vectors.json). Из material-colors берутся только HCT и палитры схем;
//! разрешение ролей (spec.rs), QuantizeCelebi (quantize.rs), Score (score.rs) и сглаживающий масштаб Qt
//! (scale.rs) портированы с эталона — у material-colors они расходятся с Python.
use std::collections::BTreeMap;
use std::path::Path;

use material_colors::color::Rgb;
use material_colors::dynamic_color::{
    DynamicScheme, Platform, Role, SpecVersion, Variant as MaterialVariant,
};
use material_colors::hct::Hct;

pub mod icons;
mod palettes;
pub use palettes::{palette, palette_scheme, Palette, PALETTES};
mod quantize;
pub use quantize::CrtRand;
mod scale;
mod score;
mod spec;

pub const DEFAULT_SEED: u32 = 0xFF6750A4;

/// 59 ролей — ровно COLOR_NAMES из materialyoucolor (ключи ThemeEngine.colors в QML).
pub const ROLE_NAMES: [&str; 59] = [
    "background",
    "onBackground",
    "surface",
    "surfaceDim",
    "surfaceBright",
    "surfaceContainerLowest",
    "surfaceContainerLow",
    "surfaceContainer",
    "surfaceContainerHigh",
    "surfaceContainerHighest",
    "onSurface",
    "surfaceVariant",
    "onSurfaceVariant",
    "outline",
    "outlineVariant",
    "inverseSurface",
    "inverseOnSurface",
    "shadow",
    "scrim",
    "surfaceTint",
    "primary",
    "primaryDim",
    "onPrimary",
    "primaryContainer",
    "onPrimaryContainer",
    "inversePrimary",
    "primaryFixed",
    "primaryFixedDim",
    "onPrimaryFixed",
    "onPrimaryFixedVariant",
    "secondary",
    "secondaryDim",
    "onSecondary",
    "secondaryContainer",
    "onSecondaryContainer",
    "secondaryFixed",
    "secondaryFixedDim",
    "onSecondaryFixed",
    "onSecondaryFixedVariant",
    "tertiary",
    "tertiaryDim",
    "onTertiary",
    "tertiaryContainer",
    "onTertiaryContainer",
    "tertiaryFixed",
    "tertiaryFixedDim",
    "onTertiaryFixed",
    "onTertiaryFixedVariant",
    "error",
    "errorDim",
    "onError",
    "errorContainer",
    "onErrorContainer",
    "primaryPaletteKeyColor",
    "secondaryPaletteKeyColor",
    "tertiaryPaletteKeyColor",
    "neutralPaletteKeyColor",
    "neutralVariantPaletteKeyColor",
    "errorPaletteKeyColor",
];

/// Роли material-colors в порядке ROLE_NAMES.
const ROLES: [Role; 59] = [
    Role::Background,
    Role::OnBackground,
    Role::Surface,
    Role::SurfaceDim,
    Role::SurfaceBright,
    Role::SurfaceContainerLowest,
    Role::SurfaceContainerLow,
    Role::SurfaceContainer,
    Role::SurfaceContainerHigh,
    Role::SurfaceContainerHighest,
    Role::OnSurface,
    Role::SurfaceVariant,
    Role::OnSurfaceVariant,
    Role::Outline,
    Role::OutlineVariant,
    Role::InverseSurface,
    Role::InverseOnSurface,
    Role::Shadow,
    Role::Scrim,
    Role::SurfaceTint,
    Role::Primary,
    Role::PrimaryDim,
    Role::OnPrimary,
    Role::PrimaryContainer,
    Role::OnPrimaryContainer,
    Role::InversePrimary,
    Role::PrimaryFixed,
    Role::PrimaryFixedDim,
    Role::OnPrimaryFixed,
    Role::OnPrimaryFixedVariant,
    Role::Secondary,
    Role::SecondaryDim,
    Role::OnSecondary,
    Role::SecondaryContainer,
    Role::OnSecondaryContainer,
    Role::SecondaryFixed,
    Role::SecondaryFixedDim,
    Role::OnSecondaryFixed,
    Role::OnSecondaryFixedVariant,
    Role::Tertiary,
    Role::TertiaryDim,
    Role::OnTertiary,
    Role::TertiaryContainer,
    Role::OnTertiaryContainer,
    Role::TertiaryFixed,
    Role::TertiaryFixedDim,
    Role::OnTertiaryFixed,
    Role::OnTertiaryFixedVariant,
    Role::Error,
    Role::ErrorDim,
    Role::OnError,
    Role::ErrorContainer,
    Role::OnErrorContainer,
    Role::PrimaryPaletteKeyColor,
    Role::SecondaryPaletteKeyColor,
    Role::TertiaryPaletteKeyColor,
    Role::NeutralPaletteKeyColor,
    Role::NeutralVariantPaletteKeyColor,
    Role::ErrorPaletteKeyColor,
];

/// Варианты схемы; имена — значения Settings.schemeVariant и ThemeEngine.variants (в этом порядке).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Variant {
    Content,
    /// Calm pastel containers (PixelPlayer's default "Tonal Spot"); the app's default.
    #[default]
    TonalSpot,
    Vibrant,
    Expressive,
    Fidelity,
    Neutral,
    Monochrome,
    /// Primary hue turned 50° back, playful pastel containers (spec 2021 palettes).
    FruitSalad,
    Rainbow,
}

impl Variant {
    pub const ALL: [Variant; 9] = [
        Variant::Content,
        Variant::TonalSpot,
        Variant::Vibrant,
        Variant::Expressive,
        Variant::Fidelity,
        Variant::Neutral,
        Variant::Monochrome,
        Variant::FruitSalad,
        Variant::Rainbow,
    ];

    /// "content" | "tonalSpot" | "vibrant" | "expressive" | "fidelity" | "neutral" | "monochrome" | "fruitSalad" | "rainbow"
    pub fn name(self) -> &'static str {
        match self {
            Variant::Content => "content",
            Variant::TonalSpot => "tonalSpot",
            Variant::Vibrant => "vibrant",
            Variant::Expressive => "expressive",
            Variant::Fidelity => "fidelity",
            Variant::Neutral => "neutral",
            Variant::Monochrome => "monochrome",
            Variant::FruitSalad => "fruitSalad",
            Variant::Rainbow => "rainbow",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }

    /// The schemes the user can pick: the Material 3 Expressive (spec 2025) dynamic-colour variants, plus
    /// Monochrome. Content, Fidelity, Fruit salad and Rainbow are the older spec-2021 palettes; they stay in `ALL`
    /// only for the reference vectors and are no longer offered.
    pub const CHOICES: [Variant; 5] =
        [Variant::TonalSpot, Variant::Neutral, Variant::Vibrant, Variant::Expressive, Variant::Monochrome];

    /// A stored choice; an older palette (or anything unknown) becomes the default Tonal Spot.
    pub fn from_choice(name: &str) -> Self {
        Self::CHOICES.into_iter().find(|v| v.name() == name).unwrap_or_default()
    }

    /// Имена в порядке ThemeEngine.variants.
    pub fn names() -> [&'static str; 9] {
        Self::ALL.map(Variant::name)
    }

    fn material(self) -> MaterialVariant {
        match self {
            Variant::Content => MaterialVariant::Content,
            Variant::TonalSpot => MaterialVariant::TonalSpot,
            Variant::Vibrant => MaterialVariant::Vibrant,
            Variant::Expressive => MaterialVariant::Expressive,
            Variant::Fidelity => MaterialVariant::Fidelity,
            Variant::Neutral => MaterialVariant::Neutral,
            Variant::Monochrome => MaterialVariant::Monochrome,
            Variant::FruitSalad => MaterialVariant::FruitSalad,
            Variant::Rainbow => MaterialVariant::Rainbow,
        }
    }
}

impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Variant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        Self::from_name(s).ok_or(())
    }
}

/// ARGB всех 59 ролей.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub seed: u32,
    pub dark: bool,
    pub variant: Variant,
    pub colors: BTreeMap<&'static str, u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("could not open image: {0}")]
    Image(String),
}

/// Схема из seed (build_scheme в theme.py). ~1 мс; можно звать из GUI-потока для стартовой схемы.
pub fn build_scheme(seed: u32, dark: bool, variant: Variant, contrast: f64) -> Scheme {
    // Content/Fidelity/Monochrome сами откатываются на spec 2021 — как _maybe_fallback_spec_version в Python
    let scheme = DynamicScheme::from_spec(
        Hct::new(Rgb::from_u32(seed)),
        variant.material(),
        dark,
        Some(contrast),
        Platform::Phone,
        SpecVersion::Spec2025,
    );
    // палитры — из material-colors, разрешение ролей — порт Python (spec.rs)
    let resolver = spec::Resolver::new(&scheme);
    let colors = ROLE_NAMES
        .iter()
        .zip(ROLES)
        .map(|(&name, role)| (name, resolver.argb(role)))
        .collect();
    Scheme {
        seed,
        dark,
        variant,
        colors,
    }
}

/// A seed with no real colour (black-and-white artwork, a grey custom colour): a tinted scheme would invent a hue
/// the picture does not have, so such seeds get the grey Monochrome scheme (as PixelPlayer does).
pub fn is_grayscale(seed: u32) -> bool {
    let [_, r, g, b] = seed.to_be_bytes();
    let delta = r.max(g).max(b) - r.min(g).min(b);
    Hct::new(Rgb::from_u32(seed)).get_chroma() <= 8.0 && delta <= 20
}

/// The variant to build for : Monochrome for grey seeds, otherwise the chosen one.
pub fn variant_for_seed(seed: u32, chosen: Variant) -> Variant {
    if is_grayscale(seed) { Variant::Monochrome } else { chosen }
}

/// The app's scheme: `build_scheme` with grey seeds turned Monochrome. For Monochrome the accent roles follow
/// PixelPlayer Desktop (ColorRoles.toComposeColorScheme): its grey palettes with the usual tone table, so the
/// containers stay deep in dark mode and pale in light mode. The Monochrome spec itself puts primaryContainer at
/// T85 in dark and T25 in light, which turned Now Playing's background light in dark mode and dark in light mode.
pub fn app_scheme(seed: u32, dark: bool, chosen: Variant, contrast: f64) -> Scheme {
    let variant = variant_for_seed(seed, chosen);
    let mut scheme = build_scheme(seed, dark, variant, contrast);
    if variant != Variant::Monochrome {
        return scheme;
    }
    let palettes = DynamicScheme::from_spec(
        Hct::new(Rgb::from_u32(seed)),
        variant.material(),
        dark,
        Some(contrast),
        Platform::Phone,
        SpecVersion::Spec2025,
    );
    let tone = |palette: &material_colors::palette::TonalPalette, t: f64| -> u32 {
        Rgb::from(Hct::from(palette.hue(), palette.chroma(), t)).as_argb_u32()
    };
    // (role, dark tone, light tone)
    let accents: [(&str, f64, f64); 12] = [
        ("primary", 80.0, 40.0),
        ("onPrimary", 20.0, 100.0),
        ("primaryContainer", 30.0, 90.0),
        ("onPrimaryContainer", 90.0, 10.0),
        ("secondary", 80.0, 40.0),
        ("onSecondary", 20.0, 100.0),
        ("secondaryContainer", 30.0, 90.0),
        ("onSecondaryContainer", 90.0, 10.0),
        ("tertiary", 80.0, 40.0),
        ("onTertiary", 20.0, 100.0),
        ("tertiaryContainer", 30.0, 90.0),
        ("onTertiaryContainer", 90.0, 10.0),
    ];
    for (role, dark_tone, light_tone) in accents {
        let palette = match role.trim_start_matches("on").to_ascii_lowercase() {
            r if r.starts_with("primary") => &palettes.primary_palette,
            r if r.starts_with("secondary") => &palettes.secondary_palette,
            _ => &palettes.tertiary_palette,
        };
        if let Some(slot) = scheme.colors.get_mut(role) {
            *slot = tone(palette, if dark { dark_tone } else { light_tone });
        }
    }
    for (role, dark_tone, light_tone) in [("inversePrimary", 40.0, 80.0), ("surfaceTint", 80.0, 40.0)] {
        if let Some(slot) = scheme.colors.get_mut(role) {
            *slot = tone(&palettes.primary_palette, if dark { dark_tone } else { light_tone });
        }
    }
    scheme
}

/// Сторона картинки, до которой она уменьшается перед квантованием.
pub const SAMPLE_SIZE: u32 = 112;
/// Число кластеров QuantizeCelebi.
pub const MAX_COLORS: usize = 128;

/// Доминантный цвет картинки (seed_from_image): масштаб до 112×112 (сглаживание, без сохранения пропорций),
/// RGB, QuantizeCelebi(pixels, 128) → Score.score → первый; пусто → DEFAULT_SEED.
///
/// Отличия от Python: PNG дают те же пиксели, что Qt, с точностью до единицы в редких краевых пикселях;
/// JPEG декодирует не libjpeg-turbo, а zune-jpeg (≈1 % отсчётов на 1–2 уровня), поэтому у части обложек seed
/// выходит соседним оттенком. Квантование и ранжирование на одних и тех же пикселях совпадают точно.
/// Блокирующая (декодирование + ~10 мс квантования) — звать из spawn_blocking.
pub fn seed_from_image(path: &Path) -> Result<u32, ThemeError> {
    seed_from_image_with(path, CrtRand::native())
}

/// seed_from_image с явным выбором rand() (см. CrtRand).
pub fn seed_from_image_with(path: &Path, crt: CrtRand) -> Result<u32, ThemeError> {
    let image = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| ThemeError::Image(format!("{}: {e}", path.display())))?
        .decode()
        .map_err(|e| ThemeError::Image(format!("{}: {e}", path.display())))?;
    Ok(seed_from_rgba_with(&image.to_rgba8(), crt))
}

/// То же по уже декодированной картинке (например, из памяти).
pub fn seed_from_rgba(image: &image::RgbaImage) -> u32 {
    seed_from_rgba_with(image, CrtRand::native())
}

pub fn seed_from_rgba_with(image: &image::RgbaImage, crt: CrtRand) -> u32 {
    seed_from_pixels_with(
        &scale::smooth_scale_rgb(image, SAMPLE_SIZE, SAMPLE_SIZE),
        crt,
    )
}

/// То же по готовым пикселям RGB.
pub fn seed_from_pixels(pixels: &[[u8; 3]]) -> u32 {
    seed_from_pixels_with(pixels, CrtRand::native())
}

pub fn seed_from_pixels_with(pixels: &[[u8; 3]], crt: CrtRand) -> u32 {
    seed_with_scoring(pixels, crt, score::score)
}

/// The seed the app uses for album artwork: same sampling and quantizing as `seed_from_image`, but ranked by
/// `score::score_accurate` (hue population + fidelity to the artwork's representative colour, like PixelPlayer),
/// so a small neon accent no longer wins over the colour the cover is made of. Blocking, like `seed_from_image`.
pub fn artwork_seed_from_image(path: &Path) -> Result<u32, ThemeError> {
    let image = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| ThemeError::Image(format!("{}: {e}", path.display())))?
        .decode()
        .map_err(|e| ThemeError::Image(format!("{}: {e}", path.display())))?;
    Ok(artwork_seed_from_pixels(&scale::smooth_scale_rgb(&image.to_rgba8(), SAMPLE_SIZE, SAMPLE_SIZE)))
}

/// `artwork_seed_from_image` on ready RGB pixels.
pub fn artwork_seed_from_pixels(pixels: &[[u8; 3]]) -> u32 {
    seed_with_scoring(pixels, CrtRand::native(), score::score_accurate)
}

fn seed_with_scoring(pixels: &[[u8; 3]], crt: CrtRand, rank: fn(&[(u32, u32)]) -> Vec<u32>) -> u32 {
    if pixels.is_empty() {
        return DEFAULT_SEED;
    }
    let argb: Vec<u32> = pixels
        .iter()
        .map(|&[r, g, b]| 0xFF00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
        .collect();
    let population = quantize::quantize_celebi(&argb, MAX_COLORS, crt);
    let ranked = rank(&population);
    // no colourful colour at all (black-and-white artwork): the score falls back to Google blue, a hue the
    // picture does not have; its most common colour (a grey) is the accurate seed
    if ranked == [score::FALLBACK] && !population.iter().any(|&(argb, _)| argb == score::FALLBACK) {
        if let Some(&(dominant, _)) = population.iter().max_by_key(|&&(_, count)| count) {
            return dominant;
        }
    }
    ranked.first().copied().unwrap_or(DEFAULT_SEED)
}

/// The colours of one cover for a small tinted surface (Home's recently-played pills): the container tone of the
/// cover's own scheme and the text colours that read on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverTones {
    pub container: u32,
    pub on_container: u32,
    /// secondary text: `on_container` 80 % over the container (PixelPlayer's artist line), still well above 4.5:1
    pub on_container_variant: u32,
}

/// `CoverTones` of a seed: primaryContainer / onPrimaryContainer of its Tonal Spot scheme (grey seeds: Monochrome).
/// A grey cover takes the neutral surfaceContainerHighest / onSurface pair instead: Monochrome's primaryContainer
/// is near white in the dark scheme, a glaring pill on a dark Home.
pub fn cover_tones(seed: u32, dark: bool) -> CoverTones {
    let grey = is_grayscale(seed);
    let scheme = build_scheme(seed, dark, variant_for_seed(seed, Variant::TonalSpot), 0.0);
    let (container_role, on_role) =
        if grey { ("surfaceContainerHighest", "onSurface") } else { ("primaryContainer", "onPrimaryContainer") };
    let container = scheme.colors[container_role];
    let on_container = scheme.colors[on_role];
    CoverTones { container, on_container, on_container_variant: blend(on_container, container, 0.80) }
}

/// Light and dark `CoverTones` of a decoded cover (seed as for the app theme, `artwork_seed_from_pixels`).
pub fn cover_tones_from_rgba(image: &image::RgbaImage) -> (CoverTones, CoverTones) {
    let seed = artwork_seed_from_pixels(&scale::smooth_scale_rgb(image, SAMPLE_SIZE, SAMPLE_SIZE));
    (cover_tones(seed, false), cover_tones(seed, true))
}

/// `cover_tones_from_rgba` of an image file. Blocking (decoding + quantizing): call from spawn_blocking.
pub fn cover_tones_from_image(path: &Path) -> Result<(CoverTones, CoverTones), ThemeError> {
    let image = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| ThemeError::Image(format!("{}: {e}", path.display())))?
        .decode()
        .map_err(|e| ThemeError::Image(format!("{}: {e}", path.display())))?;
    Ok(cover_tones_from_rgba(&image.to_rgba8()))
}

/// `front` over `back` with the given opacity of `front` (opaque result).
fn blend(front: u32, back: u32, amount: f64) -> u32 {
    let channel = |shift: u32| {
        let (f, b) = (f64::from((front >> shift) & 0xFF), f64::from((back >> shift) & 0xFF));
        ((f * amount + b * (1.0 - amount)).round() as u32).min(255) << shift
    };
    0xFF00_0000 | channel(16) | channel(8) | channel(0)
}

/// WCAG contrast ratio of two opaque colours (1..21).
pub fn contrast_ratio(a: u32, b: u32) -> f64 {
    let luminance = |argb: u32| {
        let linear = |shift: u32| {
            let c = f64::from((argb >> shift) & 0xFF) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
    };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// "#RRGGBB" / "#AARRGGBB" → ARGB (альфа FF), как QColor(str); неверная строка → None.
/// Как и QColor, понимает ещё "#RGB", "#RRRGGGBBB" и "#RRRRGGGGBBBB" (берутся старшие 8 бит канала).
pub fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = |s: &str| u32::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let v = value(hex)?;
            let (r, g, b) = ((v >> 8) & 0xF, (v >> 4) & 0xF, v & 0xF);
            Some(0xFF00_0000 | (r * 0x11) << 16 | (g * 0x11) << 8 | (b * 0x11))
        }
        6 => Some(0xFF00_0000 | value(hex)?),
        8 => value(hex),
        9 | 12 => {
            let n = hex.len() / 3;
            let channel = |i: usize| value(&hex[i * n..(i + 1) * n]).map(|c| c >> ((n - 2) * 4));
            Some(0xFF00_0000 | channel(0)? << 16 | channel(1)? << 8 | channel(2)?)
        }
        _ => None,
    }
}

/// ARGB → "#AARRGGBB" (верхний регистр, как в тест-векторах).
pub fn format_color(argb: u32) -> String {
    format!("#{argb:08X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grey_cover_keeps_deep_containers_in_dark_mode_and_pale_ones_in_light_mode() {
        let grey = 0xFF80_8080;
        let luminance = |argb: u32| {
            let [_, r, g, b] = argb.to_be_bytes();
            (u32::from(r) + u32::from(g) + u32::from(b)) / 3
        };
        let dark = app_scheme(grey, true, Variant::TonalSpot, 0.0);
        let light = app_scheme(grey, false, Variant::TonalSpot, 0.0);
        assert_eq!(dark.variant, Variant::Monochrome);
        assert!(luminance(dark.colors["primaryContainer"]) < 90, "{:08x}", dark.colors["primaryContainer"]);
        assert!(luminance(dark.colors["onPrimaryContainer"]) > 180);
        assert!(luminance(light.colors["primaryContainer"]) > 190, "{:08x}", light.colors["primaryContainer"]);
        assert!(luminance(light.colors["onPrimaryContainer"]) < 60);
        // a colourful seed is the plain build_scheme
        let red = app_scheme(0xFFE9_1E63, true, Variant::TonalSpot, 0.0);
        assert_eq!(red.colors, build_scheme(0xFFE9_1E63, true, Variant::TonalSpot, 0.0).colors);
    }

    fn camel(snake: &str) -> String {
        let mut out = String::new();
        let mut upper = false;
        for c in snake.chars() {
            if c == '_' {
                upper = true;
            } else if upper {
                out.extend(c.to_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn roles_match_names() {
        for (name, role) in ROLE_NAMES.iter().zip(ROLES) {
            assert_eq!(*name, camel(role.name()));
        }
    }

    #[test]
    fn variant_names() {
        assert_eq!(
            Variant::names(),
            [
                "content",
                "tonalSpot",
                "vibrant",
                "expressive",
                "fidelity",
                "neutral",
                "monochrome",
                "fruitSalad",
                "rainbow"
            ]
        );
        for v in Variant::ALL {
            assert_eq!(Variant::from_name(v.name()), Some(v));
        }
        assert_eq!(Variant::from_name("spritz"), None);
        assert_eq!(Variant::default(), Variant::TonalSpot);
    }

    #[test]
    fn black_and_white_artwork_gives_a_grey_seed_and_a_grey_scheme() {
        let pixels: Vec<[u8; 3]> = (0..4096).map(|i| if i % 3 == 0 { [20, 20, 20] } else { [200, 200, 200] }).collect();
        let seed = seed_from_pixels(&pixels);
        assert!(is_grayscale(seed), "{seed:08X}");
        let seed = artwork_seed_from_pixels(&pixels);
        assert!(is_grayscale(seed), "{seed:08X}");
        assert_eq!(variant_for_seed(seed, Variant::TonalSpot), Variant::Monochrome);
        assert_eq!(variant_for_seed(0xFFE91E63, Variant::TonalSpot), Variant::TonalSpot);
        // every variant builds, the new spec 2021 ones too
        for v in Variant::ALL {
            let scheme = build_scheme(0xFFE91E63, true, v, 0.0);
            assert_eq!(scheme.colors.len(), ROLE_NAMES.len());
        }
    }

    #[test]
    fn parse_colors() {
        assert_eq!(parse_color("#6750A4"), Some(0xFF6750A4));
        assert_eq!(parse_color("#806750a4"), Some(0x806750A4));
        assert_eq!(parse_color("#fc0"), Some(0xFFFFCC00));
        assert_eq!(parse_color("#ffffcccc0000"), Some(0xFFFFCC00));
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("6750A4"), None);
        assert_eq!(parse_color("#6750A"), None);
        assert_eq!(parse_color("#zzzzzz"), None);
        assert_eq!(format_color(0xFF6750A4), "#FF6750A4");
    }

    #[test]
    fn empty_pixels_default_seed() {
        assert_eq!(seed_from_pixels(&[]), DEFAULT_SEED);
        assert_eq!(artwork_seed_from_pixels(&[]), DEFAULT_SEED);
    }

    #[test]
    fn artwork_seed_follows_the_main_colour_not_a_small_neon_accent() {
        // a 64×64 cover: 40 % muted olive, 25 % neon magenta, 35 % light grey (CAM16 hue ~209°, far from both)
        let olive = [112u8, 106, 78];
        let magenta = [255u8, 0, 200];
        let pixels: Vec<[u8; 3]> = (0..4096)
            .map(|i| match i % 20 { 0..=7 => olive, 8..=12 => magenta, _ => [200, 200, 200] })
            .collect();
        let hue = |argb: u32| material_colors::hct::Hct::new(material_colors::color::Rgb::from_u32(argb)).get_hue();
        let near = |a: f64, b: f64| 180.0 - ((a - b).abs() - 180.0).abs() < 15.0;
        let olive_hue = hue(0xFF00_0000 | 112 << 16 | 106 << 8 | 78);
        let magenta_hue = hue(0xFFFF_00C8);
        // the stock score takes the accent, the artwork seed the olive
        assert!(near(hue(seed_from_pixels(&pixels)), magenta_hue));
        assert!(near(hue(artwork_seed_from_pixels(&pixels)), olive_hue));
    }

    fn solid(r: u8, g: u8, b: u8) -> image::RgbaImage {
        image::RgbaImage::from_pixel(32, 32, image::Rgba([r, g, b, 255]))
    }

    #[test]
    fn cover_tones_take_the_hue_of_the_cover_and_keep_their_text_readable() {
        let hue = |argb: u32| Hct::new(Rgb::from_u32(argb)).get_hue();
        let distance = |a: f64, b: f64| { let d = (a - b).abs() % 360.0; d.min(360.0 - d) };
        for (r, g, b) in [(200, 30, 40), (20, 90, 200), (40, 160, 70), (230, 180, 20)] {
            let cover_hue = hue(0xFF00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b));
            let (light, dark) = cover_tones_from_rgba(&solid(r, g, b));
            for tones in [light, dark] {
                assert!(distance(hue(tones.container), cover_hue) < 30.0, "{tones:?} vs hue {cover_hue}");
                assert!(contrast_ratio(tones.on_container, tones.container) >= 4.5, "{tones:?}");
                assert!(contrast_ratio(tones.on_container_variant, tones.container) >= 3.0, "{tones:?}");
            }
            // the light container is light, the dark one dark
            assert!(contrast_ratio(light.container, 0xFFFF_FFFF) < contrast_ratio(dark.container, 0xFFFF_FFFF));
        }
    }

    #[test]
    fn a_grey_cover_gets_grey_tones_and_different_covers_differ() {
        let (light, dark) = cover_tones_from_rgba(&solid(128, 128, 128));
        for argb in [light.container, dark.container] {
            assert!(Hct::new(Rgb::from_u32(argb)).get_chroma() < 8.0, "{argb:08X}");
        }
        assert_ne!(cover_tones_from_rgba(&solid(200, 30, 40)), cover_tones_from_rgba(&solid(20, 90, 200)));
    }

    #[test]
    fn only_the_expressive_variants_are_offered_and_old_palettes_fall_back() {
        assert_eq!(Variant::from_choice("vibrant"), Variant::Vibrant);
        assert_eq!(Variant::from_choice("monochrome"), Variant::Monochrome);
        for old in ["content", "fidelity", "fruitSalad", "rainbow", "", "nonsense"] {
            assert_eq!(Variant::from_choice(old), Variant::TonalSpot, "{old}");
        }
        assert!(Variant::CHOICES.iter().all(|v| Variant::ALL.contains(v)));
    }

    #[test]
    fn contrast_ratio_spans_one_to_twenty_one() {
        assert!((contrast_ratio(0xFF00_0000, 0xFFFF_FFFF) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(0xFF33_6699, 0xFF33_6699) - 1.0).abs() < 1e-9);
    }
}
