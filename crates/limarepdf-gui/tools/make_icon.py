# Makes the app icon (docs/images/icon.png and crates/limarepdf-gui/assets/icon.ico) from
# Phosphor's file-pdf glyph, the icon on the start page card, in the light theme's accent.
# Needs Pillow. Run from the repository root:
#   python3 crates/limarepdf-gui/tools/make_icon.py <path to egui-phosphor's res/Phosphor.ttf>
import sys
from PIL import Image, ImageDraw, ImageFont

ACCENT = (37, 99, 235, 255)  # theme.rs, light palette
GLYPH = ""  # egui_phosphor::regular::FILE_PDF
MASTER = 2048
FILL = 0.92  # share of the canvas the glyph's longer side takes
ICO_SIZES = [256, 64, 48, 40, 32, 24, 20, 16]


def fitted_font(path):
    font = ImageFont.truetype(path, MASTER)
    left, top, right, bottom = font.getbbox(GLYPH)
    scale = FILL * MASTER / max(right - left, bottom - top)
    return ImageFont.truetype(path, round(MASTER * scale))


def main():
    font = fitted_font(sys.argv[1])
    left, top, right, bottom = font.getbbox(GLYPH)
    at = ((MASTER - (right - left)) / 2 - left, (MASTER - (bottom - top)) / 2 - top)
    master = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
    ImageDraw.Draw(master).text(at, GLYPH, font=font, fill=ACCENT)
    icon = master.resize((256, 256), Image.LANCZOS)
    icon.save("docs/images/icon.png", optimize=True)
    icon.save("crates/limarepdf-gui/assets/icon.ico", sizes=[(s, s) for s in ICO_SIZES])


if __name__ == "__main__":
    main()
