"""Build app/assets/icon.ico from app/assets/icon.png.

Requires Pillow. Run from the repository root:

    python scripts/make-icon.py
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "app" / "assets" / "icon.png"
TARGET = ROOT / "app" / "assets" / "icon.ico"
SIZES = [16, 24, 32, 48, 64, 128, 256]


def main():
    image = Image.open(SOURCE).convert("RGBA")
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    image.save(TARGET, format="ICO", sizes=[(size, size) for size in SIZES])
    print(f"wrote {TARGET} ({TARGET.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
