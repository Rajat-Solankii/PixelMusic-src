//! Icon generator for Materialfy SVG logos
//! Converts SVG logos to PNG files at multiple sizes and creates Windows ICO files

use anyhow::{Context, Result};
use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use resvg::usvg::{Options, Tree};
use std::fs;
use std::path::{Path, PathBuf};

const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
const COLORWAYS: [&str; 6] = [
    "blue-purple",
    "baby-pink-purple",
    "graphite-gray",
    "lime-green",
    "lime-lemon",
    "orange-yellow",
];

fn main() -> Result<()> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .context("Failed to find project root")?
        .to_path_buf();

    let logos_dir = project_root.join("src/materialfy/assets/logos");
    let icons_dir = project_root.join("src/materialfy/assets/icons");

    fs::create_dir_all(&icons_dir)
        .with_context(|| format!("Failed to create icons dir: {:?}", icons_dir))?;

    for colorway in &COLORWAYS {
        println!("Processing {}...", colorway);
        generate_icons_for_colorway(&logos_dir, &icons_dir, colorway)?;
    }

    println!("Done! Icons generated in {:?}", icons_dir);
    Ok(())
}

fn generate_icons_for_colorway(logos_dir: &Path, icons_dir: &Path, colorway: &str) -> Result<()> {
    let svg_path = logos_dir.join(format!("{}.svg", colorway));
    println!("  Reading {:?}", svg_path);

    // Read SVG content
    let svg_data = fs::read(&svg_path)
        .with_context(|| format!("Failed to read SVG: {:?}", svg_path))?;

    // Parse SVG tree
    let opt = Options::default();
    let tree = Tree::from_data(&svg_data, &opt)
        .with_context(|| format!("Failed to parse SVG: {:?}", svg_path))?;

    // Generate PNG at each size
    let mut png_paths: Vec<PathBuf> = Vec::new();
    for size in SIZES {
        let png_path = icons_dir.join(format!("{}-{}.png", colorway, size));
        println!("    Generating {}x{} PNG", size, size);
        render_png(&tree, &png_path, size)?;
        png_paths.push(png_path.clone());
    }

    // Create ICO file
    let ico_path = icons_dir.join(format!("{}.ico", colorway));
    println!("    Creating ICO file");
    create_ico(&png_paths, &ico_path)?;

    Ok(())
}

fn render_png(tree: &Tree, output_path: &Path, size: u32) -> Result<()> {
    // Get original dimensions
    let size_f = size as f32;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).context("Failed to create pixmap")?;

    // Transform to fit the target size
    let svg_size = tree.size();
    let scale_x = size_f / svg_size.width();
    let scale_y = size_f / svg_size.height();
    let scale = scale_x.min(scale_y);

    // Create transform
    let transform = tiny_skia::Transform::from_scale(scale, scale);

    // Render to pixmap using resvg
    resvg::render(tree, transform, &mut pixmap.as_mut());

    // Save as PNG
    pixmap.save_png(output_path)
        .with_context(|| format!("Failed to save PNG: {:?}", output_path))?;

    Ok(())
}

fn create_ico(png_paths: &[PathBuf], ico_path: &Path) -> Result<()> {
    let mut entries: Vec<IconDirEntry> = Vec::new();

    for png_path in png_paths {
        // Read PNG file
        let png_reader = std::io::BufReader::new(
            fs::File::open(png_path)
                .with_context(|| format!("Failed to open PNG: {:?}", png_path))?
        );
        
        let image = IconImage::read_png(png_reader)
            .with_context(|| format!("Failed to read PNG: {:?}", png_path))?;

        let entry = IconDirEntry::encode(&image)
            .with_context(|| format!("Failed to encode IconDirEntry for {:?}", png_path))?;

        entries.push(entry);
    }

    // Create ICO directory
    let mut ico_file = IconDir::new(ResourceType::Icon);
    for entry in entries {
        ico_file.add_entry(entry);
    }

    let mut buffer = Vec::new();
    ico_file.write(&mut buffer)
        .context("Failed to write ICO to buffer")?;

    fs::write(ico_path, buffer)
        .with_context(|| format!("Failed to write ICO: {:?}", ico_path,))?;

    Ok(())
}
