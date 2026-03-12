import re

with open('src/draw.rs', 'r') as f:
    text = f.read()

# 1. Remove pixels from BlockData
text = re.sub(r'pixels:\s*Vec<PrePixel>,\s*(fg:\s*\[u8; 3\],)', r'\1', text)

# 2. In GPU path, remove the allocation of pixels
gpu_push_pattern = r'''let mut pixels = Vec::with_capacity\(bp\);\s+let start_idx = result_idx \* bp;\s+for i in 0\.\.bp \{\s+pixels\.push\(all_blocks_pixels\[start_idx \+ i\]\);\s+\}'''
text = re.sub(gpu_push_pattern, '', text)

# 3. In GPU path, remove pixels from BlockData push
text = re.sub(r'row_data\.push\(BlockData \{\s*pixels,\s*fg: final_fg,', r'row_data.push(BlockData {\n                    fg: final_fg,', text)

# 4. In CPU path, remove best_block_pixels and pixels
text = re.sub(r'let mut best_block_pixels = Vec::new\(\);', '', text)
text = re.sub(r'best_block_pixels = pixels\.to_vec\(\);', '', text)
text = re.sub(r'let pixels = best_block_pixels;', '', text)
text = re.sub(r'row_data\.push\(BlockData \{\s*pixels,\s*fg: final_fg,', r'row_data.push(BlockData {\n                        fg: final_fg,', text)

# 5. In Phase 2, replace `px` lookup
phase2_lookup_pattern = r'let px = &data\.pixels\[i\];'
phase2_lookup_replacement = r'''let lx = i % gw;
                        let ly = i / gw;
                        let rgb = crate::draw::unpack_rgb(image.block[y][x].pixels[ly][lx].orig);
                        let px_r = rgb.0 as u64;
                        let px_g = rgb.1 as u64;
                        let px_b = rgb.2 as u64;'''

def replace_px(match):
    return '''let lx = i % gw;
                        let ly = i / gw;
                        let rgb = unpack_rgb(image.block[y][x].pixels[ly][lx].orig);
                        let px_r = rgb[0] as u64;
                        let px_g = rgb[1] as u64;
                        let px_b = rgb[2] as u64;'''

text = re.sub(phase2_lookup_pattern, replace_px, text)

# Replace px.r, px.g, px.b in Phase 2 with px_r, px_g, px_b
# Note: we only do this where data.pixels[i] was used.
text = re.sub(r'px\.r\.abs_diff', r'px_r.abs_diff', text)
text = re.sub(r'px\.g\.abs_diff', r'px_g.abs_diff', text)
text = re.sub(r'px\.b\.abs_diff', r'px_b.abs_diff', text)

with open('src/draw.rs', 'w') as f:
    f.write(text)

print("Applied fix_draw.py")
