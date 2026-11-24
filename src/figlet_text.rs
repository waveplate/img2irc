use figlet_rs::FIGlet;

/// Preserve word separators even in fonts whose kerning consumes space glyphs.
/// Words keep the font's own kerning and baseline; gaps occupy real columns and
/// therefore participate in all subsequent width checks.
pub fn render(font: &FIGlet, text: &str) -> Option<String> {
    let text = text.trim_end();
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
        return Some(lines.join("\n"));
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
                if gap > 0 {
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

/// Lay out source text in rendered FIGlet columns, preserving explicit indentation.
/// Wrap source words (or long words by character), never the generated art rows.
pub fn render_wrapped(font: &FIGlet, text: &str, width: Option<usize>) -> Option<String> {
    fn block(font: &FIGlet, text: &str) -> Option<Vec<String>> {
        let rendered = render(font, text)?;
        let rows: Vec<Vec<char>> = rendered.lines().map(|row| row.chars().collect()).collect();
        let top = rows.iter().position(|row| row.iter().any(|c| !c.is_whitespace()))?;
        let bottom = rows.iter().rposition(|row| row.iter().any(|c| !c.is_whitespace()))?;
        let left = rows.iter().filter_map(|row| row.iter().position(|c| !c.is_whitespace())).min()?;
        let right = rows.iter().filter_map(|row| row.iter().rposition(|c| !c.is_whitespace())).max()?;
        let spaces = text.chars().take_while(|c| c.is_whitespace()).count();
        let space_width = font.fonts.get(&32).map_or(1, |glyph| glyph.width.max(1)) as usize;
        let left = left.saturating_sub(spaces * space_width);
        Some(rows[top..=bottom].iter().map(|row| (left..=right).map(|x| row.get(x).copied().unwrap_or(' ')).collect()).collect())
    }
    let width = width.filter(|width| *width > 0);
    let mut output = Vec::new();
    for source in text.trim_end().lines() {
        if source.trim().is_empty() { output.push(String::new()); continue; }
        let mut remaining = source.trim_end_matches('\r');
        loop {
            let rows = block(font, remaining)?;
            if width.is_none_or(|width| rows.iter().all(|row| row.chars().count() <= width)) {
                output.extend(rows);
                break;
            }
            let limit = width?;
            let mut end = 0;
            let mut word_break = None;
            for (index, character) in remaining.char_indices() {
                let next = index + character.len_utf8();
                if character.is_whitespace() {
                    if !remaining[..index].trim().is_empty() { word_break = Some(index); }
                    continue;
                }
                let candidate = block(font, &remaining[..next])?;
                if candidate.iter().any(|row| row.chars().count() > limit) { break; }
                end = next;
            }
            let end = word_break.filter(|at| *at <= end).unwrap_or(end);
            if end == 0 { return None; }
            output.extend(block(font, &remaining[..end])?);
            remaining = remaining[end..].trim_start();
            if remaining.is_empty() { break; }
        }
    }
    (!output.is_empty()).then(|| output.join("\n"))
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
        let ink_rows = |text: &str| {
            let art = render(&font, text).unwrap();
            let rows: Vec<_> = art.lines().collect();
            let top = rows.iter().position(|row| !row.trim().is_empty()).unwrap();
            let bottom = rows.iter().rposition(|row| !row.trim().is_empty()).unwrap();
            rows[top..=bottom].join("\n")
        };
        let first = ink_rows("ONE TWO");
        let second = ink_rows("THREE");
        for newlines in 1..=4 {
            let text = format!("ONE TWO{}THREE", "\n".repeat(newlines));
            assert_eq!(render(&font, &text).unwrap(), format!("{first}{}{second}", "\n".repeat(newlines)));
        }
    }

    #[test]
    fn leading_spaces_survive_layout_and_words_wrap_in_rendered_columns() {
        let font = FIGlet::small().unwrap();
        let base = render_wrapped(&font, "HELLO", None).unwrap();
        let indented = render_wrapped(&font, "  HELLO", None).unwrap();
        let space = font.fonts.get(&32).unwrap().width.max(1) as usize;
        assert_eq!(indented, base.lines().map(|row| format!("{}{row}", " ".repeat(2 * space))).collect::<Vec<_>>().join("\n"));
        let first = render_wrapped(&font, "ONE", None).unwrap();
        let second = render_wrapped(&font, "TWO", None).unwrap();
        let width = first.lines().chain(second.lines()).map(|row| row.chars().count()).max().unwrap();
        assert_eq!(render_wrapped(&font, "ONE TWO", Some(width)).unwrap(), format!("{first}\n{second}"));
        assert_eq!(render_wrapped(&font, "ONE\n\nTWO", Some(width)).unwrap(), format!("{first}\n\n{second}"));
        let long = render_wrapped(&font, "ONETWO", Some(width)).unwrap();
        assert!(long.lines().all(|row| row.chars().count() <= width));
        assert!(long.lines().count() > first.lines().count());
        assert!(render_wrapped(&font, "W", Some(1)).is_none());
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
