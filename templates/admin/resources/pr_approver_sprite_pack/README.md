# PR approver sprite pack

This pack is the archival source and deterministic build contract for the bespectacled PR approval officer used by the AKRA admin diorama.

## Runtime layout

The runtime atlas is a strict 6 x 4 grid. Every large cell is 128 x 192 pixels; the half atlas uses 64 x 96 cells.

| Row | State | Playback | Frames |
| --- | --- | --- | --- |
| 0 | intake | one shot | neutral, reserved document-arrival frame, receive, open, first glance, settle |
| 1 | review | loop | scan upper/lower, pinch, lift, mid-flip, settle |
| 2 | failure | one shot then hold | detect, glare, red mark, angry shake, reject, angry hold |
| 3 | success | one shot then hold | detect, glasses, green check, close, proud nod, proud hold |

Row 0 column 1 is intentionally marked `reserved` in metadata. The generated source contains an external floating hand, so runtime animation clips must not select that frame.

All frames use a common body center and foot baseline. The outer two pixels of every 128 x 192 cell are transparent. The 64 x 96 atlas is not independently resized: every pixel is the exact top-left nearest sample from the corresponding 2 x 2 block in the large atlas.

## Files

- `pr-approver-chroma-source.png`: received ImageGen chroma source.
- `pr-approver-alpha-source.png`: archival alpha-extracted source used by the prepare script.
- `pr-approver-sprite-metadata.json`: source segmentation, normalization, frame map, and validation contract.
- `pr-approver-atlas-128x192.png`: archival copy of the large runtime atlas.
- `pr-approver-atlas-64x96.png`: archival copy of the exact-nearest half atlas.
- `preview-pr-approver-dark.png`: opaque preview over the admin dark background.
- `PROVENANCE.md`: generation provenance and the complete production prompt.

Runtime copies live under `assets/admin/graphics/` and are embedded by the admin API. Never hand-edit either runtime or archival atlas.

## Rebuild and verify

From the repository root:

```text
npm --prefix assets/admin/game run sprites:approver:prepare
npm --prefix assets/admin/game run sprites:approver:check
npm --prefix assets/admin/game run sprites:check
```

The validator rejects non-zero RGB in transparent pixels, chroma fringe near the exterior silhouette, empty cells, occupied frame borders, archive/runtime drift, preview drift, and any half-atlas pixel that is not the declared exact nearest sample.
