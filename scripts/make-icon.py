"""Munseo 앱 아이콘(1024px PNG)을 그린다.

    python3 -m venv .venv && .venv/bin/pip install pillow
    curl -fsSL -o NotoSansKR.ttf "https://github.com/google/fonts/raw/main/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf"
    .venv/bin/python scripts/make-icon.py NotoSansKR.ttf assets/icon/icon-1024.png

그다음 icon-256.png, munseo.ico(16~256px), munseo.icns(iconutil)를 이 PNG 에서 만든다.
"""
import sys
from PIL import Image, ImageDraw, ImageFont, ImageFilter

FONT, OUT = sys.argv[1], sys.argv[2]
N = 1024
SS = 2                      # supersample for smooth edges
W = N * SS

def rr_mask(size, box, radius):
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius=radius, fill=255)
    return m

img = Image.new("RGBA", (W, W), (0, 0, 0, 0))

# macOS icon grid: 824px body inside 1024 canvas, corner radius ~185
inset, body = 100 * SS, 824 * SS
box = (inset, inset, inset + body, inset + body)

# drop shadow
shadow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
shadow.putalpha(rr_mask((W, W), (box[0], box[1] + 14 * SS, box[2], box[3] + 14 * SS), 185 * SS).point(lambda a: a * 0.35))
img = Image.alpha_composite(img, shadow.filter(ImageFilter.GaussianBlur(18 * SS)))

# body: diagonal indigo -> violet gradient
grad = Image.new("RGBA", (W, W))
top, bottom = (67, 56, 202), (124, 58, 237)
gp = grad.load()
for y in range(W):
    for x in range(0, W, 1):
        t = min(1.0, max(0.0, (x * 0.35 + y) / (W * 1.35)))
        gp[x, y] = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,)
body_layer = Image.new("RGBA", (W, W), (0, 0, 0, 0))
body_layer.paste(grad, (0, 0), rr_mask((W, W), box, 185 * SS))
img = Image.alpha_composite(img, body_layer)

# page with folded corner
d = ImageDraw.Draw(img)
px0, py0, px1, py1 = 292 * SS, 218 * SS, 732 * SS, 806 * SS
fold = 120 * SS
page_shadow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
ImageDraw.Draw(page_shadow).polygon([(px0, py0 + 16*SS), (px1 - fold, py0 + 16*SS), (px1, py0 + fold + 16*SS), (px1, py1 + 16*SS), (px0, py1 + 16*SS)], fill=(20, 10, 60, 110))
img = Image.alpha_composite(img, page_shadow.filter(ImageFilter.GaussianBlur(16 * SS)))
d = ImageDraw.Draw(img)
d.rounded_rectangle((px0, py0, px1, py1), radius=26 * SS, fill=(255, 255, 255, 255))
# cut the corner and draw the fold
d.polygon([(px1 - fold, py0 - 2), (px1 + 2, py0 - 2), (px1 + 2, py0 + fold)], fill=(0, 0, 0, 0))
img_px = img.load()
# restore gradient where the cut removed page (corner triangle area shows body)
for y in range(py0 - 2, py0 + fold + 1):
    for x in range(px1 - fold, px1 + 3):
        if (x - (px1 - fold)) >= (y - (py0 - 2)) and img_px[x, y][3] == 0:
            img_px[x, y] = gp[x, y]
d.polygon([(px1 - fold, py0), (px1 - fold, py0 + fold), (px1, py0 + fold)], fill=(221, 214, 254, 255))

# 「문」 in bold indigo
font = ImageFont.truetype(FONT, 330 * SS)
font.set_variation_by_name("Bold")
cx, cy = (px0 + px1) // 2, (py0 + py1) // 2 - 40 * SS
d.text((cx, cy), "문", font=font, fill=(55, 48, 163, 255), anchor="mm")

# text lines under the glyph
for i, w in enumerate([300, 250]):
    y = cy + 205 * SS + i * 58 * SS
    d.rounded_rectangle((cx - w * SS // 2, y, cx + w * SS // 2, y + 22 * SS), radius=11 * SS, fill=(196, 181, 253, 255))

img.resize((N, N), Image.LANCZOS).save(OUT)
