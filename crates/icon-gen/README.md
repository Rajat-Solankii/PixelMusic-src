# Icon Generator

A Rust utility to convert SVG logos to PNG and Windows ICO files.

## Overview

This crate converts the Materialfy SVG logo colorways into:
- PNG files at 7 different sizes: 16x16, 24x24, 32x32, 48x48, 64x64, 128x128, 256x256
- Windows ICO files containing all PNG sizes

## Usage

```bash
cargo run --release --package icon-gen
```

## Input

Reads SVG files from: `src/materialfy/assets/logos/`
- blue-purple.svg (default)
- baby-pink-purple.svg
- graphite-gray.svg
- lime-green.svg
- lime-lemon.svg
- orange-yellow.svg

## Output

Generates files in: `src/materialfy/assets/icons/`

For each colorway:
- `{colorway}.ico` - Windows icon file
- `{colorway}-16.png` through `{colorway}-256.png` - Individual PNG sizes

Total: 48 files (6 colorways × 8 files each)

## Dependencies

- `resvg` - SVG parsing and rendering
- `tiny-skia` - 2D rendering (via resvg)
- `usvg` - SVG simplification (via resvg)
- `ico` - ICO file format encoding
- `png` - PNG encoding (via tiny-skia)

## Technical Details

- Renders SVGs at exact pixel dimensions
- Maintains aspect ratio and scales to fit
- Uses resvg's high-quality rendering engine
- Creates multi-resolution ICO files compatible with Windows

## Regeneration

Run this tool whenever SVG logos are updated to regenerate all PNG and ICO assets.
