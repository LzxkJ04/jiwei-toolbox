"""生成极维工具箱全套图标：圆角渐变底 + 白色工具箱。"""
import struct
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1] / "src-tauri" / "icons"
MASTER = 1024

# 与界面紫色主色一致
C0 = (167, 139, 250)
C1 = (109, 70, 229)


def lerp(a, b, t):
    return int(a + (b - a) * t)


def rounded_gradient(size, radius):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    px = img.load()
    for y in range(size):
        for x in range(size):
            t = (x * 0.35 + y * 0.75) / (size * 1.1)
            t = 0 if t < 0 else 1 if t > 1 else t
            px[x, y] = (
                lerp(C0[0], C1[0], t),
                lerp(C0[1], C1[1], t),
                lerp(C0[2], C1[2], t),
                255,
            )
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    img.putalpha(mask)
    return img


def star(draw, cx, cy, r, fill):
    pts = []
    for i in range(8):
        ang = -3.14159 / 2 + i * 3.14159 / 4
        rr = r if i % 2 == 0 else r * 0.38
        pts.append((cx + rr * __import__("math").cos(ang), cy + rr * __import__("math").sin(ang)))
    draw.polygon(pts, fill=fill)


def render_master():
    s = MASTER
    img = rounded_gradient(s, int(s * 0.22))
    d = ImageDraw.Draw(img)
    # 顶部高光
    gloss = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    gd = ImageDraw.Draw(gloss)
    gd.ellipse((int(s * 0.08), int(s * -0.18), int(s * 0.92), int(s * 0.42)), fill=(255, 255, 255, 46))
    mask = img.getchannel("A")
    gloss.putalpha(Image.composite(gloss.getchannel("A"), Image.new("L", (s, s), 0), mask))
    img = Image.alpha_composite(img, gloss)
    d = ImageDraw.Draw(img)

    white = (255, 255, 255, 255)
    # 箱体
    d.rounded_rectangle(
        (int(s * 0.20), int(s * 0.40), int(s * 0.80), int(s * 0.78)),
        radius=int(s * 0.07),
        fill=white,
    )
    # 提手（粗描边，16px 仍能看见）
    d.arc(
        (int(s * 0.34), int(s * 0.20), int(s * 0.66), int(s * 0.52)),
        start=200,
        end=340,
        fill=white,
        width=int(s * 0.065),
    )
    # 扣锁，用主色挖在白箱上
    d.rounded_rectangle(
        (int(s * 0.44), int(s * 0.50), int(s * 0.56), int(s * 0.66)),
        radius=int(s * 0.025),
        fill=C1 + (255,),
    )
    # 右上角星标
    star(d, int(s * 0.74), int(s * 0.30), int(s * 0.055), white)
    return img


def down(master, size):
    return master.resize((size, size), Image.Resampling.LANCZOS)


def dib_entry(im):
    """标准 32 位 BMP 图标（自下而上 BGRA + 空 AND 遮罩）。任务栏不吃 PNG 套壳的 ico。"""
    im = im.convert("RGBA")
    w, h = im.size
    pixels = im.tobytes()
    xor_rows = []
    for y in range(h - 1, -1, -1):
        row = bytearray()
        for x in range(w):
            r, g, b, a = pixels[(y * w + x) * 4 : (y * w + x) * 4 + 4]
            row += bytes((b, g, r, a))
        xor_rows.append(bytes(row))
    xor = b"".join(xor_rows)
    mask_stride = ((w + 31) // 32) * 4
    mask = b"\x00" * (mask_stride * h)
    header = struct.pack(
        "<IIIHHIIIIII",
        40,
        w,
        h * 2,
        1,
        32,
        0,
        len(xor) + len(mask),
        0,
        0,
        0,
        0,
    )
    return header + xor + mask


def write_ico(path, images):
    import struct as _struct

    count = len(images)
    header = _struct.pack("<HHH", 0, 1, count)
    blobs = [dib_entry(im) for im in images]
    offset = 6 + 16 * count
    entries = b""
    for im, blob in zip(images, blobs):
        w = 0 if im.width >= 256 else im.width
        h = 0 if im.height >= 256 else im.height
        entries += _struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    path.write_bytes(header + entries + b"".join(blobs))


def main():
    master = render_master()
    pngs = list(ROOT.rglob("*.png"))
    for path in pngs:
        raw = path.read_bytes()
        w = int.from_bytes(raw[16:20], "big")
        h = int.from_bytes(raw[20:24], "big")
        size = max(w, h)
        icon = down(master, size)
        if w != h:
            canvas = Image.new("RGBA", (w, h), (0, 0, 0, 0))
            icon = down(master, min(w, h))
            canvas.paste(icon, ((w - icon.width) // 2, (h - icon.height) // 2), icon)
            icon = canvas
        icon.save(path, "PNG")
        print(f"png {w}x{h} {path.relative_to(ROOT)}")

    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [down(master, n) for n in sizes]
    ico = ROOT / "icon.ico"
    write_ico(ico, images)
    print(f"ico {ico.stat().st_size} bytes")


if __name__ == "__main__":
    main()
