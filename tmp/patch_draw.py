
lines = []
with open('src/draw.rs', 'r') as f:
    lines = f.readlines()

new_lines = []
skip = False
for i, line in enumerate(lines):
    # 0-indexed i corresponds to line i+1
    lineno = i + 1
    
    # Line 751
    if lineno == 751:
        new_lines.append('                let mut candidates: Vec<(u64, usize, bool)> = Vec::new();\n')
        continue
        
    # Line 842
    if lineno == 842:
        new_lines.append('                        if edge_cost >= best_cost { continue; }\n')
        continue

    # Lines 858-862
    if lineno == 858: # if args.experimental
        continue
    if lineno == 859: # candidates.push
        continue
    if lineno == 860: # } else {
        continue
    if lineno == 862: # }
        continue
        
    # Lines 866-895
    if lineno >= 866 and lineno <= 895:
        continue

    new_lines.append(line)

with open('src/draw.rs', 'w') as f:
    f.writelines(new_lines)
