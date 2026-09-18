use figlet_rs::FIGlet;

/// Preserve word separators even in fonts whose kerning consumes space glyphs.
/// Words keep the font's own kerning and baseline; gaps occupy real columns and
/// therefore participate in all subsequent width checks.
pub fn render(font: &FIGlet, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.contains('\n') {
        let mut lines = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                lines.push(String::new());
                continue;
            }
            let rendered = render(font, line)?;
            let rows: Vec<_> = rendered.lines().collect();
            let top = rows.iter().position(|row| !row.trim().is_empty())?;
            let bottom = rows.iter().rposition(|row| !row.trim().is_empty())?;
            lines.push(rows[top..=bottom].join("\n"));
        }
        return Some(lines.join("\n\n"));
    }
    let render_word = |word: &str| {
        let figure = font.convert(word)?;
        (figure.characters.len() == word.chars().count()).then(|| figure.as_str())
    };
    if !text.chars().any(char::is_whitespace) {
        return render_word(text);
    }

    let space_width = font.fonts.get(&32).map_or(1, |glyph| glyph.width.max(1)) as usize;
    let mut output = vec![String::new(); font.header_line.height as usize];
    let mut has_word = false;
    let mut gap = 0;
    for part in text.split_inclusive(char::is_whitespace) {
        let word = part.trim_end();
        if !word.is_empty() {
            let art = render_word(word)?;
            let rows: Vec<Vec<char>> = art.lines().map(|line| line.chars().collect()).collect();
            let left = rows
                .iter()
                .filter_map(|row| row.iter().position(|c| !c.is_whitespace()))
                .min()?;
            let right = rows
                .iter()
                .filter_map(|row| row.iter().rposition(|c| !c.is_whitespace()))
                .max()?
                + 1;
            if rows.len() != output.len() {
                return None;
            }
            for (line, row) in output.iter_mut().zip(rows) {
                if has_word {
                    line.push_str(&" ".repeat(gap * space_width));
                }
                for col in left..right {
                    line.push(row.get(col).copied().unwrap_or(' '));
                }
            }
            has_word = true;
            gap = 0;
        }
        gap += part.chars().rev().take_while(|c| c.is_whitespace()).count();
    }
    has_word.then(|| output.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_columns(art: &str) -> usize {
        let rows: Vec<Vec<char>> = art.lines().map(|row| row.chars().collect()).collect();
        (0..rows.iter().map(Vec::len).max().unwrap_or(0))
            .filter(|&x| {
                rows.iter()
                    .all(|row| row.get(x).is_none_or(|c| c.is_whitespace()))
            })
            .count()
    }

    #[test]
    fn spaces_survive_even_when_the_font_defines_a_zero_width_space() {
        let mut font = FIGlet::small().unwrap();
        let space = font.fonts.get_mut(&32).unwrap();
        space.width = 0;
        for row in &mut space.characters {
            row.clear();
        }
        let compact = render(&font, "ONETWOTHREE").unwrap();
        let spaced = render(&font, "ONE TWO THREE").unwrap();
        assert!(blank_columns(&spaced) >= blank_columns(&compact) + 2);
        let double = render(&font, "ONE  TWO THREE").unwrap();
        assert_eq!(
            double.lines().next().unwrap().chars().count(),
            spaced.lines().next().unwrap().chars().count() + 1
        );
    }

    #[test]
    fn multiline_text_uses_one_font_and_retains_line_breaks() {
        let font = FIGlet::small().unwrap();
        let art = render(&font, "ONE TWO\nTHREE").unwrap();
        let lines: Vec<_> = art.lines().collect();
        assert!(lines.len() > font.header_line.height as usize);
        assert!(lines.iter().any(|line| line.is_empty()));
        assert!(art.contains("\n\n"));
    }

    #[test]
    fn single_words_keep_existing_font_layout_and_unsupported_text_fails() {
        let font = FIGlet::small().unwrap();
        assert_eq!(
            render(&font, "HELLO").unwrap(),
            font.convert("HELLO").unwrap().as_str()
        );
        assert!(render(&font, "ONE \u{10ffff}").is_none());
    }
}
