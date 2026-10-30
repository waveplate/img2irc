use super::*;

const RESIZE_MESSAGE: &str = "INCREASE WINDOW SIZE FOR BEST VIEWING EXPERIENCE";

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ResizeNotice {
    width: u16,
    top: u16,
    lines: Vec<String>,
}

impl ResizeNotice {
    pub fn write(&self, packet: &mut Vec<u8>) -> io::Result<()> {
        for (index, line) in self.lines.iter().enumerate() {
            let row = usize::from(self.top) + index + 1;
            let column = 1 + (usize::from(self.width) - line.len()) / 2;
            // Use absolute positioning without a newline at the terminal's
            // bottom edge, where a line feed would scroll/cut off the video.
            write!(
                packet,
                "\x1b[0m\x1b[40m\x1b[{row};1H\x1b[2K\x1b[{row};{column}H\x1b[1;33m{line}"
            )?;
        }
        Ok(())
    }
}

pub(super) struct FrameLayout {
    pub columns: u16,
    pub rows: u16,
    pub notice: Option<ResizeNotice>,
}

pub(super) fn layout(header: &Header, viewport: Option<(u16, u16)>, fit: bool) -> FrameLayout {
    let Some((width, height)) = viewport
        .map(|(w, h)| (w.max(1), h.max(1)))
        .filter(|&(w, h)| header.width > w || header.height > h)
    else {
        return FrameLayout {
            columns: header.width,
            rows: header.height,
            notice: None,
        };
    };
    let notice = fit.then(|| {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in RESIZE_MESSAGE.split_ascii_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > usize::from(width) {
                lines.push(std::mem::take(&mut line));
            }
            // The message is ASCII; even one-column terminals are safe.
            let mut chunks = word.as_bytes().chunks(usize::from(width)).peekable();
            while let Some(chunk) = chunks.next() {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(std::str::from_utf8(chunk).expect("ASCII resize message"));
                if chunks.peek().is_some() {
                    lines.push(std::mem::take(&mut line));
                }
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines.truncate(usize::from(height.saturating_sub(1).max(1)));
        ResizeNotice {
            width,
            top: height - lines.len() as u16,
            lines,
        }
    });
    let available_height = notice.as_ref().map_or(height, |notice| notice.top);
    let bounds = (width.min(header.width), available_height.min(header.height));
    let (columns, rows) = if available_height == 0 {
        (0, 0)
    } else if fit {
        fit_size(f64::from(header.width) / f64::from(header.height), bounds)
    } else {
        bounds
    };
    FrameLayout {
        columns,
        rows,
        notice,
    }
}

/// One baseline per viewer, updated only for frames actually presented.
/// Source files remain independently decodable, so late frames can be skipped.
#[derive(Default)]
pub(super) struct Presenter {
    previous: Option<Buffer>,
    viewport: Option<(u16, u16)>,
    notice: Option<ResizeNotice>,
}

impl Presenter {
    pub fn prepare(
        &mut self,
        frame: &[u8],
        header: &Header,
        viewport: Option<(u16, u16)>,
        fit: bool,
        clear: bool,
    ) -> Result<Vec<u8>> {
        let layout = layout(header, viewport, fit);
        // Layout changes must remove a previous bottom-row notice, including
        // when the fitted video dimensions happen to stay the same.
        let clear = clear
            || (self.previous.is_some()
                && (self.viewport != viewport || self.notice != layout.notice));
        let content = std::str::from_utf8(frame)?;
        let content = content.strip_prefix("\x1b[?2026h\x1b[H").unwrap_or(content);
        let content = content
            .strip_suffix("\x1b[0m\x1b[?2026l")
            .unwrap_or(content);
        let mut source = Buffer::empty(Rect::new(0, 0, header.width, header.height));
        Paragraph::new(content.into_text()?).render(source.area, &mut source);
        let target = if layout.columns != header.width || layout.rows != header.height {
            let (columns, rows) = (layout.columns, layout.rows);
            let mut target = Buffer::empty(Rect::new(0, 0, columns, rows));
            for y in 0..rows {
                for x in 0..columns {
                    let sx = if fit {
                        sample_cell(x, columns, header.width)
                    } else {
                        x
                    };
                    let sy = if fit {
                        sample_cell(y, rows, header.height)
                    } else {
                        y
                    };
                    target[(x, y)] = source[(sx, sy)].clone();
                }
            }
            target
        } else {
            source
        };
        let packet = if let Some(previous) = self
            .previous
            .as_ref()
            .filter(|p| !clear && p.area == target.area)
        {
            let updates = previous.diff(&target);
            if updates.is_empty() {
                Vec::new()
            } else {
                let mut delta = b"\x1b[?2026h".to_vec();
                crossterm::style::force_color_output(true);
                CrosstermBackend::new(&mut delta).draw(updates.into_iter())?;
                delta.write_all(b"\x1b[0m\x1b[?2026l")?;
                // Keep dense updates as compact as the old complete redraw.
                if delta.len() > frame.len() / 2 {
                    let full = prepare_frame(frame, header, viewport, fit, false)?;
                    if full.len() < delta.len() {
                        full
                    } else {
                        delta
                    }
                } else {
                    delta
                }
            }
        } else {
            prepare_frame(frame, header, viewport, fit, clear)?
        };
        self.previous = Some(target);
        self.viewport = viewport;
        self.notice = layout.notice;
        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_frames_need_no_bytes_and_resize_forces_a_redraw() {
        let header = Header {
            format: "img2irc-video".into(),
            version: 1,
            width: 40,
            height: 3,
            fps: 24.0,
        };
        let frame = format!(
            "\x1b[?2026h\x1b[H{}\x1b[0m\x1b[?2026l",
            vec!["A".repeat(40); 3].join("\r\n")
        );
        let mut presenter = Presenter::default();
        assert!(!presenter
            .prepare(frame.as_bytes(), &header, None, true, false)
            .unwrap()
            .is_empty());
        assert!(presenter
            .prepare(frame.as_bytes(), &header, None, true, false)
            .unwrap()
            .is_empty());
        let changed = frame.replacen('A', "Z", 1);
        let delta = presenter
            .prepare(changed.as_bytes(), &header, None, true, false)
            .unwrap();
        assert!(delta.len() < frame.len() / 2);
        assert!(delta.contains(&b'Z'));
        let resized = presenter
            .prepare(changed.as_bytes(), &header, Some((20, 2)), true, true)
            .unwrap();
        assert!(resized.windows(4).any(|w| w == b"\x1b[2J"));
    }
}
