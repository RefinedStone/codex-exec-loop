# PR approver sprite provenance

- Generated: 2026-08-11
- Generator: OpenAI ImageGen through the Codex image generation tool
- Intended use: production pixel-art animation atlas for the AKRA isometric admin dashboard
- Archival inputs: `pr-approver-chroma-source.png` and `pr-approver-alpha-source.png`
- Deterministic derivative pipeline: `assets/admin/game/scripts/prepare-pr-approver-sprites.mjs`

The received square source did not provide equal-height encoded grid cells, so the preparation pipeline records the four visual row bounds and six exact 209-pixel columns in metadata. It then normalizes each frame around an alpha-weighted lower-body anchor, a shared body center, and a shared foot baseline. The external hand in intake row 0 column 1 is preserved only as an archival/reserved frame and is excluded from runtime clips.

The prompt requested exact `#ff00ff`. The received chroma image is not mathematically uniform; the validator therefore uses the sampled exterior reference `[241, 8, 226]` only as a fringe detector. Runtime transparency comes from the archived alpha source, and every fully transparent output pixel is forced to RGB zero.

## Complete production prompt

```text
Use case: production pixel-art game sprite atlas for the AKRA isometric admin dashboard.

Create a clean EXACT 6 columns x 4 rows sprite sheet, 24 equal cells, showing ONE AND THE SAME strict pull-request approval officer in every cell. Preserve the character identity from the first reference exactly: adult woman, dark auburn hair in a low bun, rectangular black glasses, navy blazer, cream blouse, ID lanyard, charcoal knee skirt, burgundy flats, white review documents. Match the second reference's polished RPG pixel-art rendering, front/down three-quarter camera, pixel density, crisp dark outlines, scale, lighting, and proportions.

Critical geometry: full body visible in every cell; identical head size, body height, foot baseline, center alignment, camera angle, and visual scale in all 24 cells. Feet must land at exactly the same vertical baseline. Never zoom, crop, rotate, change pose scale, or switch character. Only arms, paper, eyes, mouth, and tiny status accents move. No whole-body crossfade concept.

Animation layout, left to right:
ROW 1 INTAKE: (1) neutral holding closed folder, (2) document arrives from left, (3) catches document, (4) opens folder, (5) adjusts glasses and first glance, (6) settles into reading.
ROW 2 REVIEW LOOP: (1) scans upper page, (2) scans lower page, (3) pinches page corner, (4) lifts page halfway, (5) page crosses center in mid-flip, (6) turned page settles and she resumes reading. This six-frame row must loop naturally from frame 6 back to frame 1 without making her walk or resize.
ROW 3 FAILURE ONE-SHOT: (1) detects a problem, (2) stern glare over glasses, (3) marks document with red pen, (4) small angry head shake with subtle red anger accent, (5) firmly presents rejected document with a small red X seal/icon and no words, (6) stable angry rejection hold.
ROW 4 SUCCESS ONE-SHOT: (1) detects all checks passed, (2) adjusts glasses, (3) applies small green check approval seal/icon and no words, (4) closes approved folder, (5) restrained proud nod with one tiny green sparkle, (6) stable composed proud approval hold.

Background: perfectly uniform flat chroma key EXACT #ff00ff across every cell. No floor, no cast shadow, no gradient, no texture, no border, no captions, no labels, no letters, no UI, no cell dividers. No white halo or light-colored outline around the silhouette. White paper must have a crisp dark 2-pixel-equivalent outline and must not merge into the background. Keep at least 3% empty chroma margin around every cell silhouette. Output one square atlas only.
```
