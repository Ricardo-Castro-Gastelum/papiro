use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::Theme;

#[derive(Clone, Copy)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

pub fn fence_states(lines: &[String]) -> Vec<bool> {
    let mut inside = false;
    lines
        .iter()
        .map(|line| {
            let before = inside;
            if is_fence(line) {
                inside = !inside;
            }
            before
        })
        .collect()
}

fn is_fence(line: &str) -> bool {
    let text = line.trim_start();
    text.starts_with("```") || text.starts_with("~~~")
}

fn fence_length(text: &str) -> usize {
    let marker = text.chars().next().unwrap_or(' ');
    text.chars()
        .take_while(|character| *character == marker)
        .count()
}

fn grapheme_index(offsets: &[usize], byte: usize) -> usize {
    offsets.partition_point(|offset| *offset < byte)
}

fn is_rule(text: &str) -> bool {
    let trimmed = text.trim_end_matches([' ', '\t']);
    let mut characters = trimmed.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    matches!(first, '-' | '*' | '_')
        && trimmed.chars().count() >= 3
        && trimmed.chars().all(|character| character == first)
}

fn list_marker(text: &str) -> Option<usize> {
    let leading = text
        .char_indices()
        .find(|(_, character)| !character.is_whitespace())
        .map_or(text.len(), |(index, _)| index);
    let leading_count = text[..leading].chars().count();
    let after = &text[leading..];

    let mut characters = after.chars();
    let first = characters.next()?;
    if matches!(first, '-' | '+' | '*') {
        if characters.next().is_some_and(char::is_whitespace) {
            return Some(leading_count + 2);
        }
        return None;
    }

    if first.is_ascii_digit() {
        let digits = after
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .count();
        let rest = &after[digits..];
        let mut characters = rest.chars();
        if matches!(characters.next(), Some('.') | Some(')'))
            && characters.next().is_some_and(char::is_whitespace)
        {
            return Some(leading_count + digits + 2);
        }
    }

    None
}

fn inline(
    rest: &str,
    byte: usize,
    offsets: &[usize],
    current: usize,
    theme: &Theme,
) -> Option<(usize, Style)> {
    if rest.starts_with('`') {
        let run = rest
            .chars()
            .take_while(|character| *character == '`')
            .count();
        let needle = "`".repeat(run);
        if let Some(relative) = rest[run..].find(&needle) {
            let end = grapheme_index(offsets, byte + run + relative + run);
            if end > current {
                return Some((end - current, theme.code()));
            }
        }
    }

    if rest.starts_with('[')
        && let Some(close) = rest.find("](")
        && let Some(end_relative) = rest[close + 2..].find(')')
    {
        let end = grapheme_index(offsets, byte + close + 2 + end_relative + 1);
        if end > current {
            return Some((end - current, theme.link()));
        }
    }

    if rest.starts_with('<')
        && let Some(end_relative) = rest.find('>')
    {
        let end = grapheme_index(offsets, byte + end_relative + 1);
        if end > current {
            return Some((end - current, theme.html()));
        }
    }

    for marker in ["**", "__", "~~", "*", "_"] {
        if rest.starts_with(marker) {
            return Some((marker.chars().count(), theme.emphasis()));
        }
    }

    if rest.starts_with('|') {
        return Some((1, theme.dim()));
    }

    None
}

pub fn highlight(line: &str, in_fence: bool, theme: &Theme) -> Vec<Segment> {
    let offsets = line
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let count = offsets.len();
    if count == 0 {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut index = 0;
    while index < count {
        let byte = offsets[index];
        let rest = &line[byte..];
        let mut advance = 1;
        let style;

        if in_fence {
            style = theme.code();
        } else if index == 0 {
            let hashes = rest
                .chars()
                .take_while(|character| *character == '#')
                .count();
            let heading = hashes > 0
                && hashes <= 6
                && rest[hashes..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace);

            if heading {
                advance = count;
                style = theme.heading(hashes);
            } else if is_rule(rest) {
                advance = count;
                style = theme.rule();
            } else if rest.starts_with("```") || rest.starts_with("~~~") {
                advance = fence_length(rest);
                style = theme.code();
            } else if let Some(after_quote) = rest.strip_prefix('>') {
                advance = if after_quote.starts_with(' ') { 2 } else { 1 };
                style = theme.quote();
            } else if let Some(marker) = list_marker(rest) {
                advance = marker.min(count);
                style = theme.bullet();
            } else if let Some((length, found)) = inline(rest, byte, &offsets, index, theme) {
                advance = length;
                style = found;
            } else {
                style = Style::default();
            }
        } else if let Some((length, found)) = inline(rest, byte, &offsets, index, theme) {
            advance = length;
            style = found;
        } else {
            style = Style::default();
        }

        segments.push(Segment {
            start: index,
            end: (index + advance).min(count),
            style,
        });
        index += advance;
    }

    merge(segments)
}

fn merge(segments: Vec<Segment>) -> Vec<Segment> {
    let mut result: Vec<Segment> = Vec::new();
    for segment in segments {
        if segment.start >= segment.end {
            continue;
        }
        if let Some(last) = result.last_mut()
            && last.end == segment.start
            && last.style == segment.style
        {
            last.end = segment.end;
            continue;
        }
        result.push(segment);
    }
    result
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;
    use unicode_segmentation::UnicodeSegmentation;

    use super::{Segment, fence_states, highlight};
    use crate::theme::Theme;

    fn covers(line: &str, segments: &[Segment]) -> bool {
        let count = line.graphemes(true).count();
        let mut cursor = 0;
        for segment in segments {
            if segment.start != cursor || segment.end <= segment.start {
                return false;
            }
            cursor = segment.end;
        }
        cursor == count
    }

    #[test]
    fn cubre_toda_la_linea_sin_saltos() {
        let theme = Theme::ansi();
        for line in [
            "# Título",
            "- uno **dos** `tres`",
            "> cita [enlace](https://ejemplo.com)",
            "1. primero | segundo",
            "---",
            "á🐈 *cursiva*",
            "```rust",
        ] {
            assert!(covers(line, &highlight(line, false, &theme)), "{line}");
        }
    }

    #[test]
    fn colorea_encabezados_y_listas() {
        let theme = Theme::ansi();
        assert_eq!(
            highlight("# Hola", false, &theme)[0].style.fg,
            Some(ratatui::style::Color::Cyan)
        );
        assert_eq!(
            highlight("## Hola", false, &theme)[0].style.fg,
            Some(ratatui::style::Color::Blue)
        );
        assert_eq!(
            highlight("- hola", false, &theme)[0].style.fg,
            Some(ratatui::style::Color::Green)
        );
        assert_eq!(
            highlight("1. hola", false, &theme)[0].style.fg,
            Some(ratatui::style::Color::Green)
        );
    }

    #[test]
    fn colorea_codigo_enlaces_y_enfasis() {
        let theme = Theme::ansi();
        let code = highlight("usa `ppr` aquí", false, &theme);
        assert!(
            code.iter()
                .any(|segment| segment.style.fg == Some(ratatui::style::Color::Yellow))
        );

        let link = highlight("[texto](url)", false, &theme);
        assert!(
            link.iter()
                .any(|segment| segment.style.fg == Some(ratatui::style::Color::Blue))
        );

        let bold = highlight("**texto**", false, &theme);
        assert!(
            bold.iter()
                .any(|segment| segment.style.fg == Some(ratatui::style::Color::Magenta))
        );
    }

    #[test]
    fn colorea_el_contenido_de_las_cercas() {
        let theme = Theme::ansi();
        let states = fence_states(&[
            "```".to_owned(),
            "fn main() {}".to_owned(),
            "```".to_owned(),
        ]);
        assert_eq!(states, vec![false, true, true]);

        let inside = highlight("fn main() {}", true, &theme);
        assert_eq!(inside.len(), 1);
        assert_eq!(inside[0].style.fg, Some(ratatui::style::Color::Yellow));
    }

    #[test]
    fn respeta_los_limites_de_grafemas() {
        let theme = Theme::ansi();
        let segments = highlight("á🐈 `x`", false, &theme);
        assert!(covers("á🐈 `x`", &segments));
        assert!(
            segments
                .iter()
                .any(|segment| segment.start <= 2 && segment.end >= 3)
        );
    }

    #[test]
    fn no_color_no_pone_color_pero_mantiene_atributos() {
        let theme = Theme::plain();
        let segments = highlight("# Hola", false, &theme);
        assert!(segments.iter().all(|segment| segment.style.fg.is_none()));
        assert!(
            segments
                .iter()
                .any(|segment| segment.style.add_modifier.contains(Modifier::BOLD))
        );
    }
}
