# Image Generation Prompt Set

모든 이미지는 built-in `image_gen`의 `precise-object-edit` 모드로 생성했습니다. CLI/API fallback은 사용하지 않았습니다.

아래 프롬프트의 좌석·라운지 수량은 생성 목표입니다. 모델 출력에는 가구 수량 변동이 있으므로 결과 PNG를 정확한 런타임 fixture로 간주하지 않습니다. 갤러리의 통제된 5명 비교는 기존 character atlas를 HTML로 별도 합성하며, 최종 선택안은 결정적 좌표와 정확한 수량으로 다시 제작합니다.

## 모든 안에 공통으로 적용한 불변 조건

```text
Use case: precise-object-edit
Asset type: 16:9 desktop admin game environment concept
Input images: Image 1 is the edit target and immutable style/perspective reference
Style/medium: polished high-end pixel-art game environment, crisp isometric geometry, production-ready concept art, coherent sprite scale
Composition/framing: full 16:9 room overview; generous collision-free walking space
Lighting/mood: focused late-night operations, warm amber task lights balanced with cool cyan systems light
Color palette: preserve Image 1 deep navy, charcoal, dark walnut, restrained cyan, amber, and green accents
Materials/textures: pixel-art wood, dark stone, brushed metal, glass displays, subtle floor inlays marking zones
Constraints: change only the interior floor plan and furnishings; preserve the outer walls, camera angle, isometric projection, pixel-art language, palette, and room proportions from Image 1; no people or characters; no text, numbers, labels, logos, watermarks, UI panels, floating HUD, or illegible glyphs; keep walking lanes readable for later HTML sprite overlays; all furniture aligned to one coherent isometric grid
Avoid: clutter, scattered desks, random props, oversized furniture, blocked paths, photorealism, painterly blur, neon overload
```

## A · Command Spine

```text
Primary request: reorganize the empty interior into a premium, highly legible "Command Spine" operations room for AKRA agents
Scene/backdrop: retain the exact outer room architecture and isometric cutaway camera; add a disciplined central command spine with a broad uninterrupted aisle, five compact agent workstations arranged as two mirrored pairs plus one lead station, one clearly separated event-log tower on the right, and a calm three-seat standby lounge along the lower edge
Composition emphasis: strongest hierarchy at center; obvious walking lanes; generous collision-free space around every workstation
```

## B · Orbit Pods

```text
Primary request: reorganize the empty interior into a premium, highly legible "Orbit Pods" operations room for AKRA agents
Scene/backdrop: retain the exact outer room architecture and isometric cutaway camera; build one compact central mission core with five individual agent desks forming a balanced open ring around it, a continuous wide circulation loop between core and desks, one event-log tower anchored on the right, and a quiet three-seat standby lounge along the lower edge
Composition emphasis: radial hierarchy with strong center; every desk has a clear inward sightline and an outward escape path; generous collision-free walking ring
```

## C · Flow Line

```text
Primary request: reorganize the empty interior into a premium, highly legible "Flow Line" operations room that makes AKRA task progress readable from left to right
Scene/backdrop: retain the exact outer room architecture and isometric cutaway camera; arrange five compact agent workstations in a clean stepped production line from upper-left planning through center execution to lower-right review and delivery, with broad parallel walking lanes, a terminal event-log tower at the far right, a lead station above the flow, and a separate three-seat standby lounge along the lower-left edge
Composition emphasis: unmistakable directional sequence without arrows or text; strong negative space between workflow stages; no crossing routes
```

## D · District Grid

```text
Primary request: reorganize the empty interior into a premium, highly legible "District Grid" operations room for AKRA agents
Scene/backdrop: retain the exact outer room architecture and isometric cutaway camera; divide the main floor into four visually coherent operational districts around one broad cross-shaped avenue: planning at upper-left, execution with five compact desks split across upper-right and lower-left, review and event-log tower at lower-right, command desk centered at the top, and a calm three-seat standby lounge along the lower edge
Composition emphasis: strongest scanability at a glance; strict aligned blocks; broad horizontal and vertical circulation; each district has a distinct floor texture boundary while remaining one room
```

### D 비교 편향 교정

첫 출력에서 D만 좌상단 화이트보드가 고휘도 청색 지도 패널로 바뀌어, 나머지 안과 외벽의 시각 무게가 달라졌습니다. D의 공간 구조를 그대로 둔 채 원본 화이트보드를 복원하는 두 번째 `precise-object-edit`를 적용했습니다.

```text
Make exactly one controlled correction: replace the large bright blue map display mounted on the upper-left wall with the same quiet neutral off-white workflow whiteboard treatment seen in the source reference. Preserve the entire District Grid interior topology, all four floor districts, cross-shaped avenue, every workstation, furniture item, wall, window, lighting, camera, pixel scale, color palette, and image dimensions. No other visual changes.
```

## E · Review Amphitheater

```text
Primary request: reorganize the empty interior into a premium, highly legible "Review Amphitheater" operations room for AKRA agents
Scene/backdrop: retain the exact outer room architecture and isometric cutaway camera; arrange five compact agent workstations in a clean shallow horseshoe facing a central review stage and event-log tower, place one lead command station centered behind the horseshoe, keep two wide side aisles for movement, and create a calm three-seat standby lounge along the lower edge separated by a low visual boundary
Composition emphasis: clear shared focal point for review; tiered but non-overlapping sightlines; generous central presentation space and collision-free side aisles
```
