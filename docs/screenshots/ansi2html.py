"""Turns a screen that `tuitor --script` saved with @dump (truecolor ANSI) into an HTML page
for a headless browser to take a screenshot of, and prints the page's size in pixels.

usage: ansi2html.py SCREEN.ans PAGE.html

Backgrounds and text are two layers, as in a terminal, so that cells of one colour join
without seams; the block elements tuitor draws with (the lower eighths ▁ to █, and the
octants of the header's logo with the quadrants and halves among them) are painted as cells
of two colours rather than as glyphs, for the same reason and because fonts lack octants.
"""

import html
import re
import sys

FONT, CELL_W, CELL_H, MARGIN = 15, 7.5, 18, 12  # px; an Iosevka cell is half its size wide
BG, FG = '#0c0c0e', '#d6d6d6'
SGR = re.compile(r'\x1b\[([0-9;]*)m')

# The cells a block element fills, bit n for octant n + 1, left to right and top to bottom:
# those Unicode encodes outside the octants, then U+1CD00 on in the order of the rest's bits,
# as crates/tuitor/src/ui/logo.rs draws them.
NOT_OCTANTS = {
    0x00: ' ', 0x01: '\U0001CEA8', 0x02: '\U0001CEAB', 0x03: '\U0001FB82', 0x05: '▘', 0x0A: '▝',
    0x0F: '▀', 0x14: '\U0001FBE6', 0x28: '\U0001FBE7', 0x3F: '\U0001FB85', 0x40: '\U0001CEA3',
    0x50: '▖', 0x55: '▌', 0x5A: '▞', 0x5F: '▛', 0x80: '\U0001CEA0', 0xA0: '▗', 0xA5: '▚',
    0xAA: '▐', 0xAF: '▜', 0xC0: '▂', 0xF0: '▄', 0xF5: '▙', 0xFA: '▟', 0xFC: '▆', 0xFF: '█',
}
OCTANTS = {ch: bits for bits, ch in NOT_OCTANTS.items() if bits}
OCTANTS.update((chr(0x1CD00 + i), bits) for i, bits in enumerate(b for b in range(256) if b not in NOT_OCTANTS))


def cells(line):
    fg = bg = None
    bold = italic = False
    pos = 0
    for m in [*SGR.finditer(line), None]:
        for ch in line[pos:m.start() if m else len(line)]:
            yield ch, fg, bg, bold, italic
        if not m:
            return
        pos = m.end()
        codes = [int(c) for c in m.group(1).split(';') if c]
        i = 0
        while i < len(codes):
            if codes[i] == 0:
                fg = bg = None
                bold = italic = False
            elif codes[i] == 1:
                bold = True
            elif codes[i] == 3:
                italic = True
            elif codes[i] in (38, 48) and codes[i + 1:i + 2] == [2]:
                color = '#%02x%02x%02x' % tuple(codes[i + 2:i + 5])
                fg, bg = (color, bg) if codes[i] == 38 else (fg, color)
                i += 4
            i += 1


def octant(bits, fg, bg):
    """A cell of a 2 × 4 grid: the left column's rows over the whole cell, the right one's
    over its right half, so that rounding leaves no gap between them."""
    def column(side):
        colours = [fg if bits >> (2 * row + side) & 1 else bg for row in range(4)]
        stops = ','.join('%s %d%% %d%%' % (c, 25 * row, 25 * row + 25) for row, c in enumerate(colours))
        return 'linear-gradient(%s)' % stops
    return 'background:%s right/50%% 100%% no-repeat,%s;display:inline-block;height:%dpx;vertical-align:top' % (
        column(1), column(0), CELL_H)


def style(cell, text):
    ch, fg, bg, bold, italic = cell
    if ch in OCTANTS:
        if text:
            return ' ', 'color:transparent'
        return ' ', octant(OCTANTS[ch], fg or FG, bg or BG)
    if ch == '▀' or '▁' <= ch <= '█':
        if text:
            return ' ', 'color:transparent'
        cut = 50 if ch == '▀' else 100 - 100 * (ord(ch) - 0x2580) / 8
        top, bottom = (fg, bg) if ch == '▀' else (bg, fg)
        gradient = 'linear-gradient(%s %g%%,%s %g%%)' % (top or BG, cut, bottom or BG, cut)
        return ' ', 'background:%s;display:inline-block;height:%dpx;vertical-align:top' % (gradient, CELL_H)
    if not text:
        return ch, 'color:transparent' + (';background:' + bg if bg else '')
    return ch, 'color:' + (fg or FG) + (';font-weight:700' if bold else '') + (';font-style:italic' if italic else '')


def layer(rows, text):
    lines = []
    for row in rows:
        spans, run, last = [], [], None
        for cell in row:
            ch, css = style(cell, text)
            if css != last and run:
                spans.append('<span style="%s">%s</span>' % (last, html.escape(''.join(run))))
                run = []
            last = css
            run.append(ch)
        if run:
            spans.append('<span style="%s">%s</span>' % (last, html.escape(''.join(run))))
        lines.append(''.join(spans))
    return '\n'.join(lines)


screen, page = sys.argv[1], sys.argv[2]
text = open(screen, encoding='utf-8').read().replace('\x1b[2J\x1b[H', '')
rows = [list(cells(line)) for line in text.rstrip('\n').split('\n')]
with open(page, 'w', encoding='utf-8') as out:
    out.write(
        '<!doctype html><meta charset="utf-8"><style>html,body{margin:0;background:%s}'
        'div{position:relative;margin:%dpx}pre{margin:0;font:%dpx/%dpx "Iosevka Nerd Font Mono","Iosevka",monospace;'
        'font-variant-ligatures:none}pre+pre{position:absolute;top:0;left:0}</style>'
        '<div><pre>%s</pre><pre>%s</pre></div>\n' % (BG, MARGIN, FONT, CELL_H, layer(rows, False), layer(rows, True))
    )
width = max(len(row) for row in rows)
print('%d,%d' % (round(width * CELL_W + 2 * MARGIN), len(rows) * CELL_H + 2 * MARGIN))
