#!/usr/bin/env python3
import argparse
import json
from PIL import Image

def main():
    p = argparse.ArgumentParser(
        description="Slice a row-image of glyphs into bitmaps and output JSON mapping."
    )
    p.add_argument('chars_file', help='Space-delimited glyphs text file (UTF-8)')
    p.add_argument('image_file', help='Image with glyphs in one row, no gaps')
    p.add_argument('--icw', type=int, required=True, help='Input char width (px)')
    p.add_argument('--ich', type=int, required=True, help='Input char height (px)')
    p.add_argument('--threshold', type=int, required=True, help='B/W threshold (0–255)')
    p.add_argument('--obw', type=int, required=True, help='Output bitmap width')
    p.add_argument('--obh', type=int, required=True, help='Output bitmap height')
    p.add_argument('--output', required=True, help='Output JSON filename')
    args = p.parse_args()

    # 1) Read glyphs
    with open(args.chars_file, 'r', encoding='utf-8') as f:
        glyphs = f.read().strip().split(' ')

    # 2) Load image & sanity-check
    im = Image.open(args.image_file).convert('L')
    w, h = im.size
    if h != args.ich:
        raise ValueError(f"Image height {h} ≠ ich {args.ich}")
    count = w // args.icw
    if count * args.icw != w:
        raise ValueError(f"Image width {w} not a multiple of icw {args.icw}")
    if len(glyphs) != count:
        raise ValueError(f"{len(glyphs)} glyphs but {count} blocks in image")

    # 3) Build mapping glyph → grid
    mapping = {}
    for i, glyph in enumerate(glyphs):
        left = i * args.icw
        block = im.crop((left, 0, left + args.icw, args.ich))
        block = block.resize((args.obw, args.obh), Image.LANCZOS)
        data = list(block.getdata())
        bits = [1 if pix > args.threshold else 0 for pix in data]
        grid = [bits[row*args.obw:(row+1)*args.obw] for row in range(args.obh)]
        mapping[glyph] = grid

    # 4) Write JSON manually so rows stay inline
    with open(args.output, 'w', encoding='utf-8') as out:
        out.write('{\n')
        total = len(mapping)
        for idx, (glyph, grid) in enumerate(mapping.items()):
            # properly quote the glyph
            key = json.dumps(glyph, ensure_ascii=False)
            out.write(f'    {key}: [\n')
            for ridx, row in enumerate(grid):
                row_str = ','.join(str(bit) for bit in row)
                comma = ',' if ridx < len(grid) - 1 else ''
                out.write(f'        [{row_str}]{comma}\n')
            out.write('    ]')
            out.write(',\n' if idx < total - 1 else '\n')
        out.write('}\n')

if __name__ == '__main__':
    main()
