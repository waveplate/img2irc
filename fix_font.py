import sys

with open('src/font.rs', 'r') as f:
    lines = f.readlines()

# The correct export_glyphs_json ends around line 711
# We want to find the first 'let bp = cw * ch;' after line 700
# and remove everything until the line before 'pub fn list_ranges()'

start_line = -1
end_line = -1

for i, line in enumerate(lines):
    if i > 710 and 'let bp = cw * ch;' in line:
        start_line = i
        break

if start_line == -1:
    print("Could not find orphaned start")
    sys.exit(1)

for i in range(start_line, len(lines)):
    if 'pub fn list_ranges()' in lines[i]:
        end_line = i - 1
        break

if end_line == -1:
    print("Could not find list_ranges")
    sys.exit(1)

# Remove the lines
new_lines = lines[:start_line-1] + lines[end_line:]

with open('src/font.rs', 'w') as f:
    f.writelines(new_lines)

print(f"Removed orphaned lines {start_line} to {end_line}")
