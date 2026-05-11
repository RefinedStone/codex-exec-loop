# Gamebaljeonguk Sprite Pack

Source: processed from the provided image only. No character redraw/regeneration was applied.

## Files
- gamebaljeonguk_original_transparent.png
  - Original layout with white background removed.
- gamebaljeonguk_atlas_128x192.png
  - Engine atlas for Godot/Unity slicing.
  - Cell size: 128x192
  - Columns: 8
  - Rows: 7
  - Transparent PNG.
- gamebaljeonguk_atlas_64x96.png
  - Half-size atlas using nearest-neighbor scaling.
  - Cell size: 64x96.
- $gamebaljeonguk_planner.png
  - RPG Maker single-character sheet.
  - Cell size: 128x192; 3 columns x 4 rows.
  - Right-facing row is mirrored from the side row because the source image did not provide a separate right-facing row.
- $gamebaljeonguk_coffee_addict.png
  - RPG Maker single-character sheet.
  - Same format as above.
- gamebaljeonguk_sprite_metadata.json
  - Frame coordinates and source crop data.
- preview_*.png
  - Checkerboard previews for checking transparency.

## Unity
Import the atlas as Sprite Mode: Multiple.
Use Grid By Cell Size:
- 128x192 for gamebaljeonguk_atlas_128x192.png
- 64x96 for gamebaljeonguk_atlas_64x96.png
Set Filter Mode to Point if you want hard pixel edges.

## Godot
Import the atlas as Texture2D/SpriteFrames.
Slice by:
- H: 128, V: 192 for the large atlas.
Disable filtering for sharper pixel art.

## RPG Maker MV/MZ
Use the files beginning with "$" as single-character sheets.
The source only had complete down/side/back rows for the first two characters, so only those two were converted to complete RPG Maker-style sheets.
