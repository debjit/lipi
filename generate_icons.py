#!/usr/bin/env python3
"""
Generate Lipi application and system tray icons featuring the Bengali name 'লিপি'.
1. lipi_source.png (1024x1024) - high-res brand squircle with Bengali 'লিপি'
2. tray_idle.png (64x64 & 32x32) - clean crisp Bengali 'লিপি' for tray
3. tray_recording.png (64x64 & 32x32) - Bengali 'লিপি' with active red recording indicator
"""

from PIL import Image, ImageDraw, ImageFont

FONT_PATH = "/usr/share/fonts/truetype/noto/NotoSansBengali-Bold.ttf"

def create_brand_logo(size=1024):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))

    margin = int(size * 0.05)
    corner_radius = int(size * 0.22)
    box = [margin, margin, size - margin, size - margin]

    bg = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    bg_draw = ImageDraw.Draw(bg)

    # Subtle neutral dark gradient (#1e293b to #0f172a)
    for y in range(margin, size - margin):
        r = (y - margin) / (size - 2 * margin)
        cr = int(30 - 15 * r)
        cg = int(41 - 18 * r)
        cb = int(59 - 17 * r)
        bg_draw.line([(margin, y), (size - margin, y)], fill=(cr, cg, cb, 255))

    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(box, radius=corner_radius, fill=255)
    img.paste(bg, (0, 0), mask)

    # Inner border highlight
    border = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(border).rounded_rectangle(
        box, radius=corner_radius, outline=(255, 255, 255, 35), width=int(size * 0.008)
    )
    img.paste(border, (0, 0), border)

    # Render Bengali text 'লিপি'
    draw = ImageDraw.Draw(img)
    font_size = int(size * 0.33)
    font = ImageFont.truetype(FONT_PATH, font_size, layout_engine=ImageFont.Layout.RAQM)

    bbox = draw.textbbox((0, 0), "লিপি", font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    tx = (size - tw) // 2 - bbox[0]
    ty = (size - th) // 2 - bbox[1] - int(size * 0.015)

    # Soft text shadow
    shadow_offset = int(size * 0.012)
    draw.text((tx, ty + shadow_offset), "লিপি", font=font, fill=(0, 0, 0, 100))
    # Crisp white text
    draw.text((tx, ty), "লিপি", font=font, fill=(255, 255, 255, 255))

    return img

def create_tray_icon(size=64, recording=False):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Font size fitted for tray
    font_size = int(size * 0.36)
    font = ImageFont.truetype(FONT_PATH, font_size, layout_engine=ImageFont.Layout.RAQM)

    bbox = draw.textbbox((0, 0), "লিপি", font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    tx = (size - tw) // 2 - bbox[0]
    ty = (size - th) // 2 - bbox[1]

    # Dark stroke for contrast on both light and dark system trays
    outline_color = (15, 23, 42, 230)
    for dx in [-1, 0, 1]:
        for dy in [-1, 0, 1]:
            if dx != 0 or dy != 0:
                draw.text((tx + dx, ty + dy), "লিপি", font=font, fill=outline_color)

    # Crisp white text
    draw.text((tx, ty), "লিপি", font=font, fill=(255, 255, 255, 255))

    if recording:
        # Compact red recording badge at top-right
        badge_r = int(size * 0.15)
        bx = size - badge_r - 2
        by = badge_r + 2

        # Shadow ring
        draw.ellipse([bx - badge_r - 1, by - badge_r - 1, bx + badge_r + 1, by + badge_r + 1], fill=(15, 23, 42, 240))
        # Red circle
        draw.ellipse([bx - badge_r, by - badge_r, bx + badge_r, by + badge_r], fill=(239, 68, 68, 255))
        # Specular white highlight
        draw.ellipse([bx - badge_r // 3, by - badge_r // 3, bx + badge_r // 3, by + badge_r // 3], fill=(255, 255, 255, 240))

    return img

if __name__ == "__main__":
    logo = create_brand_logo(1024)
    logo.save("src-tauri/icons/lipi_source.png", "PNG")
    print("Saved lipi_source.png with Bengali name 'লিপি'")

    tray_idle = create_tray_icon(64, recording=False)
    tray_idle.save("src-tauri/icons/tray_idle.png", "PNG")
    tray_idle.resize((32, 32), Image.Resampling.LANCZOS).save("src-tauri/icons/tray_idle_32.png", "PNG")
    print("Saved tray_idle.png")

    tray_rec = create_tray_icon(64, recording=True)
    tray_rec.save("src-tauri/icons/tray_recording.png", "PNG")
    tray_rec.resize((32, 32), Image.Resampling.LANCZOS).save("src-tauri/icons/tray_recording_32.png", "PNG")
    print("Saved tray_recording.png")
