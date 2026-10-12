#!/usr/bin/env python3
"""Renders the README badges as glass pill SVGs.

    render.py OUT_DIR [--version V] [--ci passing|failing] [--coverage PCT]
                      [--crap SHIELDS_JSON] [--license NAME]

Writes crates.svg, ci.svg, coverage.svg, crap.svg and license.svg into
OUT_DIR, one for each value given; a badge whose value is not given is
left as it is in OUT_DIR. --crap reads the Shields.io endpoint JSON that
`cargo crap --format shields` writes. Each SVG follows prefers-color-scheme,
so one file serves GitHub's light and dark themes and crates.io. Ported
from minikin/popover's tool/badges/render.py.
"""

import argparse
import json
from html import escape
from pathlib import Path

# SF Pro advance widths per 1000 units at weights 400 and 600, read from
# SFNS.ttf with fontTools. Segoe UI and Roboto are close enough for padding.
REGULAR = {
    "a": 552, "b": 614, "c": 560, "d": 614, "e": 571, "f": 362, "g": 609,
    "h": 588, "i": 247, "j": 247, "k": 543, "l": 253, "m": 870, "n": 583,
    "o": 591, "p": 610, "q": 609, "r": 381, "s": 523, "t": 363, "u": 583,
    "v": 542, "w": 774, "x": 524, "y": 543, "z": 539, "A": 674, "B": 657,
    "C": 716, "D": 727, "E": 596, "F": 572, "G": 747, "H": 742, "I": 268,
    "J": 538, "K": 659, "L": 568, "M": 874, "N": 742, "O": 771, "P": 635,
    "Q": 771, "R": 653, "S": 637, "T": 634, "U": 737, "V": 674, "W": 968,
    "X": 679, "Y": 655, "Z": 662, "0": 630, "1": 464, "2": 604, "3": 627,
    "4": 644, "5": 618, "6": 637, "7": 569, "8": 639, "9": 637, ".": 297,
    "%": 925, "-": 472, " ": 281, ",": 297,
}
SEMIBOLD = {
    "a": 577, "b": 639, "c": 578, "d": 639, "e": 591, "f": 393, "g": 633,
    "h": 618, "i": 274, "j": 273, "k": 582, "l": 281, "m": 912, "n": 613,
    "o": 610, "p": 635, "q": 635, "r": 417, "s": 552, "t": 396, "u": 613,
    "v": 569, "w": 823, "x": 562, "y": 579, "z": 557, "A": 713, "B": 681,
    "C": 733, "D": 740, "E": 613, "F": 589, "G": 756, "H": 770, "I": 301,
    "J": 580, "K": 692, "L": 586, "M": 896, "N": 759, "O": 781, "P": 661,
    "Q": 781, "R": 681, "S": 663, "T": 651, "U": 752, "V": 706, "W": 996,
    "X": 712, "Y": 690, "Z": 671, "0": 667, "1": 496, "2": 630, "3": 655,
    "4": 672, "5": 648, "6": 668, "7": 593, "8": 675, "9": 668, ".": 331,
    "%": 999, "-": 480, " ": 266, ",": 331,
}
FONT_SIZE = 11.5
FONT = "-apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,Helvetica,Arial,sans-serif"
HEIGHT = 24
MARGIN = 2
PAD_LEFT = 10
PAD_RIGHT = 11
DOT = 3.5
HALO = 5.5
GAP = 6

STYLE = (
    ".a{stop-color:#fff}.b{stop-color:#f1f2f6}"
    ".e{stroke:#141428;stroke-opacity:.1}"
    ".t{stop-color:#fff;stop-opacity:1}"
    ".h{fill-opacity:.18}.l{fill:#686874}.v{fill:#18181c}"
    "@media (prefers-color-scheme:dark){"
    ".a{stop-color:#2a2b31}.b{stop-color:#1b1c21}"
    ".e{stroke:#fff;stroke-opacity:.08}"
    ".t{stop-opacity:.12}"
    ".h{fill-opacity:.28}.l{fill:#9a9aa5}.v{fill:#f2f2f5}}"
    f"text{{font-family:{FONT};font-size:{FONT_SIZE}px}}.v{{font-weight:600}}"
)

BLUE = "#0a84ff"
GREEN = "#30d158"
ORANGE = "#ff9f0a"
RED = "#ff453a"
PURPLE = "#bf5af2"

# Shields.io colour names, as `cargo crap --format shields` writes them.
SHIELDS = {"brightgreen": GREEN, "green": GREEN, "yellow": ORANGE, "orange": ORANGE, "red": RED}


def text_width(text, bold=False):
    widths = SEMIBOLD if bold else REGULAR
    return sum(widths.get(ch, 640) for ch in text) * FONT_SIZE / 1000


def badge(label, value, color):
    label_w = text_width(label)
    value_w = text_width(value, bold=True)
    # Measured as shown, written escaped: "CRAP > 15" is nine characters wide.
    label, value = escape(label), escape(value)
    pill_w = round(PAD_LEFT + 2 * DOT + GAP + label_w + GAP + value_w + PAD_RIGHT)
    width = pill_w + 2 * MARGIN
    height = HEIGHT + 2 * MARGIN + 1
    m = MARGIN
    radius = (HEIGHT - 1) / 2
    cx = m + PAD_LEFT + DOT
    cy = m + HEIGHT / 2
    x_label = m + PAD_LEFT + 2 * DOT + GAP
    x_value = x_label + label_w + GAP
    y_text = cy + FONT_SIZE * 0.36
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" '
        f'height="{height}" viewBox="0 0 {width} {height}" role="img" '
        f'aria-label="{label}: {value}">'
        f"<title>{label}: {value}</title>"
        f"<style>{STYLE}</style>"
        "<defs>"
        '<linearGradient id="f" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" class="a"/><stop offset="1" class="b"/>'
        "</linearGradient>"
        '<linearGradient id="g" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" class="t"/>'
        '<stop offset=".45" stop-color="#fff" stop-opacity="0"/>'
        "</linearGradient>"
        '<filter id="s" x="-10%" y="-20%" width="120%" height="160%">'
        '<feDropShadow dx="0" dy="1" stdDeviation="1" '
        'flood-color="#141428" flood-opacity=".1"/>'
        "</filter>"
        "</defs>"
        f'<rect x="{m + 0.5}" y="{m + 0.5}" width="{pill_w - 1}" '
        f'height="{HEIGHT - 1}" rx="{radius}" fill="url(#f)" class="e" '
        'filter="url(#s)"/>'
        f'<rect x="{m + 1.5}" y="{m + 1.5}" width="{pill_w - 3}" '
        f'height="{HEIGHT - 3}" rx="{radius - 1}" fill="none" '
        'stroke="url(#g)"/>'
        f'<circle cx="{cx}" cy="{cy}" r="{HALO}" fill="{color}" class="h"/>'
        f'<circle cx="{cx}" cy="{cy}" r="{DOT}" fill="{color}"/>'
        f'<text class="l" x="{x_label:.1f}" y="{y_text:.1f}">{label}</text>'
        f'<text class="v" x="{x_value:.1f}" y="{y_text:.1f}">{value}</text>'
        "</svg>\n"
    )


def coverage_color(percent):
    if percent >= 90:
        return GREEN
    if percent >= 75:
        return ORANGE
    return RED


def crap_badge(endpoint):
    """The CRAP pill from a Shields.io endpoint document: its label (which
    names the threshold) and message, its colour mapped to the pill palette."""
    return badge(endpoint["label"], endpoint["message"], SHIELDS.get(endpoint["color"], ORANGE))


def badges(args):
    result = {}
    if args.version:
        result["crates"] = badge("crates.io", f"v{args.version}", BLUE)
    if args.ci:
        result["ci"] = badge("CI", args.ci, GREEN if args.ci == "passing" else RED)
    if args.coverage:
        percent = float(args.coverage)
        result["coverage"] = badge("coverage", f"{percent:.1f}%", coverage_color(percent))
    if args.crap:
        result["crap"] = crap_badge(json.loads(Path(args.crap).read_text()))
    if args.license:
        result["license"] = badge("license", args.license, PURPLE)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    for name in ("version", "ci", "coverage", "crap", "license"):
        parser.add_argument(f"--{name}")
    args = parser.parse_args()
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    for name, svg in badges(args).items():
        (out_dir / f"{name}.svg").write_text(svg)


if __name__ == "__main__":
    main()
