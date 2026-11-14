//! Curated colour palettes: pastel themes whose own colours become the app's surfaces and containers.
//!
//! A seed builds a Material 3 scheme in which every colour is derived from one hue. A palette is the other way
//! round: five hand-picked colours (the first four from the Android app's light themes) that are used **as they
//! are** in light mode: the lightest one tints the background and surface containers, three others become the
//! primary / secondary / tertiary containers, and the accent drives `primary`. The text on each container is the
//! same hue taken down to tone 10, so it always reads. In dark mode the scheme stays dark (from the accent, with
//! the chosen style), and the containers take the palette's hues at the usual dark tones.
use material_colors::color::Rgb;
use material_colors::hct::Hct;

use crate::{app_scheme, Scheme, Variant};

/// One palette; colours are 0xFFRRGGBB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Stored after "palette:" in the customSeed setting.
    pub id: &'static str,
    pub name: &'static str,
    /// The background and surface containers take this colour's hue.
    pub surface: u32,
    /// `primary` (buttons, sliders, the selected tab) is this colour's hue at tone 40 / 80.
    pub accent: u32,
    pub primary_container: u32,
    pub secondary_container: u32,
    pub tertiary_container: u32,
    /// All five colours, lightest first, for the picker.
    pub colors: [u32; 5],
}

pub const PALETTES: [Palette; 8] = [
    // ── the Android app's four light themes ──
    Palette {
        id: "softPink",
        name: "Soft Pink Delight",
        surface: 0xFFFFE5EC,
        accent: 0xFFFB6F92,
        primary_container: 0xFFFFB3C6,
        secondary_container: 0xFFFFC2D1,
        tertiary_container: 0xFFFF8FAB,
        colors: [0xFFFFE5EC, 0xFFFFC2D1, 0xFFFFB3C6, 0xFFFF8FAB, 0xFFFB6F92],
    },
    Palette {
        id: "goldenGlow",
        name: "Golden Glow",
        surface: 0xFFFDF8E1,
        accent: 0xFFF9DC5C,
        primary_container: 0xFFF9DC5C,
        secondary_container: 0xFFFCEFB4,
        tertiary_container: 0xFFFAE588,
        colors: [0xFFFDF8E1, 0xFFFCEFB4, 0xFFFAE588, 0xFFF9DC5C, 0xFFE9B824],
    },
    Palette {
        id: "cottonCandy",
        name: "Cotton Candy",
        surface: 0xFFFFC8DD,
        accent: 0xFFA2D2FF,
        primary_container: 0xFFBDE0FE,
        secondary_container: 0xFFFFC8DD,
        tertiary_container: 0xFFCDB4DB,
        colors: [0xFFFFC8DD, 0xFFBDE0FE, 0xFFFFAFCC, 0xFFA2D2FF, 0xFFCDB4DB],
    },
    Palette {
        id: "sageSand",
        name: "Sage & Sand",
        surface: 0xFFFEFAE0,
        accent: 0xFFCCD5AE,
        primary_container: 0xFFCCD5AE,
        secondary_container: 0xFFFAEDCD,
        tertiary_container: 0xFFE9EDC9,
        colors: [0xFFFEFAE0, 0xFFFAEDCD, 0xFFE9EDC9, 0xFFCCD5AE, 0xFFD4A373],
    },
    // ── four more in the same spirit ──
    Palette {
        id: "coralReef",
        name: "Coral Reef",
        surface: 0xFFFFDBBA,
        accent: 0xFFFF8894,
        primary_container: 0xFFFFACA8,
        secondary_container: 0xFFA1E0DD,
        tertiary_container: 0xFFFFDBBA,
        colors: [0xFFFFDBBA, 0xFFFFACA8, 0xFFFF8894, 0xFFA1E0DD, 0xFF5CC2C6],
    },
    Palette {
        id: "candySky",
        name: "Candy Sky",
        surface: 0xFFF7DFF6,
        accent: 0xFF5EAEE6,
        primary_container: 0xFFA4E9FF,
        secondary_container: 0xFFF7DFF6,
        tertiary_container: 0xFFFFB3D6,
        colors: [0xFFF7DFF6, 0xFFA4E9FF, 0xFFFFB3D6, 0xFFC38FC2, 0xFF5EAEE6],
    },
    Palette {
        id: "oceanBreeze",
        name: "Ocean Breeze",
        surface: 0xFFCAF0F8,
        accent: 0xFF00B4D8,
        primary_container: 0xFF90E0EF,
        secondary_container: 0xFFADE8F4,
        tertiary_container: 0xFFB8E0D2,
        colors: [0xFFCAF0F8, 0xFFADE8F4, 0xFF90E0EF, 0xFF48CAE4, 0xFF00B4D8],
    },
    Palette {
        id: "sunsetSorbet",
        name: "Sunset Sorbet",
        surface: 0xFFFFCDB2,
        accent: 0xFFE5989B,
        primary_container: 0xFFFFB4A2,
        secondary_container: 0xFFFFD9C7,
        tertiary_container: 0xFFE5989B,
        colors: [0xFFFFCDB2, 0xFFFFB4A2, 0xFFE5989B, 0xFFB5838D, 0xFF6D6875],
    },
];

/// The palette stored as "palette:<id>" (or a bare id).
pub fn palette(choice: &str) -> Option<&'static Palette> {
    let id = choice.trim();
    let id = id.strip_prefix("palette:").unwrap_or(id);
    PALETTES.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

fn hct(argb: u32) -> Hct {
    Hct::new(Rgb::from_u32(argb))
}

fn tone_of(argb: u32, chroma: f64, tone: f64) -> u32 {
    let h = hct(argb);
    Rgb::from(Hct::from(h.get_hue(), chroma, tone)).as_argb_u32()
}

/// The scheme of a palette in light or dark mode; `chosen` is the scheme style used for the dark scheme.
pub fn palette_scheme(p: &Palette, dark: bool, chosen: Variant, contrast: f64) -> Scheme {
    let mut scheme = app_scheme(p.accent, dark, chosen, contrast);
    let mut set = |role: &'static str, argb: u32| {
        if let Some(slot) = scheme.colors.get_mut(role) {
            *slot = argb | 0xFF00_0000;
        }
    };
    let chroma = |argb: u32| hct(argb).get_chroma();
    let accent_chroma = chroma(p.accent).max(36.0);
    let containers = [
        ("primary", p.primary_container),
        ("secondary", p.secondary_container),
        ("tertiary", p.tertiary_container),
    ];

    if dark {
        // a dark scheme in the palette's hues: deep containers, light text and accents
        set("primary", tone_of(p.accent, accent_chroma, 80.0));
        set("onPrimary", tone_of(p.accent, accent_chroma, 20.0));
        set("surfaceTint", tone_of(p.accent, accent_chroma, 80.0));
        set("inversePrimary", tone_of(p.accent, accent_chroma, 40.0));
        for (name, color) in containers {
            let c = chroma(color).clamp(24.0, 48.0);
            set(role(name, ""), tone_of(color, c, 80.0));
            set(role(name, "on"), tone_of(color, c, 20.0));
            set(role(name, "container"), tone_of(color, c, 30.0));
            set(role(name, "onContainer"), tone_of(color, c, 90.0));
        }
        return scheme;
    }

    // Light: the palette's colours themselves.
    let surface_chroma = chroma(p.surface).min(16.0);
    let surface = |tone: f64| tone_of(p.surface, surface_chroma, tone);
    for (name, tone) in [
        ("background", 97.0),
        ("surface", 97.0),
        ("surfaceBright", 98.0),
        ("surfaceContainerLowest", 99.0),
        ("surfaceContainerLow", 95.0),
        ("surfaceContainer", 93.0),
        ("surfaceContainerHigh", 91.0),
        ("surfaceContainerHighest", 89.0),
        ("surfaceVariant", 89.0),
        ("surfaceDim", 86.0),
        ("inverseOnSurface", 95.0),
    ] {
        set(name, surface(tone));
    }
    let ink = surface_chroma.min(8.0);
    set("onBackground", tone_of(p.surface, ink, 10.0));
    set("onSurface", tone_of(p.surface, ink, 10.0));
    set("onSurfaceVariant", tone_of(p.surface, ink + 4.0, 30.0));
    set("outline", tone_of(p.surface, ink + 4.0, 50.0));
    set("outlineVariant", tone_of(p.surface, ink + 4.0, 80.0));
    set("inverseSurface", tone_of(p.surface, ink, 20.0));

    set("primary", tone_of(p.accent, accent_chroma, 40.0));
    set("onPrimary", 0xFFFFFFFF);
    set("surfaceTint", tone_of(p.accent, accent_chroma, 40.0));
    set("inversePrimary", tone_of(p.accent, accent_chroma, 80.0));
    for (name, color) in containers {
        let c = chroma(color).max(16.0);
        set(role(name, ""), tone_of(color, c.max(32.0), 40.0));
        set(role(name, "on"), 0xFFFFFFFF);
        set(role(name, "container"), color);
        set(role(name, "onContainer"), tone_of(color, c.max(24.0), 10.0));
    }
    scheme
}

/// "primary" + "" → "primary", "on" → "onPrimary", "container" → "primaryContainer", "onContainer" → ….
fn role(base: &str, kind: &str) -> &'static str {
    match (base, kind) {
        ("primary", "") => "primary",
        ("primary", "on") => "onPrimary",
        ("primary", "container") => "primaryContainer",
        ("primary", "onContainer") => "onPrimaryContainer",
        ("secondary", "") => "secondary",
        ("secondary", "on") => "onSecondary",
        ("secondary", "container") => "secondaryContainer",
        ("secondary", "onContainer") => "onSecondaryContainer",
        ("tertiary", "") => "tertiary",
        ("tertiary", "on") => "onTertiary",
        ("tertiary", "container") => "tertiaryContainer",
        ("tertiary", "onContainer") => "onTertiaryContainer",
        _ => "shadow",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contrast_ratio;

    #[test]
    fn palettes_resolve_by_stored_value() {
        assert_eq!(palette("palette:softPink").map(|p| p.name), Some("Soft Pink Delight"));
        assert_eq!(palette("sageSand").map(|p| p.id), Some("sageSand"));
        assert!(palette("#8C6DFF").is_none());
        let mut ids: Vec<_> = PALETTES.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PALETTES.len());
    }

    #[test]
    fn light_palettes_use_their_own_colours_and_stay_readable() {
        for p in &PALETTES {
            for dark in [false, true] {
                let s = palette_scheme(p, dark, Variant::TonalSpot, 0.0);
                if !dark {
                    assert_eq!(s.colors["primaryContainer"], p.primary_container, "{}", p.id);
                    assert_eq!(s.colors["secondaryContainer"], p.secondary_container, "{}", p.id);
                }
                for (bg, fg) in [
                    ("surface", "onSurface"),
                    ("surfaceContainerHighest", "onSurface"),
                    ("primary", "onPrimary"),
                    ("primaryContainer", "onPrimaryContainer"),
                    ("secondaryContainer", "onSecondaryContainer"),
                    ("tertiaryContainer", "onTertiaryContainer"),
                ] {
                    let ratio = contrast_ratio(s.colors[bg], s.colors[fg]);
                    assert!(ratio >= 4.5, "{} dark={dark}: {fg} on {bg} is {ratio:.2}", p.id);
                }
            }
        }
    }
}
