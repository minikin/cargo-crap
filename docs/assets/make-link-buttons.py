"""Draw the link buttons at the top of the README.

The buttons copy the project banner on minikin.me: DM Sans Medium, an 8px
radius, a zinc border and a faint indigo shadow, each in a light and a dark
version. GitHub shows README images through <img>, where web fonts never
load, so each label is converted to outlines and renders the same everywhere.

    pip install fonttools uharfbuzz
    curl -sSLo /tmp/DMSans.ttf \
      'https://github.com/google/fonts/raw/main/ofl/dmsans/DMSans%5Bopsz,wght%5D.ttf'
    python3 docs/assets/make-link-buttons.py /tmp/DMSans.ttf docs/assets

Add a button by adding a line to BUTTONS, then reference both SVGs from the
README with a <picture> element, like the others.
"""

import sys
import tempfile
from pathlib import Path

import uharfbuzz as hb
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

BUTTONS = {
    "link-blog-untested-complexity": ("post", "Blog: Finding untested complexity"),
    "link-blog-triaging-duplicates": ("post", "Blog: Triaging duplicates with TypeSafe"),
    "link-talk-youtube": ("youtube", "Talk on YouTube"),
}

SIZE = 14.5
H = 40
RADIUS = 8
PAD_L, ICON, GAP, PAD_R = 14, 16, 9, 16
M = 5  # room for the shadow around the button

# Feather icons (MIT), 24x24 stroke paths.
ICONS = {
    "post": (
        '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>'
        '<polyline points="14 2 14 8 20 8"/>'
        '<line x1="16" y1="13" x2="8" y2="13"/>'
        '<line x1="16" y1="17" x2="8" y2="17"/>'
        '<polyline points="10 9 9 9 8 9"/>'
    ),
    "youtube": (
        '<path d="M22.54 6.42a2.78 2.78 0 0 0-1.94-2C18.88 4 12 4 12 4s-6.88 0-8.6.46'
        "a2.78 2.78 0 0 0-1.94 2A29 29 0 0 0 1 11.75a29 29 0 0 0 .46 5.33A2.78 2.78 0 0 0 3.4 19"
        "c1.72.46 8.6.46 8.6.46s6.88 0 8.6-.46a2.78 2.78 0 0 0 1.94-2 29 29 0 0 0 .46-5.25"
        ' 29 29 0 0 0-.46-5.33z"/>'
        '<polygon points="9.75 15.02 15.5 11.75 9.75 8.48 9.75 15.02"/>'
    ),
}

THEMES = {
    "": dict(bg="#ffffff", border="#e4e4e7", text="#3f3f46", icon="#a1a1aa",
             shadow="#5046e5", shadow_opacity=0.10),
    "-dark": dict(bg="#161b22", border="#30363d", text="#e6edf3", icon="#8b949e",
                  shadow="#000000", shadow_opacity=0.35),
}


def number(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


class Label:
    def __init__(self, variable_font):
        instance = Path(tempfile.gettempdir()) / "DMSans-500.ttf"
        font = TTFont(variable_font)
        instantiateVariableFont(font, {"wght": 500, "opsz": 14}, inplace=True)
        font.save(instance)
        self.font = TTFont(instance)
        self.glyphs = self.font.getGlyphSet()
        self.order = self.font.getGlyphOrder()
        self.upem = self.font["head"].unitsPerEm
        self.cap = self.font["OS/2"].sCapHeight
        self.hb_font = hb.Font(hb.Face(hb.Blob.from_file_path(str(instance))))

    def outline(self, text, x0, baseline):
        """The shaped text as one SVG path, and its advance width in px."""
        buf = hb.Buffer()
        buf.add_str(text)
        buf.guess_segment_properties()
        hb.shape(self.hb_font, buf, {"kern": True, "liga": True})
        scale = SIZE / self.upem
        pen_x, parts = 0, []
        for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
            pen = SVGPathPen(self.glyphs, ntos=number)
            tx = x0 + (pen_x + pos.x_offset) * scale
            ty = baseline - pos.y_offset * scale
            glyph = self.glyphs[self.order[info.codepoint]]
            glyph.draw(TransformPen(pen, (scale, 0, 0, -scale, tx, ty)))
            if d := pen.getCommands():
                parts.append(d)
            pen_x += pos.x_advance
        return " ".join(parts), pen_x * scale


def button(label, icon, text, t):
    baseline = M + H / 2 + label.cap * SIZE / label.upem / 2
    d, width = label.outline(text, M + PAD_L + ICON + GAP, round(baseline, 2))
    w = round(PAD_L + ICON + GAP + width + PAD_R)
    total_w, total_h = w + 2 * M, H + 2 * M
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" role="img" aria-label="{text}">
<title>{text}</title>
<defs><filter id="s" x="-10%" y="-30%" width="120%" height="160%"><feDropShadow dx="0" dy="1.5" stdDeviation="2" flood-color="{t['shadow']}" flood-opacity="{t['shadow_opacity']}"/></filter></defs>
<rect x="{M + 0.5}" y="{M + 0.5}" width="{w - 1}" height="{H - 1}" rx="{RADIUS}" fill="{t['bg']}" stroke="{t['border']}" filter="url(#s)"/>
<g transform="translate({M + PAD_L} {number(M + (H - ICON) / 2)}) scale({number(ICON / 24)})" fill="none" stroke="{t['icon']}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{ICONS[icon]}</g>
<path fill="{t['text']}" d="{d}"/>
</svg>
"""


def main(variable_font, out_dir):
    label = Label(variable_font)
    out = Path(out_dir)
    for name, (icon, text) in BUTTONS.items():
        for suffix, theme in THEMES.items():
            path = out / f"{name}{suffix}.svg"
            path.write_text(button(label, icon, text, theme))
            print(path)


if __name__ == "__main__":
    main(*sys.argv[1:3])
