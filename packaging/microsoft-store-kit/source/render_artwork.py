#!/usr/bin/env python3
"""Render Store PNGs from repository-native SVG branding (requires cairosvg)."""
from pathlib import Path
import math
import xml.etree.ElementTree as ET
import cairosvg

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parents[1] / 'artwork'
SVG = ROOT / 'assets/branding/electrocraft.svg'


def brand(x, y, size):
    tree = ET.fromstring(SVG.read_text())
    tree.set('x', str(x))
    tree.set('y', str(y))
    tree.set('width', str(size))
    tree.set('height', str(size))
    return ET.tostring(tree, encoding='unicode')


def wrap(w, h, body):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
<title>Electrocraft Store artwork</title>
<defs>
 <linearGradient id="canvas" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#2c3547"/><stop offset="1" stop-color="#141820"/></linearGradient>
 <radialGradient id="halo"><stop stop-color="#67b7f0" stop-opacity=".18"/><stop offset="1" stop-color="#67b7f0" stop-opacity="0"/></radialGradient>
 <pattern id="grid" width="48" height="48" patternUnits="userSpaceOnUse"><path d="M48 0H0V48" fill="none" stroke="#a8c6e6" stroke-opacity=".055" stroke-width="1"/></pattern>
</defs>
{body}</svg>'''


def surface(w, h):
    return f'<rect width="{w}" height="{h}" fill="url(#canvas)"/><rect width="{w}" height="{h}" fill="url(#grid)"/>'


def titled(w, h):
    size = min(w * .63, h * .4)
    return wrap(w, h, surface(w, h) + brand((w-size)/2, h*.06, size) + f'''
<text x="{w/2}" y="{h*.59}" fill="#eef4ff" font-family="Inter, sans-serif" font-weight="600" font-size="{w*.091}" text-anchor="middle">Electrocraft</text>
<path d="M{w*.18} {h*.65}H{w*.82}" stroke="#67b7f0" stroke-width="3" stroke-opacity=".65"/>
''')


def hero():
    w, h = 1920, 1080
    # Original abstract circuit artwork: no UI, words, or product title.
    body = surface(w, h) + '<ellipse cx="960" cy="390" rx="660" ry="420" fill="url(#halo)"/>'
    body += '''<g fill="none" stroke="#52e07e" stroke-width="12" stroke-linejoin="round" stroke-linecap="round">
<path d="M-20 320H320V200H680V320H800M1120 320H1240V200H1600V320H1940"/>
<path d="M-20 620H260V480H520V620H790M1130 620H1400V480H1670V620H1940"/>
<path d="M320 200V80H680M1600 200V80H1240"/>
</g>
<g fill="#ffe066">'''
    for x,y in [(260,620),(320,200),(520,480),(680,200),(1240,200),(1400,480),(1600,200),(1670,620)]:
        body += f'<circle cx="{x}" cy="{y}" r="13"/>'
    body += '</g>'
    body += '<g fill="none" stroke="#a7c8e6" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"><path d="M370 320h28l18 -35 28 70 28 -70 28 70 28 -70 28 70 18 -35h28"/><path d="M1300 320h28l18 -35 28 70 28 -70 28 70 28 -70 28 70 18 -35h28"/></g>'
    body += '<path d="M1010 150L825 423H945L885 640L1095 327H975L1040 150Z" fill="#67b7f0" stroke="#d7ecff" stroke-width="9" stroke-linejoin="round"/>'
    pts = ' '.join(f'{x},{590+55*math.sin((x-100)/75)}' for x in range(100,1821,5))
    body += f'<polyline points="{pts}" fill="none" stroke="#62c9ef" stroke-width="5" opacity=".38"/>'
    return wrap(w,h,body)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    jobs = {
        'StoreLogo300x300': (300, 300, SVG.read_text()),
        'StoreLogo1080x1080': (1080, 1080, titled(1080,1080)),
        'StoreLogo720x1080': (720, 1080, titled(720,1080)),
        'StoreLogoOverride150x150': (150, 150, SVG.read_text()),
        'StoreLogoOverride71x71': (71, 71, SVG.read_text()),
        'PromoImage1920x1080': (1920, 1080, hero()),
    }
    for name,(w,h,svg) in jobs.items():
        (OUT / f'{name}.svg').write_text(svg)
        cairosvg.svg2png(bytestring=svg.encode(),write_to=str(OUT / f'{name}.png'),output_width=w,output_height=h)
        print(f'{name}.png: {w} x {h}')


if __name__ == '__main__':
    main()
