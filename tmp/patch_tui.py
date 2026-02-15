
import sys

def patch_tui():
    with open('src/bin/tui.rs', 'r') as f:
        lines = f.readlines()

    new_lines = []
    in_toggle_control = False
    in_ui = False
    
    # Logic for toggle_control
    # We look for "25 => app.edge_detection = !app.edge_detection,"
    # Then we skip a few lines to find the closing brace of "else if app.current_tab == 1"
    # And insert our code.
    
    # Logic for ui
    # We look for "1 => draw_effects_tab(f, app, &chunks[0])," inside ui
    # And insert "2 => draw_blocks_tab(f, app, &chunks[0]),"
    
    for i, line in enumerate(lines):
        new_lines.append(line)
        
        # UI Patch
        if "1 => draw_effects_tab(f, app, &chunks[0])," in line:
             new_lines.append("            2 => draw_blocks_tab(f, app, &chunks[0]),\n")
             
        # Toggle Control Patch
        if "25 => app.edge_detection = !app.edge_detection," in line:
            # The next lines are:
            # _ => {}
            # }
            # }
            # We want to insert after the SECOND closing brace from here?
            # actually, just append after this line, but we need to close the match and the else if block first?
            # No, I should find the line that closes the else if block.
            # It's hard to know exactly which line without parsing.
            # But I know the structure:
            # 25 => ...
            # _ => {}
            # }
            # }
            pass

    # Re-reading to apply toggle_control patch more robustly
    # I'll search for the line "25 => ..." and then count braces?
    # Or just replace the specific block ending.
    
    final_lines = []
    iter_lines = iter(new_lines)
    for line in iter_lines:
        final_lines.append(line)
        if "25 => app.edge_detection = !app.edge_detection," in line:
            # consume next line: "            _ => {}"
            final_lines.append(next(iter_lines))
            # consume next line: "        }" (closing match)
            final_lines.append(next(iter_lines))
            # consume next line: "    }" (closing else if)
            final_lines.append(next(iter_lines))
            
            # Now insert our code
            final_lines.append("    } else if app.current_tab == 2 {\n")
            final_lines.append("         if app.selected_control < app.available_blocks.len() {\n")
            final_lines.append("             let name = app.available_blocks[app.selected_control].clone();\n")
            final_lines.append("             if app.render_args.blocks.is_empty() {\n")
            final_lines.append("                 app.render_args.blocks = vec![name];\n")
            final_lines.append("             } else {\n")
            final_lines.append("                 if let Some(pos) = app.render_args.blocks.iter().position(|x| *x == name) {\n")
            final_lines.append("                     app.render_args.blocks.remove(pos);\n")
            final_lines.append("                 } else {\n")
            final_lines.append("                     app.render_args.blocks.push(name);\n")
            final_lines.append("                 }\n")
            final_lines.append("             }\n")
            final_lines.append("         }\n")
    
    with open('src/bin/tui.rs', 'w') as f:
        f.writelines(final_lines)

patch_tui()
