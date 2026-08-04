from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


HERE = Path(__file__).resolve().parent
CAPTURES = [
    ("1  WELCOME", "welcome-empty-80x24.txt"),
    ("2  EDITING", "welcome-editing-80x24.txt"),
    ("3  FIRST SUBMIT", "active-conversation-80x24.txt"),
]
FONT_CANDIDATES = [
    Path(r"C:\Windows\Fonts\CascadiaMono.ttf"),
    Path("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"),
]
FONT_PATH = next(path for path in FONT_CANDIDATES if path.exists())
FONT = ImageFont.truetype(str(FONT_PATH), 16)
LABEL_FONT = ImageFont.truetype(str(FONT_PATH), 15)
CELL_WIDTH = FONT.getlength("M")
LINE_HEIGHT = 22
PADDING_X = 24
PADDING_Y = 18
LABEL_HEIGHT = 38
PANEL_GAP = 18
COLUMNS = 80
ROWS = 24

CANVAS = "#05090d"
PANEL = "#080d12"
PRIMARY = "#d9e2ec"
MUTED = "#8b98a9"
BRAND = "#00e5b7"
AMBER = "#f2c94c"
STATUS_BACKGROUND = "#0a1017"
COMPOSER_BACKGROUND = "#111b26"
GUIDE = "#244d49"


def frame_lines(filename: str) -> list[str]:
    lines = (HERE / filename).read_text(encoding="utf-8").splitlines()
    return (lines + [""] * ROWS)[:ROWS]


panel_width = round(PADDING_X * 2 + COLUMNS * CELL_WIDTH)
panel_height = LABEL_HEIGHT + PADDING_Y * 2 + ROWS * LINE_HEIGHT
canvas_width = panel_width + PADDING_X * 2
canvas_height = PADDING_Y * 2 + len(CAPTURES) * panel_height + (len(CAPTURES) - 1) * PANEL_GAP
image = Image.new("RGB", (canvas_width, canvas_height), CANVAS)
draw = ImageDraw.Draw(image)

for panel_index, (label, filename) in enumerate(CAPTURES):
    panel_x = PADDING_X
    panel_y = PADDING_Y + panel_index * (panel_height + PANEL_GAP)
    text_x = panel_x + PADDING_X
    text_y = panel_y + LABEL_HEIGHT + PADDING_Y
    lines = frame_lines(filename)
    composer_start = next(index for index, line in enumerate(lines) if line.startswith("╭ Task"))
    status_start = next(index for index, line in enumerate(lines) if line.startswith("Akra /"))

    draw.rounded_rectangle(
        (panel_x, panel_y, panel_x + panel_width, panel_y + panel_height),
        radius=10,
        fill=PANEL,
        outline="#1b2631",
        width=1,
    )
    draw.text((text_x, panel_y + 10), f"{label}   ·   composer bottom row 23", font=LABEL_FONT, fill=BRAND)
    draw.rectangle(
        (
            panel_x + 1,
            text_y + status_start * LINE_HEIGHT - 2,
            panel_x + panel_width - 1,
            text_y + composer_start * LINE_HEIGHT - 2,
        ),
        fill=STATUS_BACKGROUND,
    )
    draw.rectangle(
        (
            panel_x + 1,
            text_y + composer_start * LINE_HEIGHT - 2,
            panel_x + panel_width - 1,
            text_y + ROWS * LINE_HEIGHT,
        ),
        fill=COMPOSER_BACKGROUND,
    )
    guide_y = text_y + ROWS * LINE_HEIGHT
    draw.line((panel_x + 1, guide_y, panel_x + panel_width - 1, guide_y), fill=GUIDE, width=2)

    for row, line in enumerate(lines):
        color = PRIMARY
        if row < 6 or line.startswith("Akra /") or line.startswith(("╭", "│", "╰")):
            color = BRAND
        elif line.startswith(("status:", "startup:")):
            color = MUTED
        elif "CHECKING" in line or "PENDING" in line:
            color = AMBER
        draw.text((text_x, text_y + row * LINE_HEIGHT), line, font=FONT, fill=color)

image.save(HERE / "stable-composer-three-state-80x24.png")
