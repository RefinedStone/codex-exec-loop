from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


HERE = Path(__file__).resolve().parent
FONT = ImageFont.truetype(r"C:\Windows\Fonts\CascadiaMono.ttf", 18)
CELL_WIDTH = FONT.getlength("M")
LINE_HEIGHT = 25
PADDING_X = 28
PADDING_Y = 24
COLUMNS = 100
ROWS = 30

CANVAS = "#080d12"
PRIMARY = "#d9e2ec"
MUTED = "#8b98a9"
SUBTLE = "#657286"
BRAND = "#00e5b7"
ACCENT = "#5b8def"
MAGENTA = "#d783ff"
GREEN = "#70d68b"
RED = "#ff7d75"
STATUS_BACKGROUND = "#0a1017"
COMPOSER_BACKGROUND = "#111b26"


def x(column: int) -> float:
    return PADDING_X + column * CELL_WIDTH


def y(row: int) -> int:
    return PADDING_Y + row * LINE_HEIGHT


def draw_text(draw: ImageDraw.ImageDraw, row: int, column: int, text: str, fill: str) -> None:
    draw.text((x(column), y(row)), text, font=FONT, fill=fill)


def draw_substring(draw: ImageDraw.ImageDraw, lines: list[str], row: int, text: str, fill: str) -> None:
    draw_text(draw, row, lines[row].index(text), text, fill)


width = round(PADDING_X * 2 + COLUMNS * CELL_WIDTH)
height = PADDING_Y * 2 + ROWS * LINE_HEIGHT
image = Image.new("RGB", (width, height), CANVAS)
draw = ImageDraw.Draw(image)

draw.rectangle((0, y(24) - 2, width, y(27) - 2), fill=STATUS_BACKGROUND)
draw.rectangle((0, y(27) - 2, width, y(30)), fill=COMPOSER_BACKGROUND)

lines = (HERE / "frame-100x30.txt").read_text(encoding="utf-8").splitlines()
for row, line in enumerate(lines[:ROWS]):
    draw_text(draw, row, 0, line, PRIMARY)

for row, label in [(0, "You:"), (3, "Codex Commentary:"), (8, "Codex Commentary:")]:
    draw_text(draw, row, 0, label, BRAND)
draw_substring(draw, lines, 6, "› ◆ explore", MAGENTA)
draw_substring(draw, lines, 11, "▼ ◆ patch", MAGENTA)
draw_substring(draw, lines, 12, "[update]", ACCENT)
draw_text(draw, 13, 0, lines[13], MUTED)
draw_text(draw, 14, 0, lines[14], RED)
draw_text(draw, 15, 0, lines[15], GREEN)
draw_text(draw, 24, 0, "Akra", BRAND)
draw_substring(draw, lines, 24, "PENDING", "#f2c94c")
draw_text(draw, 25, 0, "startup:", MUTED)
draw_text(draw, 26, 0, "… CHECKING", "#f2c94c")
draw_text(draw, 27, 0, lines[27], BRAND)
draw_text(draw, 28, 0, "│", BRAND)
draw_text(draw, 28, 1, lines[28][1:-1], PRIMARY)
draw_text(draw, 28, 99, "│", BRAND)
draw_text(draw, 29, 0, "╰", BRAND)
draw_text(draw, 29, 1, lines[29][1:-1], MUTED)
draw_text(draw, 29, 99, "╯", BRAND)

image.save(HERE / "composer-surface-100x30.png")
