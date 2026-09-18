import sys

def main():
    with open('src/tui.rs', 'r') as f:
        content = f.read()

    # 1. Add batch_size to AutoOptimizeState
    old_state = """struct AutoOptimizeState {
    inputs: Vec<TextArea<'static>>,
    focused_idx: usize,
    optimize_shortest_longest_line: bool,
}"""
    new_state = """struct AutoOptimizeState {
    inputs: Vec<TextArea<'static>>,
    focused_idx: usize,
    optimize_shortest_longest_line: bool,
    batch_size: u32,
}"""
    content = content.replace(old_state, new_state)

    # 2. Add batch_size initialization
    old_init = """        self.auto_optimize_state = Some(AutoOptimizeState {
            inputs,
            focused_idx: 0,
            optimize_shortest_longest_line: self.saved_auto_optimize_shortest_longest_line,
        });"""
    new_init = """        self.auto_optimize_state = Some(AutoOptimizeState {
            inputs,
            focused_idx: 0,
            optimize_shortest_longest_line: self.saved_auto_optimize_shortest_longest_line,
            batch_size: self.render_args.auto_optimize_batch_size,
        });"""
    content = content.replace(old_init, new_init)

    # 3. Save batch_size before flush
    old_start = """    fn start_auto_optimization(&mut self) {
        if self.is_optimizing {
            return;
        }

        self.flush_render_args();

        let state = self.auto_optimize_state.take().unwrap();

        let optimize_shortest_longest_line = state.optimize_shortest_longest_line;"""
    new_start = """    fn start_auto_optimization(&mut self) {
        if self.is_optimizing {
            return;
        }

        if let Some(state) = &self.auto_optimize_state {
            self.render_args.auto_optimize_batch_size = state.batch_size;
        }
        self.flush_render_args();

        let state = self.auto_optimize_state.take().unwrap();

        let optimize_shortest_longest_line = state.optimize_shortest_longest_line;"""
    content = content.replace(old_start, new_start)

    # 4. Update the KeyHandler
    old_handler = """                            match key.code {
                                KeyCode::Esc => {
                                    app.input_mode = InputMode::Normal;
                                }
                                KeyCode::Enter => {
                                    app.input_mode = InputMode::Normal;
                                    app.start_auto_optimization();
                                }
                                KeyCode::Up => {
                                    if state.focused_idx == 16 {
                                        state.focused_idx = 14; // Go to last row, left col
                                    } else if state.focused_idx >= 2 {
                                        state.focused_idx -= 2;
                                    } else {
                                        state.focused_idx = 16;
                                    }
                                }
                                KeyCode::Down => {
                                    if state.focused_idx == 16 {
                                        state.focused_idx = 0;
                                    } else if state.focused_idx < 14 {
                                        state.focused_idx += 2;
                                    } else {
                                        state.focused_idx = 16;
                                    }
                                }
                                KeyCode::Left => {
                                    if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 16;
                                    }
                                }
                                KeyCode::Right => {
                                    if state.focused_idx < 16 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::BackTab => {
                                    if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 16;
                                    }
                                }
                                KeyCode::Tab => {
                                    if state.focused_idx < 16 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::Char(' ') if state.focused_idx == 16 => {
                                    state.optimize_shortest_longest_line = !state.optimize_shortest_longest_line;
                                }
                                _ => {
                                    if state.focused_idx < 16 {
                                        state.inputs[state.focused_idx].input(key);
                                    }
                                }
                            }"""
    
    new_handler = """                            match key.code {
                                KeyCode::Esc => {
                                    app.input_mode = InputMode::Normal;
                                }
                                KeyCode::Enter => {
                                    app.input_mode = InputMode::Normal;
                                    app.start_auto_optimization();
                                }
                                KeyCode::Up => {
                                    if state.focused_idx == 16 {
                                        state.focused_idx = 14;
                                    } else if state.focused_idx == 17 {
                                        state.focused_idx = 16;
                                    } else if state.focused_idx >= 2 {
                                        state.focused_idx -= 2;
                                    } else {
                                        state.focused_idx = 17;
                                    }
                                }
                                KeyCode::Down => {
                                    if state.focused_idx == 16 {
                                        state.focused_idx = 17;
                                    } else if state.focused_idx == 17 {
                                        state.focused_idx = 0;
                                    } else if state.focused_idx < 14 {
                                        state.focused_idx += 2;
                                    } else {
                                        state.focused_idx = 16;
                                    }
                                }
                                KeyCode::Left => {
                                    if state.focused_idx == 17 {
                                        state.batch_size = state.batch_size.saturating_sub(1).max(1);
                                    } else if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 17;
                                    }
                                }
                                KeyCode::Right => {
                                    if state.focused_idx == 17 {
                                        state.batch_size = state.batch_size.saturating_add(1).min(100);
                                    } else if state.focused_idx < 17 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::BackTab => {
                                    if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 17;
                                    }
                                }
                                KeyCode::Tab => {
                                    if state.focused_idx < 17 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::Char(' ') if state.focused_idx == 16 => {
                                    state.optimize_shortest_longest_line = !state.optimize_shortest_longest_line;
                                }
                                KeyCode::Char('-') | KeyCode::Char('_') if state.focused_idx == 17 => {
                                    state.batch_size = state.batch_size.saturating_sub(1).max(1);
                                }
                                KeyCode::Char('+') | KeyCode::Char('=') if state.focused_idx == 17 => {
                                    state.batch_size = state.batch_size.saturating_add(1).min(100);
                                }
                                _ => {
                                    if state.focused_idx < 16 {
                                        state.inputs[state.focused_idx].input(key);
                                    }
                                }
                            }"""
    content = content.replace(old_handler, new_handler)

    # 5. Fix dialog rendering
    old_render = """        // 8 rows of inputs + 1 row for checkbox
        let mut constraints = vec![Constraint::Length(3); 8];
        constraints.push(Constraint::Length(3));
        let rows = Layout::vertical(constraints).split(inner_area);"""

    new_render = """        // 8 rows of inputs + 1 row for checkbox + 1 row for slider
        let mut constraints = vec![Constraint::Length(3); 8];
        constraints.push(Constraint::Length(3));
        constraints.push(Constraint::Length(3));
        let rows = Layout::vertical(constraints).split(inner_area);"""
    
    content = content.replace(old_render, new_render)

    old_render_end = """        let cb_paragraph = Paragraph::new(cb_text).style(cb_style).alignment(Alignment::Center);
        f.render_widget(cb_paragraph, cb_area);
    }
}"""
    new_render_end = """        let cb_paragraph = Paragraph::new(cb_text).style(cb_style).alignment(Alignment::Center);
        f.render_widget(cb_paragraph, cb_area);

        // Render slider for batch size
        let slider_area = rows[9];
        let slider_style = if state.focused_idx == 17 {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(Color::Gray)
        };
        let slider_text = format!("Batch Size: {} (< / > to adjust)", state.batch_size);
        let slider_p = Paragraph::new(slider_text).style(slider_style).alignment(Alignment::Center);
        f.render_widget(slider_p, slider_area);
    }
}"""
    content = content.replace(old_render_end, new_render_end)

    # 6. Change height constraint from 31 to 34
    old_layout = """        let vert = Layout::vertical([
            Constraint::Percentage(20),
            Constraint::Length(31),
            Constraint::Percentage(20),
        ])"""
    new_layout = """        let vert = Layout::vertical([
            Constraint::Percentage(20),
            Constraint::Length(34),
            Constraint::Percentage(20),
        ])"""
    content = content.replace(old_layout, new_layout)

    with open('src/tui.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
