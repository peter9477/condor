#!/usr/bin/env python3
"""Draws Condor's icon (an Andean condor soaring, seen from below/front) and writes
condor.svg, condor.png (256 px) and condor.ico. Run from this folder; needs Pillow.
Then compile condor.rc (llvm-rc condor.rc) to condor.res, which the build links in."""
import math
from PIL import Image, ImageDraw

S = 256  # design units

def quad(p0, c, p1, n=24):
    return [((1-t)**2*p0[0] + 2*(1-t)*t*c[0] + t*t*p1[0], (1-t)**2*p0[1] + 2*(1-t)*t*c[1] + t*t*p1[1])
            for t in (i/n for i in range(n+1))]

def finger(root, deg, length, wmax=4.2, lift=4.0, n=10):
    """One primary: narrow at the root, fullest in the middle, a rounded tip curving slightly up."""
    ang = math.radians(deg)
    d = (math.cos(ang), math.sin(ang))
    nrm = (-d[1], d[0])
    T = 0.88                                 # the straight part; the round cap covers the rest
    width = lambda t: wmax * (0.6 + 0.4 * math.sin(math.pi * t))
    def at(t, side):
        w = width(t) * side
        return (root[0] + d[0]*length*t + nrm[0]*w, root[1] + d[1]*length*t + nrm[1]*w - lift*t*t)
    c, r = at(T, 0), width(T)
    cap = [(c[0] + r*(-nrm[0]*math.cos(a) + d[0]*math.sin(a)), c[1] + r*(-nrm[1]*math.cos(a) + d[1]*math.sin(a)))
           for a in (math.pi * k / 8 for k in range(1, 8))]
    return [at(T*i/n, -1) for i in range(n+1)] + cap + [at(T*i/n, 1) for i in range(n, -1, -1)]

def wing():
    """Left wing: a broad, nearly flat plank, with eight long primaries splayed at the tip."""
    arm = quad((126, 96), (90, 84), (54, 88)) + [(51, 110)] + quad((58, 136), (92, 147), (126, 147))
    fingers = []
    for i in range(8):
        f = i / 7
        root = (57 + 5*f, 91 + 42*f)         # along the end of the hand
        fingers.append(finger(root, 198 - 52*f, 48 - 18*abs(f - 0.3), wmax=3.8))  # fanning up-left to down-left,
    return [arm] + fingers                   # the rear ones shorter

def patch():
    """The white triangle along the front of the underwing: widest at the body, a point at the wrist."""
    return quad((118, 100), (90, 89.5), (60, 92.5)) + quad((62, 98.5), (94, 105), (118, 127))

def rot_ellipse(cx, cy, rx, ry, deg, n=40):
    a = math.radians(deg)
    return [(cx + rx*math.cos(t)*math.cos(a) - ry*math.sin(t)*math.sin(a),
             cy + rx*math.cos(t)*math.sin(a) + ry*math.sin(t)*math.cos(a))
            for t in (2*math.pi*i/n for i in range(n))]

def head():
    """Long bare head and neck, in profile, turned to the right and rising from the ruff."""
    return rot_ellipse(136, 86.5, 12.5, 6, -24)

def beak():
    """Pale hooked beak at the end of the head."""
    return [(143.5, 79.5), (149.5, 79.5), (153, 83), (152.5, 89), (150, 85.5), (145.5, 86.5)]

def tail():
    """Short, broad fan of a tail, right behind the wings."""
    return [(112, 144)] + quad((101, 172), (128, 184), (155, 172)) + [(144, 144)]

def foot(x, lean):
    """A pale foot tucked back along the tail base: a slim sliver, not a dot."""
    return rot_ellipse(x, 159, 2.2, 5.5, lean)

def mirror(pts):
    return [(S - x, y) for x, y in pts]

def polys(parts, col):
    return [("poly", p, col) for p in parts]

BG_TOP, BG_BOT = (154, 188, 226), (197, 226, 246)   # sky, lightened so the bird stands out small
BIRD, PATCH, RUFF, HEAD = (24, 26, 31), (240, 242, 238), (46, 48, 55), (232, 146, 116)
BEAK, FEET = (242, 232, 206), (214, 208, 198)

shapes = [  # (kind, data, colour), back to front
    *polys(wing() + [mirror(p) for p in wing()], BIRD),
    *polys([patch(), mirror(patch())], PATCH),
    ("poly", tail(), BIRD),
    ("ellipse", (128, 124, 17, 31), BIRD),                              # body
    *polys([foot(123.5, 8), foot(132.5, -8)], FEET),
    ("poly", head(), HEAD),
    ("poly", beak(), BEAK),
    ("ellipse", (128, 100, 14, 8), RUFF),                              # dark ruff, over the neck base
]

def render(px):
    k = 4 * px / S
    big = 4 * px
    im = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    # Rounded-square sky badge with a vertical gradient.
    grad = Image.new("RGBA", (big, big))
    gd = ImageDraw.Draw(grad)
    for y in range(big):
        t = y / (big - 1)
        gd.line([(0, y), (big, y)], fill=tuple(round(a + (b-a)*t) for a, b in zip(BG_TOP, BG_BOT)) + (255,))
    mask = Image.new("L", (big, big), 0)
    m = 6 * k
    ImageDraw.Draw(mask).rounded_rectangle([m, m, big - m, big - m], radius=52*k, fill=255)
    im.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(im)
    for kind, data, col in shapes:
        if kind == "poly":
            d.polygon([(x*k, y*k) for x, y in data], fill=col + (255,))
        else:
            cx, cy, rx, ry = data
            d.ellipse([(cx-rx)*k, (cy-ry)*k, (cx+rx)*k, (cy+ry)*k], fill=col + (255,))
    return im.resize((px, px), Image.LANCZOS)

def svg():
    hexc = lambda c: "#%02x%02x%02x" % c
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}">',
           '<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">'
           f'<stop offset="0" stop-color="{hexc(BG_TOP)}"/><stop offset="1" stop-color="{hexc(BG_BOT)}"/></linearGradient></defs>',
           '<rect x="6" y="6" width="244" height="244" rx="52" fill="url(#sky)"/>']
    for kind, data, col in shapes:
        if kind == "poly":
            out.append(f'<polygon fill="{hexc(col)}" points="' + " ".join(f"{x:.1f},{y:.1f}" for x, y in data) + '"/>')
        else:
            cx, cy, rx, ry = data
            out.append(f'<ellipse fill="{hexc(col)}" cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}"/>')
    out.append("</svg>")
    return "\n".join(out) + "\n"

if __name__ == "__main__":
    open("condor.svg", "w").write(svg())
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    imgs = {s: render(s) for s in sizes}
    imgs[256].save("condor.png")
    imgs[256].save("condor.ico", sizes=[(s, s) for s in sizes], append_images=[imgs[s] for s in sizes if s != 256])
    sheet = Image.new("RGBA", (sum(sizes) + 10*len(sizes), 256), (255, 255, 255, 255))
    x = 0
    for s in sizes:
        sheet.paste(imgs[s], (x, 0), imgs[s]); x += s + 10
    sheet.save("/tmp/claude-1000/-code-condor/0fd9e1e0-69fa-4e47-b410-f0c9d2b8ba12/scratchpad/icon-sheet.png")
