use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const QUOTE_PREFIX: &str = "│ ";
const CODE_INDENT: &str = "  ";
const BULLETS: [&str; 3] = ["• ", "◦ ", "▪ "];
const IMAGE_LABEL: &str = "[imagen: ";
const TAB_SPACES: usize = 4;

pub fn render(text: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    let mut builder = Builder::new(width);
    for event in Parser::new_ext(text, options) {
        builder.handle(event);
    }
    builder.finish()
}

#[derive(Clone)]
struct Chunk {
    text: String,
    style: Style,
}

impl Chunk {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    fn newline() -> Self {
        Self::new("\n", Style::default())
    }

    fn is_newline(&self) -> bool {
        self.text == "\n"
    }

    fn width(&self) -> usize {
        if self.is_newline() {
            0
        } else {
            UnicodeWidthStr::width(self.text.as_str())
        }
    }
}

#[derive(Default)]
struct InlineStyles {
    bold: usize,
    italic: usize,
    strike: usize,
    link: usize,
    image: usize,
}

struct ListState {
    next: Option<u64>,
    marker: String,
    at_start: bool,
    emitted: bool,
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: String,
    head_rows: usize,
}

struct Builder {
    width: usize,
    lines: Vec<Line<'static>>,
    content: Vec<Chunk>,
    heading: Option<HeadingLevel>,
    quote_depth: usize,
    lists: Vec<ListState>,
    inline: InlineStyles,
    code_block: bool,
    code_text: String,
    table: Option<Table>,
    task_indent: usize,
}

impl Builder {
    fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            content: Vec::new(),
            heading: None,
            quote_depth: 0,
            lists: Vec::new(),
            inline: InlineStyles::default(),
            code_block: false,
            code_text: String::new(),
            table: None,
            task_indent: 0,
        }
    }

    fn handle(&mut self, event: Event<'_>) {
        if self.table.is_some() {
            self.handle_table_event(&event);
            return;
        }

        if self.code_block {
            match event {
                Event::Text(text) => {
                    self.code_text.push_str(&text);
                    return;
                }
                Event::End(TagEnd::CodeBlock) => {
                    self.finish_code_block();
                    return;
                }
                _ => {}
            }
        }

        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                let style = self.current_style();
                self.push_text(&text, style);
            }
            Event::Code(text) => {
                let style = self.current_style().add_modifier(Modifier::DIM);
                self.push_text(&text, style);
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                self.push_text(&text, dim());
            }
            Event::SoftBreak => {
                let style = self.current_style();
                self.push_text(" ", style);
            }
            Event::HardBreak => self.content.push(Chunk::newline()),
            Event::Rule => {
                self.separate();
                let rule = Chunk::new("─".repeat(self.width), dim());
                self.lines.push(make_line(&[], &[rule]));
            }
            Event::FootnoteReference(name) => {
                let style = self.current_style().add_modifier(Modifier::DIM);
                self.push_text(&format!("[{name}]"), style);
            }
            Event::TaskListMarker(checked) => {
                self.task_indent = 4;
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_text(marker, dim());
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let style = self.current_style().add_modifier(Modifier::DIM);
                self.push_text(&text, style);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.lists.is_empty() {
                    self.separate();
                }
            }
            Tag::Heading { level, .. } => {
                self.separate();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.separate();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                self.separate();
                self.code_block = true;
                self.code_text.clear();
            }
            Tag::HtmlBlock => self.separate(),
            Tag::List(number) => {
                self.flush(false);
                if self.lists.is_empty() {
                    self.separate();
                } else {
                    if self.lists.last().is_some_and(|parent| !parent.emitted) {
                        self.flush(true);
                    }
                    if let Some(parent) = self.lists.last_mut() {
                        parent.at_start = false;
                    }
                }
                self.lists.push(ListState {
                    next: number,
                    marker: String::new(),
                    at_start: false,
                    emitted: false,
                });
            }
            Tag::Item => {
                self.flush(false);
                self.task_indent = 0;
                let depth = self.lists.len().saturating_sub(1);

                if let Some(list) = self.lists.last_mut() {
                    list.marker = match list.next {
                        Some(number) => {
                            list.next = Some(number + 1);
                            format!("{number}. ")
                        }
                        None => BULLETS[depth % BULLETS.len()].to_owned(),
                    };
                    list.at_start = true;
                    list.emitted = false;
                }
            }
            Tag::FootnoteDefinition(name) => {
                self.separate();
                self.push_text(&format!("[^{name}]: "), dim());
            }
            Tag::Table(_) => {
                self.separate();
                self.table = Some(Table::default());
            }
            Tag::Emphasis => self.inline.italic += 1,
            Tag::Strong => self.inline.bold += 1,
            Tag::Strikethrough => self.inline.strike += 1,
            Tag::Link { .. } => self.inline.link += 1,
            Tag::Image { .. } => {
                self.inline.image += 1;
                let style = self.current_style();
                self.push_text(IMAGE_LABEL, style);
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(false),
            TagEnd::Heading(_) => {
                self.flush(false);
                self.heading = None;
            }
            TagEnd::BlockQuote(_) => {
                self.flush(false);
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => self.finish_code_block(),
            TagEnd::HtmlBlock => self.flush(false),
            TagEnd::List(_) => {
                self.flush(false);
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush(true);
                self.task_indent = 0;
            }
            TagEnd::FootnoteDefinition => self.flush(false),
            TagEnd::Table => self.finish_table(),
            TagEnd::TableHead | TagEnd::TableRow | TagEnd::TableCell => {}
            TagEnd::Emphasis => self.inline.italic = self.inline.italic.saturating_sub(1),
            TagEnd::Strong => self.inline.bold = self.inline.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.inline.strike = self.inline.strike.saturating_sub(1),
            TagEnd::Link => self.inline.link = self.inline.link.saturating_sub(1),
            TagEnd::Image => {
                self.inline.image = self.inline.image.saturating_sub(1);
                let style = self.current_style();
                self.push_text("]", style);
            }
            _ => {}
        }
    }

    fn handle_table_event(&mut self, event: &Event<'_>) {
        let Some(table) = &mut self.table else {
            return;
        };

        match event {
            Event::Start(Tag::TableHead) => table.row.clear(),
            Event::End(TagEnd::TableHead) => {
                if !table.row.is_empty() {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
                table.head_rows = table.rows.len();
            }
            Event::Start(Tag::TableRow) => table.row.clear(),
            Event::End(TagEnd::TableRow) => {
                let row = std::mem::take(&mut table.row);
                table.rows.push(row);
            }
            Event::Start(Tag::TableCell) => table.cell.clear(),
            Event::End(TagEnd::TableCell) => {
                let cell = std::mem::take(&mut table.cell);
                table.row.push(cell.trim().to_owned());
            }
            Event::Text(text) | Event::Code(text) => table.cell.push_str(text),
            Event::SoftBreak | Event::HardBreak => table.cell.push(' '),
            Event::End(TagEnd::Table) => self.finish_table(),
            _ => {}
        }
    }

    fn finish_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        if table.rows.is_empty() {
            return;
        }

        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }

        let mut widths = vec![0; columns];
        for row in &table.rows {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }

        let separators = 3 * columns.saturating_sub(1);
        let total: usize = widths.iter().sum::<usize>() + separators;
        let aligned = total <= self.width;

        for (index, row) in table.rows.iter().enumerate() {
            let header = index < table.head_rows;
            let style = if header {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let mut text = String::new();
            for (column, width) in widths.iter().enumerate() {
                if column > 0 {
                    text.push_str(" │ ");
                }

                let cell = row.get(column).map_or("", String::as_str);
                text.push_str(cell);
                if aligned {
                    let padding = width.saturating_sub(UnicodeWidthStr::width(cell));
                    text.push_str(&" ".repeat(padding));
                }
            }

            self.push_text(text.trim_end(), style);
            self.flush(false);

            if header {
                let rule_width = if aligned { total } else { self.width };
                let rule = Chunk::new("─".repeat(rule_width), dim());
                self.lines.push(make_line(&[], &[rule]));
            }
        }

        self.separate();
    }

    fn finish_code_block(&mut self) {
        let text = std::mem::take(&mut self.code_text);

        let text = text.strip_suffix('\n').unwrap_or(&text);
        if text.is_empty() {
            self.code_block = false;
            return;
        }

        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.content.push(Chunk::newline());
            }
            self.push_text(line, dim());
        }

        self.flush(false);
        self.code_block = false;
    }

    fn separate(&mut self) {
        self.flush(false);

        if self.lines.is_empty() || !self.lists.is_empty() {
            return;
        }
        if self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            return;
        }

        self.lines.push(Line::default());
    }

    fn current_style(&self) -> Style {
        let mut style = match self.heading {
            Some(HeadingLevel::H1) => {
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            }
            Some(_) => Style::default().add_modifier(Modifier::BOLD),
            None => Style::default(),
        };

        if self.inline.bold > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.inline.italic > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.inline.strike > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.inline.link > 0 {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if self.inline.image > 0 {
            style = style.add_modifier(Modifier::DIM | Modifier::ITALIC);
        }

        style
    }

    fn push_text(&mut self, text: &str, style: Style) {
        for grapheme in text.graphemes(true) {
            if grapheme == "\n" {
                self.content.push(Chunk::newline());
            } else if grapheme == "\t" {
                for _ in 0..TAB_SPACES {
                    self.content.push(Chunk::new(" ", style));
                }
            } else if grapheme.chars().any(char::is_control) {
                self.content.push(Chunk::new("\u{FFFD}", style));
            } else {
                self.content.push(Chunk::new(grapheme, style));
            }
        }
    }

    fn prefixes(&self) -> (Vec<Chunk>, Vec<Chunk>) {
        let mut first = Vec::new();
        let mut continuation = Vec::new();

        if self.quote_depth > 0 {
            let quote = QUOTE_PREFIX.repeat(self.quote_depth);
            push_chunks(&mut first, &quote, dim());
            push_chunks(&mut continuation, &quote, dim());
        }

        if let Some(list) = self.lists.last() {
            let indent = "  ".repeat(self.lists.len().saturating_sub(1));
            let indent_width = UnicodeWidthStr::width(indent.as_str());
            let marker_width = UnicodeWidthStr::width(list.marker.as_str());

            push_chunks(&mut first, &indent, Style::default());
            if list.at_start {
                push_chunks(&mut first, &list.marker, dim());
            } else {
                push_chunks(
                    &mut first,
                    &" ".repeat(indent_width + marker_width),
                    Style::default(),
                );
            }

            push_chunks(
                &mut continuation,
                &" ".repeat(indent_width + marker_width + self.task_indent),
                Style::default(),
            );
        }

        if self.code_block {
            push_chunks(&mut first, CODE_INDENT, dim());
            push_chunks(&mut continuation, CODE_INDENT, dim());
        }

        (first, continuation)
    }

    fn flush(&mut self, force: bool) {
        if self.content.is_empty() && !force {
            return;
        }
        if self.content.is_empty() && force && self.lists.last().is_some_and(|list| list.emitted) {
            return;
        }

        let had_content = !self.content.is_empty();
        let (prefix, continuation) = self.prefixes();

        if had_content {
            let content = std::mem::take(&mut self.content);
            self.lines
                .extend(wrap(&content, self.width, &prefix, &continuation));
        } else if force {
            self.lines.push(make_line(&prefix, &[]));
        }

        if let Some(list) = self.lists.last_mut() {
            list.at_start = false;
            list.emitted = true;
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush(false);

        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }

        self.lines
    }
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn push_chunks(target: &mut Vec<Chunk>, text: &str, style: Style) {
    for grapheme in text.graphemes(true) {
        target.push(Chunk::new(grapheme, style));
    }
}

fn chunk_width(chunks: &[Chunk]) -> usize {
    chunks.iter().map(Chunk::width).sum()
}

fn wrap(
    content: &[Chunk],
    width: usize,
    prefix: &[Chunk],
    continuation: &[Chunk],
) -> Vec<Line<'static>> {
    let mut result = Vec::new();
    let mut first = true;

    let mut segments = content
        .split(|chunk| chunk.is_newline())
        .collect::<Vec<_>>();
    if content.last().is_some_and(Chunk::is_newline) {
        segments.pop();
    }
    if segments.is_empty() {
        segments.push(&[]);
    }

    for segment in segments {
        let segment_prefix = if first { prefix } else { continuation };
        let first_limit = width.saturating_sub(chunk_width(segment_prefix)).max(1);
        let rest_limit = width.saturating_sub(chunk_width(continuation)).max(1);

        let wrapped = if first {
            wrap_segment(segment, first_limit, rest_limit)
        } else {
            wrap_segment(segment, rest_limit, rest_limit)
        };

        for line in wrapped {
            let line_prefix = if first { prefix } else { continuation };
            result.push(make_line(line_prefix, &line));
            first = false;
        }
    }

    result
}

fn wrap_segment(segment: &[Chunk], first_limit: usize, rest_limit: usize) -> Vec<Vec<Chunk>> {
    if segment.is_empty() {
        return vec![Vec::new()];
    }

    let mut lines = Vec::new();
    let mut current: Vec<Chunk> = Vec::new();
    let mut current_width = 0;
    let mut limit = first_limit;
    let mut last_space = None;

    for chunk in segment {
        let chunk_width = chunk.width();

        if current_width + chunk_width > limit && !current.is_empty() {
            if let Some(index) = last_space.take() {
                let mut rest = current.split_off(index);
                if rest.first().is_some_and(|chunk| chunk.text == " ") {
                    rest.remove(0);
                }
                trim_trailing_spaces(&mut current);
                lines.push(std::mem::take(&mut current));
                current = rest;
                current_width = chunk_width_of(&current);
                limit = rest_limit;
            } else {
                trim_trailing_spaces(&mut current);
                lines.push(std::mem::take(&mut current));
                current_width = 0;
                limit = rest_limit;
            }
        }

        if chunk.text == " " {
            last_space = Some(current.len());
        }

        current_width += chunk_width;
        current.push(chunk.clone());
    }

    if !current.is_empty() {
        trim_trailing_spaces(&mut current);
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }

    lines
}

fn trim_trailing_spaces(chunks: &mut Vec<Chunk>) {
    while chunks.last().is_some_and(|chunk| chunk.text == " ") {
        chunks.pop();
    }
}

fn chunk_width_of(chunks: &[Chunk]) -> usize {
    chunks.iter().map(Chunk::width).sum()
}

fn make_line(prefix: &[Chunk], content: &[Chunk]) -> Line<'static> {
    let mut spans = Vec::new();
    let mut pending: Option<(String, Style)> = None;

    for chunk in prefix.iter().chain(content) {
        if chunk.is_newline() {
            continue;
        }

        match &mut pending {
            Some((text, style)) if *style == chunk.style => text.push_str(&chunk.text),
            _ => {
                if let Some((text, style)) = pending.take() {
                    spans.push(Span::styled(text, style));
                }
                pending = Some((chunk.text.clone(), chunk.style));
            }
        }
    }

    if let Some((text, style)) = pending {
        spans.push(Span::styled(text, style));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;
    use ratatui::text::Line;
    use unicode_width::UnicodeWidthStr;

    use super::render;

    fn plain(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn has_modifier(line: &Line<'_>, modifier: Modifier) -> bool {
        line.spans
            .iter()
            .any(|span| span.style.add_modifier.contains(modifier))
    }

    #[test]
    fn renderiza_encabezados_sin_marcas() {
        let lines = render("# Título\n\ntexto", 40);

        assert_eq!(plain(&lines[0]), "Título");
        assert!(has_modifier(&lines[0], Modifier::BOLD));
        assert!(has_modifier(&lines[0], Modifier::UNDERLINED));
        assert_eq!(plain(&lines[1]), "");
        assert_eq!(plain(&lines[2]), "texto");
    }

    #[test]
    fn aplica_estilos_en_linea() {
        let lines = render(
            "**negrita** *cursiva* ~~tachado~~ `código` [enlace](https://ejemplo.com)",
            80,
        );

        assert!(has_modifier(&lines[0], Modifier::BOLD));
        assert!(has_modifier(&lines[0], Modifier::ITALIC));
        assert!(has_modifier(&lines[0], Modifier::CROSSED_OUT));
        assert!(has_modifier(&lines[0], Modifier::DIM));
        assert!(has_modifier(&lines[0], Modifier::UNDERLINED));
        assert!(!plain(&lines[0]).contains("https://ejemplo.com"));
    }

    #[test]
    fn separa_parrafos_con_una_linea_en_blanco() {
        let lines = render("uno\n\ndos", 40);

        assert_eq!(lines.len(), 3);
        assert_eq!(plain(&lines[1]), "");
    }

    #[test]
    fn ajusta_las_lineas_al_ancho() {
        let paragraph =
            "Papiro ajusta el texto para que ninguna línea se salga del ancho disponible.";
        let lines = render(paragraph, 18);

        assert!(lines.len() > 1);
        for line in &lines {
            assert!(UnicodeWidthStr::width(plain(line).as_str()) <= 18);
        }
    }

    #[test]
    fn parte_palabras_mas_largas_que_el_ancho() {
        let lines = render("supercalifragilisticoespialidoso", 8);

        assert!(lines.len() > 1);
        for line in &lines {
            assert!(UnicodeWidthStr::width(plain(line).as_str()) <= 8);
        }
    }

    #[test]
    fn alinea_las_continuaciones_de_una_lista() {
        let lines = render("- uno dos tres cuatro cinco", 12);

        assert!(plain(&lines[0]).starts_with("• "));
        for line in lines.iter().skip(1) {
            assert!(plain(line).starts_with("  "));
            assert!(!plain(line).starts_with("• "));
        }
    }

    #[test]
    fn numera_los_elementos_ordenados() {
        let lines = render("1. uno\n2. dos", 40);

        assert!(plain(&lines[0]).starts_with("1. "));
        assert!(plain(&lines[1]).starts_with("2. "));
    }

    #[test]
    fn no_deja_lineas_vacias_al_cerrar_listas_anidadas() {
        let lines = render("- uno\n  - dos\n  - tres\n- cuatro", 40);

        for line in &lines {
            assert!(!plain(line).is_empty());
        }
    }

    #[test]
    fn dibuja_citas_con_barra_lateral() {
        let lines = render("> hola mundo", 40);

        assert_eq!(plain(&lines[0]), "│ hola mundo");
    }

    #[test]
    fn muestra_los_bloques_de_codigo_con_sangria() {
        let lines = render("```\nfn main() {}\n```", 40);

        assert_eq!(plain(&lines[0]), "  fn main() {}");
    }

    #[test]
    fn dibuja_tablas_simples() {
        let lines = render("| a | b |\n|---|---|\n| 1 | 2 |", 40);
        assert_eq!(plain(&lines[0]), "a │ b");
        assert!(has_modifier(&lines[0], Modifier::BOLD));
        assert!(plain(&lines[2]).contains("1 │ 2"));
    }

    #[test]
    fn dibuja_reglas_horizontales() {
        let lines = render("---", 10);

        assert_eq!(plain(&lines[0]), "──────────");
    }

    #[test]
    fn convierte_saltos_duros_en_lineas_nuevas() {
        let lines = render("uno  \ndos", 40);

        assert_eq!(plain(&lines[0]), "uno");
        assert_eq!(plain(&lines[1]), "dos");
    }

    #[test]
    fn sustituye_caracteres_de_control() {
        let lines = render("hola\u{1}mundo", 40);

        assert!(plain(&lines[0]).contains('\u{FFFD}'));
        assert!(!plain(&lines[0]).chars().any(char::is_control));
    }

    #[test]
    fn no_produce_lineas_para_un_texto_vacio() {
        assert!(render("", 40).is_empty());
    }
}
