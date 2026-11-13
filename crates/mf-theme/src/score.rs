//! Score.score из materialyoucolor (score/score.py) — ранжирование цветов квантайзера.
//! Своя копия, а не material_colors::score: в Python сумма популяций начинается с 1e-5,
//! а оттенок округляется по-банковски (round()), и порядок при равных баллах — как у устойчивой сортировки.
use material_colors::color::Rgb;
use material_colors::hct::Hct;

const TARGET_CHROMA: f64 = 48.0;
const WEIGHT_PROPORTION: f64 = 0.7;
const WEIGHT_CHROMA_ABOVE: f64 = 0.3;
const WEIGHT_CHROMA_BELOW: f64 = 0.1;
const CUTOFF_CHROMA: f64 = 5.0;
const CUTOFF_EXCITED_PROPORTION: f64 = 0.01;

pub const DESIRED: usize = 4;
pub const FALLBACK: u32 = 0xFF4285F4;

fn sanitize_degrees_int(degrees: i64) -> usize {
    degrees.rem_euclid(360) as usize
}

fn difference_degrees(a: f64, b: f64) -> f64 {
    180.0 - ((a - b).abs() - 180.0).abs()
}

/// `colors` — пары (ARGB, популяция) в порядке выдачи квантайзера. Возвращает ARGB, лучший первым.
pub fn score(colors: &[(u32, u32)]) -> Vec<u32> {
    let mut hcts = Vec::with_capacity(colors.len());
    let mut hue_population = [0u64; 360];
    let mut population_sum = 1e-5;

    for &(argb, population) in colors {
        let hct = Hct::new(Rgb::from_u32(argb));
        let hue = hct.get_hue() as usize % 360;
        hue_population[hue] += u64::from(population);
        population_sum += f64::from(population);
        hcts.push(hct);
    }

    let mut excited = [0.0f64; 360];
    for hue in 0..360i64 {
        let proportion = hue_population[hue as usize] as f64 / population_sum;
        for i in (hue - 14)..(hue + 16) {
            excited[sanitize_degrees_int(i)] += proportion;
        }
    }

    let mut scored: Vec<(Hct, f64)> = Vec::new();
    for hct in hcts {
        let hue = sanitize_degrees_int(hct.get_hue().round_ties_even() as i64);
        let proportion = excited[hue];
        let chroma = hct.get_chroma();
        if chroma < CUTOFF_CHROMA || proportion <= CUTOFF_EXCITED_PROPORTION {
            continue;
        }
        let proportion_score = proportion * 100.0 * WEIGHT_PROPORTION;
        let weight = if chroma < TARGET_CHROMA {
            WEIGHT_CHROMA_BELOW
        } else {
            WEIGHT_CHROMA_ABOVE
        };
        scored.push((hct, proportion_score + (chroma - TARGET_CHROMA) * weight));
    }
    // sort(reverse=True) в Python устойчив: равные сохраняют исходный порядок
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    pick_distinct(&scored)
}

/// The best scored colours with hues far enough apart (Score.score's last step): the hue gap shrinks from 90° to
/// 15° until DESIRED colours are found; nothing scored → FALLBACK.
fn pick_distinct(scored: &[(Hct, f64)]) -> Vec<u32> {
    let mut chosen: Vec<Hct> = Vec::new();
    for min_difference in (15..=90).rev() {
        chosen.clear();
        for (hct, _) in scored {
            let duplicate = chosen.iter().any(|c| {
                difference_degrees(hct.get_hue(), c.get_hue()) < f64::from(min_difference)
            });
            if !duplicate {
                chosen.push(*hct);
            }
            if chosen.len() >= DESIRED {
                break;
            }
        }
        if chosen.len() >= DESIRED {
            break;
        }
    }

    if chosen.is_empty() {
        return vec![FALLBACK];
    }
    chosen
        .into_iter()
        .map(|h| Rgb::from(h).as_argb_u32())
        .collect()
}

// "Accurate" artwork scoring, in the spirit of PixelPlayer's ColorRoles: the stock score above lets a small, very
// saturated accent win over the colour the artwork is actually made of. Here a candidate scores for the share of the
// picture its hue family covers (population) and for how close it is to the artwork's representative colour
// (fidelity: the population-weighted mean of the colourful clusters around the most common hue), with only a small
// bonus for chroma.
const ACCURATE_WEIGHT_POPULATION: f64 = 0.5;
const ACCURATE_WEIGHT_FIDELITY: f64 = 30.0;
/// Distance (in CAM16 a/b + tone units) at which fidelity has fallen to 1/e.
const ACCURATE_FIDELITY_FALLOFF: f64 = 20.0;
/// Half-width of the hue family the representative colour is averaged over.
const ACCURATE_FAMILY_DEGREES: f64 = 30.0;

/// A colour as a point in a perceptual space: CAM16 hue/chroma as a/b, HCT tone as lightness.
fn point(hct: &Hct) -> [f64; 3] {
    let (sin, cos) = hct.get_hue().to_radians().sin_cos();
    [hct.get_chroma() * cos, hct.get_chroma() * sin, hct.get_tone()]
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The colour the artwork reads as: the population-weighted mean of the colourful clusters whose hue lies within
/// ACCURATE_FAMILY_DEGREES of the most populated hue (`excited`, smoothed over ±15°). None without colourful
/// clusters.
fn representative(hcts: &[(Hct, u32)], excited: &[f64; 360]) -> Option<[f64; 3]> {
    let colourful = || hcts.iter().filter(|(hct, _)| hct.get_chroma() >= CUTOFF_CHROMA);
    let peak = colourful()
        .map(|(hct, _)| hct.get_hue())
        .max_by(|a, b| excited[*a as usize % 360].total_cmp(&excited[*b as usize % 360]))?;
    let (mut sum, mut weight) = ([0.0f64; 3], 0.0f64);
    let family = colourful().filter(|(hct, _)| difference_degrees(hct.get_hue(), peak) <= ACCURATE_FAMILY_DEGREES);
    for (hct, population) in family {
        let p = point(hct);
        let w = f64::from(*population);
        for i in 0..3 {
            sum[i] += p[i] * w;
        }
        weight += w;
    }
    (weight > 0.0).then(|| sum.map(|v| v / weight))
}

/// Like `score`, but ranks by hue population plus fidelity to the artwork's representative colour (see above).
/// Same input, output, cutoffs and hue spacing as `score`.
pub fn score_accurate(colors: &[(u32, u32)]) -> Vec<u32> {
    let mut hcts = Vec::with_capacity(colors.len());
    let mut hue_population = [0u64; 360];
    let mut population_sum = 1e-5;
    for &(argb, population) in colors {
        let hct = Hct::new(Rgb::from_u32(argb));
        // only colourful clusters count towards a hue's share: greys have no real hue
        if hct.get_chroma() >= CUTOFF_CHROMA {
            hue_population[hct.get_hue() as usize % 360] += u64::from(population);
        }
        population_sum += f64::from(population);
        hcts.push((hct, population));
    }
    let mut excited = [0.0f64; 360];
    for hue in 0..360i64 {
        let proportion = hue_population[hue as usize] as f64 / population_sum;
        for i in (hue - 14)..(hue + 16) {
            excited[sanitize_degrees_int(i)] += proportion;
        }
    }
    let Some(target) = representative(&hcts, &excited) else {
        return vec![FALLBACK];
    };

    let mut scored: Vec<(Hct, f64)> = Vec::new();
    for (hct, _) in &hcts {
        let proportion = excited[sanitize_degrees_int(hct.get_hue().round_ties_even() as i64)];
        let chroma = hct.get_chroma();
        if chroma < CUTOFF_CHROMA || proportion <= CUTOFF_EXCITED_PROPORTION {
            continue;
        }
        let fidelity = (-distance(point(hct), target) / ACCURATE_FIDELITY_FALLOFF).exp();
        let chroma_weight = if chroma < TARGET_CHROMA { WEIGHT_CHROMA_BELOW } else { WEIGHT_CHROMA_ABOVE };
        let total = proportion * 100.0 * ACCURATE_WEIGHT_POPULATION
            + fidelity * ACCURATE_WEIGHT_FIDELITY
            + (chroma - TARGET_CHROMA) * chroma_weight;
        scored.push((*hct, total));
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    pick_distinct(&scored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argb(hue: f64, chroma: f64, tone: f64) -> u32 {
        Rgb::from(Hct::from(hue, chroma, tone)).as_argb_u32()
    }

    fn hue_of(argb: u32) -> f64 {
        Hct::new(Rgb::from_u32(argb)).get_hue()
    }

    #[test]
    fn a_small_neon_accent_does_not_beat_the_colour_the_artwork_is_made_of() {
        // under half the cover a muted teal, a quarter a neon magenta, the rest grey
        let teal = argb(190.0, 10.0, 45.0);
        let magenta = 0xFFFF00C8;
        let colors = [(teal, 450), (magenta, 250), (argb(0.0, 0.0, 70.0), 300)];
        // the stock score takes the accent…
        assert_eq!(score(&colors)[0], magenta);
        // …the accurate one the teal, and still offers the accent second
        let ranked = score_accurate(&colors);
        assert_eq!(ranked[0], teal);
        assert_eq!(ranked[1], magenta);
    }

    #[test]
    fn within_one_hue_the_seed_is_the_shade_the_artwork_shows_not_its_brightest_pixels() {
        // a dark navy cover with a few bright blue highlights: same hue family
        let navy = argb(265.0, 30.0, 20.0);
        let navy_light = argb(262.0, 34.0, 28.0);
        let bright = argb(268.0, 80.0, 60.0);
        let colors = [(navy, 600), (navy_light, 300), (bright, 40)];
        assert_eq!(score(&colors)[0], bright);
        let first = score_accurate(&colors)[0];
        assert!(first == navy || first == navy_light, "{first:08X}");
    }

    #[test]
    fn the_most_common_hue_family_wins_and_other_hues_follow_spaced_apart() {
        let orange = argb(60.0, 60.0, 65.0);
        let orange2 = argb(66.0, 55.0, 60.0);
        let green = argb(140.0, 50.0, 50.0);
        let blue = argb(250.0, 50.0, 40.0);
        let ranked = score_accurate(&[(green, 200), (orange, 400), (blue, 150), (orange2, 250)]);
        assert!(difference_degrees(hue_of(ranked[0]), 63.0) < 10.0, "{:08X}", ranked[0]);
        // the second orange is too close in hue to be offered as well
        assert!(!(ranked.contains(&orange2) && ranked.contains(&orange)));
        assert!(ranked.contains(&green) && ranked.contains(&blue));
    }

    #[test]
    fn greys_only_fall_back_like_the_stock_score() {
        let colors = [(argb(0.0, 0.0, 10.0), 300), (argb(0.0, 0.0, 80.0), 700)];
        assert_eq!(score_accurate(&colors), vec![FALLBACK]);
        assert_eq!(score_accurate(&[]), vec![FALLBACK]);
    }
}
