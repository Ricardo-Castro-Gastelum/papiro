use std::io::Stdout;
use std::path::Path;

use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;

use crate::editor::Editor;
use crate::keys::{Command, KeySpec, Preset, Settings};
use crate::theme::Theme;
use crate::{file, markdown, render};

const QUIT_WARNING: &str = "Cambios sin guardar · Ctrl+S guardar · Ctrl+Q salir sin guardar";
const WHEEL_LINES: isize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Insert,
    Normal,
}

pub struct App {
    pub editor: Editor,
    running: bool,
    prompt: Option<Prompt>,
    message: Option<String>,
    message_error: bool,
    theme: Theme,
    settings: Settings,
    mode: Mode,
    pending: Vec<KeySpec>,
    text_area: Rect,
    view_scroll_ratio: Option<f64>,
    last_save_name: String,
    view: Option<View>,
    search: Option<Search>,
    last_search: String,
    confirm_quit: bool,
}

#[derive(Default)]
struct View {
    lines: Vec<Line<'static>>,
    width: u16,
    rendered: bool,
    scroll: usize,
    viewport_height: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    SaveAs,
    Search,
}

struct Prompt {
    kind: PromptKind,
    input: String,
}

#[derive(Clone, Copy)]
struct SearchMatch {
    line: usize,
    start: usize,
    end: usize,
}

struct Search {
    query: String,
    matches: Vec<SearchMatch>,
    current: Option<usize>,
    origin_line: usize,
    origin_column: usize,
}

impl App {
    #[cfg(test)]
    pub fn new(editor: Editor) -> Self {
        Self::with_theme(editor, Theme::from_env())
    }

    #[cfg(test)]
    pub fn with_theme(editor: Editor, theme: Theme) -> Self {
        Self::with_options(editor, theme, Settings::default())
    }

    pub fn with_settings(editor: Editor, settings: Settings) -> Self {
        Self::with_options(editor, Theme::from_env(), settings)
    }

    pub fn with_options(editor: Editor, theme: Theme, settings: Settings) -> Self {
        let mode = if settings.preset() == Preset::Vim {
            Mode::Normal
        } else {
            Mode::Insert
        };

        Self {
            editor,
            running: true,
            prompt: None,
            message: None,
            message_error: false,
            theme,
            settings,
            mode,
            pending: Vec::new(),
            text_area: Rect::default(),
            view_scroll_ratio: None,
            last_save_name: String::new(),
            view: None,
            search: None,
            last_search: String::new(),
            confirm_quit: false,
        }
    }

    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        while self.running {
            terminal.draw(|frame| render::draw(frame, self))?;

            match event::read().context("No se pudo leer un evento de la terminal")? {
                Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
                Event::Mouse(mouse) => self.handle_mouse(mouse),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        Ok(())
    }

    pub fn set_text_area(&mut self, area: Rect) {
        self.text_area = area;
    }

    pub fn mode_label(&self) -> &'static str {
        if self.view.is_some() || self.settings.preset() != Preset::Vim {
            return "";
        }
        if self.mode == Mode::Normal {
            "NORMAL"
        } else {
            "INSERT"
        }
    }

    pub fn is_vim(&self) -> bool {
        self.settings.preset() == Preset::Vim
    }

    pub fn is_insert_mode(&self) -> bool {
        self.mode == Mode::Insert
    }

    pub fn prompt_input(&self) -> Option<&str> {
        self.prompt.as_ref().map(|prompt| prompt.input.as_str())
    }

    pub fn prompt_kind(&self) -> Option<PromptKind> {
        self.prompt.as_ref().map(|prompt| prompt.kind)
    }

    pub fn search_status(&self) -> Option<String> {
        let search = self.search.as_ref()?;

        if search.query.is_empty() {
            return None;
        }

        if search.matches.is_empty() {
            return Some("sin coincidencias".to_owned());
        }

        let current = search.current.map_or(0, |index| index + 1);
        Some(format!("{current}/{}", search.matches.len()))
    }

    pub fn current_match(&self) -> Option<(usize, usize, usize)> {
        let search = self.search.as_ref()?;
        let found = search.matches.get(search.current?)?;

        Some((found.line, found.start, found.end))
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn message_is_error(&self) -> bool {
        self.message_error
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    fn set_message(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some(text.into());
        self.message_error = error;
    }

    fn clear_message(&mut self) {
        self.message = None;
        self.message_error = false;
    }

    pub fn is_view_mode(&self) -> bool {
        self.view.is_some()
    }

    pub fn ensure_view(&mut self, width: u16, height: usize) {
        let ratio = self.view_scroll_ratio.take();
        let Some(view) = &mut self.view else {
            return;
        };

        if !view.rendered || view.width != width {
            let content = self.editor.content();
            view.lines = markdown::render_with_theme(&content, width, &self.theme);
            view.width = width;
            view.rendered = true;
            if let Some(ratio) = ratio {
                view.scroll = from_ratio(ratio, height, view.lines.len());
            }
        }

        view.viewport_height = height;
        clamp_view(view);
    }

    pub fn view_lines(&self) -> &[Line<'static>] {
        self.view.as_ref().map_or(&[], |view| view.lines.as_slice())
    }

    pub fn view_scroll(&self) -> usize {
        self.view.as_ref().map_or(0, |view| view.scroll)
    }

    fn toggle_view(&mut self) {
        self.editor.break_undo_group();
        self.pending.clear();

        match self.view.take() {
            Some(view) => {
                let ratio = progress(view.scroll, view.viewport_height, view.lines.len());
                let total = self.editor.rows().len();
                let viewport = self.editor.viewport_height();
                let target = from_ratio(ratio, viewport, total);
                self.editor.set_scroll_row(target);

                let rows = self.editor.rows();
                if let Some(row) = rows.get(target) {
                    self.editor.set_cursor(row.line, row.start);
                }
            }
            None => {
                let rows = self.editor.rows();
                let ratio = progress(
                    self.editor.scroll_row(),
                    self.editor.viewport_height(),
                    rows.len(),
                );
                self.view_scroll_ratio = Some(ratio);
                self.view = Some(View::default());
            }
        }
    }

    fn scroll_view_up(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = view.scroll.saturating_sub(1);
        }
    }

    fn scroll_view_down(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = (view.scroll + 1).min(max_scroll(view));
        }
    }

    fn page_view_up(&mut self) {
        if let Some(view) = &mut self.view {
            let step = view.viewport_height.saturating_sub(1).max(1);
            view.scroll = view.scroll.saturating_sub(step);
        }
    }

    fn page_view_down(&mut self) {
        if let Some(view) = &mut self.view {
            let step = view.viewport_height.saturating_sub(1).max(1);
            view.scroll = (view.scroll + step).min(max_scroll(view));
        }
    }

    fn view_home(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = 0;
        }
    }

    fn view_end(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = max_scroll(view);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.confirm_quit {
            if is_escape(&key) {
                self.confirm_quit = false;
                self.clear_message();
            } else if self.is_bound(&key, Command::Quit) {
                self.running = false;
            } else if self.is_bound(&key, Command::Save) {
                self.confirm_quit = false;
                self.clear_message();
                self.begin_save();
            } else {
                self.confirm_quit = false;
                self.clear_message();
            }
            return;
        }

        if self.is_bound(&key, Command::Quit) {
            self.request_quit();
            return;
        }

        self.clear_message();

        if is_escape(&key) {
            if self.prompt.is_some() {
                if self.prompt_kind() == Some(PromptKind::Search) {
                    self.close_search();
                } else {
                    self.prompt = None;
                }
            } else if self.view.is_some() {
                self.toggle_view();
            } else if self.settings.preset() == Preset::Vim && self.mode == Mode::Insert {
                self.mode = Mode::Normal;
            }
            self.pending.clear();
            return;
        }

        if self.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        let insert_mode = self.settings.preset() == Preset::Default || self.mode == Mode::Insert;
        let resolved = self
            .settings
            .bindings()
            .resolve(&key, insert_mode, &mut self.pending);
        if let Some(command) = resolved
            && !(self.view.is_some() && matches!(command, Command::Undo | Command::Redo))
        {
            self.dispatch(command);
            return;
        }

        if self.view.is_some() {
            let vim_normal = self.settings.preset() == Preset::Vim && self.mode == Mode::Normal;
            match key.code {
                KeyCode::Up => self.scroll_view_up(),
                KeyCode::Down => self.scroll_view_down(),
                KeyCode::PageUp => self.page_view_up(),
                KeyCode::PageDown => self.page_view_down(),
                KeyCode::Home => self.view_home(),
                KeyCode::End => self.view_end(),
                KeyCode::Char('k') if vim_normal => self.scroll_view_up(),
                KeyCode::Char('j') if vim_normal => self.scroll_view_down(),
                _ => {}
            }
            return;
        }

        if self.settings.preset() == Preset::Vim && self.mode == Mode::Normal {
            self.handle_vim_normal(&key);
            return;
        }

        self.handle_insert_key(&key);
    }

    fn is_bound(&self, key: &KeyEvent, command: Command) -> bool {
        self.settings.bindings().matches(key, command, true) || {
            let control = match command {
                Command::Quit => '\u{11}',
                Command::Save => '\u{13}',
                Command::View => '\u{16}',
                Command::Search => '\u{06}',
                Command::Undo => '\u{1a}',
                Command::Redo => '\u{19}',
                _ => '\0',
            };
            control != '\0' && matches!(key.code, KeyCode::Char(character) if character == control)
        }
    }

    fn dispatch(&mut self, command: Command) {
        match command {
            Command::Save => self.begin_save(),
            Command::Quit => self.request_quit(),
            Command::View => self.toggle_view(),
            Command::Search => self.begin_search(),
            Command::SearchNext => self.repeat_search(true),
            Command::SearchPrev => self.repeat_search(false),
            Command::Undo => self.editor.undo(),
            Command::Redo => self.editor.redo(),
        }
    }

    fn handle_vim_normal(&mut self, key: &KeyEvent) {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return;
        }

        match key.code {
            KeyCode::Char('h') => self.editor.move_left(),
            KeyCode::Char('j') => self.editor.move_down(),
            KeyCode::Char('k') => self.editor.move_up(),
            KeyCode::Char('l') => self.editor.move_right(),
            KeyCode::Char('w') => self.editor.move_word_right(),
            KeyCode::Char('b') => self.editor.move_word_left(),
            KeyCode::Char('0') => self.editor.move_home(),
            KeyCode::Char('$') => self.editor.move_end(),
            KeyCode::Char('x') => self.editor.delete(),
            KeyCode::Char('i') => self.mode = Mode::Insert,
            KeyCode::Char('a') => {
                self.editor.move_right();
                self.mode = Mode::Insert;
            }
            KeyCode::Char('o') => {
                self.editor.move_end();
                self.editor.insert_newline();
                self.mode = Mode::Insert;
            }
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: &KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Enter => self.editor.insert_newline(),
            KeyCode::Tab => self.editor.insert_tab(),
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Up => self.editor.move_up(),
            KeyCode::Down => self.editor.move_down(),
            KeyCode::Left if control => self.editor.move_word_left(),
            KeyCode::Right if control => self.editor.move_word_right(),
            KeyCode::Left => self.editor.move_left(),
            KeyCode::Right => self.editor.move_right(),
            KeyCode::Home => self.editor.move_home(),
            KeyCode::End => self.editor.move_end(),
            KeyCode::PageUp => self.editor.page_up(),
            KeyCode::PageDown => self.editor.page_down(),
            KeyCode::Char(character) if is_text_input(key) && !character.is_control() => {
                self.editor.insert_char(character);
            }
            _ => {}
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click_at(mouse.column, mouse.row),
            MouseEventKind::ScrollUp => self.wheel(-WHEEL_LINES),
            MouseEventKind::ScrollDown => self.wheel(WHEEL_LINES),
            _ => {}
        }
    }

    fn click_at(&mut self, column: u16, row: u16) {
        let area = self.text_area;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let relative_row = row.saturating_sub(area.y);
        let relative_column = column.saturating_sub(area.x);
        if relative_row >= area.height || relative_column >= area.width {
            return;
        }

        if self.view.is_some() {
            let view_row = usize::from(relative_row) + self.view_scroll();
            let total = self.view_lines().len();
            let ratio = progress(view_row, 0, total);
            let viewport = self.editor.viewport_height();
            let target = from_ratio(ratio, viewport, self.editor.rows().len());
            self.toggle_view();
            self.editor.set_scroll_row(target);
            let rows = self.editor.rows();
            if let Some(row) = rows.get(target) {
                self.editor.set_cursor(row.line, row.start);
            }
            return;
        }

        let row_index = usize::from(relative_row) + self.editor.scroll_row();
        if let Some((line, cursor_column)) = self
            .editor
            .position_at(row_index, usize::from(relative_column))
        {
            self.editor.set_cursor(line, cursor_column);
        }
    }

    fn wheel(&mut self, delta: isize) {
        if let Some(view) = &mut self.view {
            let current = isize::try_from(view.scroll).unwrap_or(isize::MAX);
            let limit = isize::try_from(max_scroll(view)).unwrap_or(isize::MAX);
            let target = current.saturating_add(delta).clamp(0, limit);
            view.scroll = usize::try_from(target).unwrap_or(0);
        } else {
            self.editor.scroll_rows(delta);
        }
    }

    fn request_quit(&mut self) {
        if self.editor.is_modified() {
            self.prompt = None;
            self.confirm_quit = true;
            self.set_message(QUIT_WARNING, false);
        } else {
            self.running = false;
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) {
        match self.prompt_kind() {
            Some(PromptKind::SaveAs) => self.handle_save_prompt_key(key),
            Some(PromptKind::Search) => self.handle_search_prompt_key(key),
            None => {}
        }
    }

    fn handle_save_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_save_as(),
            KeyCode::Esc => self.prompt = None,
            KeyCode::Backspace => {
                if let Some(prompt) = &mut self.prompt {
                    remove_last_grapheme(&mut prompt.input);
                }
            }
            KeyCode::Char(character) if is_text_input(&key) && !character.is_control() => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.push(character);
                }
            }
            _ => {}
        }
    }

    fn handle_search_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.step_search(false)
            }
            KeyCode::Enter => self.step_search(true),
            KeyCode::Esc => self.close_search(),
            KeyCode::Backspace => {
                if let Some(prompt) = &mut self.prompt {
                    remove_last_grapheme(&mut prompt.input);
                }
                self.refresh_search();
            }
            KeyCode::Char(character) if is_text_input(&key) && !character.is_control() => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.push(character);
                }
                self.refresh_search();
            }
            _ => {}
        }
    }

    fn begin_search(&mut self) {
        if self.view.is_some() {
            self.toggle_view();
        }

        let origin_line = self.editor.cursor_line();
        let origin_column = self.editor.cursor_column();

        self.search = Some(Search {
            query: String::new(),
            matches: Vec::new(),
            current: None,
            origin_line,
            origin_column,
        });
        self.prompt = Some(Prompt {
            kind: PromptKind::Search,
            input: String::new(),
        });
        self.clear_message();
    }

    fn refresh_search(&mut self) {
        let Some(input) = self.prompt.as_ref().map(|prompt| prompt.input.clone()) else {
            return;
        };
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let origin_line = search.origin_line;
        let origin_column = search.origin_column;
        let matches = find_matches(self.editor.lines(), &input);

        let current = if input.is_empty() {
            None
        } else {
            matches
                .iter()
                .position(|candidate| {
                    candidate.line > origin_line
                        || (candidate.line == origin_line && candidate.start >= origin_column)
                })
                .or_else(|| (!matches.is_empty()).then_some(0))
        };

        if let Some(search) = self.search.as_mut() {
            search.query = input;
            search.matches = matches;
            search.current = current;
        }

        self.jump_to_current_match();
    }

    fn step_search(&mut self, forward: bool) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        if search.matches.is_empty() {
            return;
        }

        let total = search.matches.len();
        let next = match (search.current, forward) {
            (Some(index), true) => (index + 1) % total,
            (Some(index), false) => (index + total - 1) % total,
            (None, true) => 0,
            (None, false) => total - 1,
        };

        if let Some(search) = self.search.as_mut() {
            search.current = Some(next);
        }
        self.jump_to_current_match();
    }

    fn jump_to_current_match(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let Some(index) = search.current else {
            return;
        };
        let Some(found) = search.matches.get(index) else {
            return;
        };
        let (line, start) = (found.line, found.start);

        self.editor.set_cursor(line, start);
    }

    fn close_search(&mut self) {
        if let Some(search) = self.search.take()
            && !search.query.is_empty()
        {
            self.last_search = search.query;
        }
        self.prompt = None;
    }

    fn repeat_search(&mut self, forward: bool) {
        if self.last_search.is_empty() || self.prompt.is_some() {
            return;
        }

        if self.view.is_some() {
            self.toggle_view();
        }

        let line = self.editor.cursor_line();
        let column = self.editor.cursor_column();
        let query = self.last_search.clone();
        let matches = find_matches(self.editor.lines(), &query);

        if matches.is_empty() {
            self.set_message(format!("Sin coincidencias para «{query}»."), false);
            return;
        }

        let target = if forward {
            matches
                .iter()
                .position(|candidate| {
                    candidate.line > line || (candidate.line == line && candidate.start > column)
                })
                .unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|candidate| {
                    candidate.line < line || (candidate.line == line && candidate.start < column)
                })
                .unwrap_or(matches.len() - 1)
        };

        let found = matches[target];
        self.editor.set_cursor(found.line, found.start);
        self.set_message(
            format!("Coincidencia {} de {}", target + 1, matches.len()),
            false,
        );
    }

    fn begin_save(&mut self) {
        if let Some(path) = self.editor.path().map(Path::to_path_buf) {
            self.save_to(&path);
        } else {
            self.prompt = Some(Prompt {
                kind: PromptKind::SaveAs,
                input: std::mem::take(&mut self.last_save_name),
            });
        }
    }

    fn confirm_save_as(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };

        let name = prompt.input.trim();
        if name.is_empty() {
            self.set_message("El nombre del archivo no puede estar vacío.", true);
            return;
        }

        self.last_save_name = name.to_owned();
        self.save_to(Path::new(name));
    }

    fn save_to(&mut self, path: &Path) {
        match file::save_file(path, &self.editor.content()) {
            Ok(()) => {
                self.editor.set_path(path.to_path_buf());
                self.editor.mark_saved();
                self.set_message("Guardado.", false);
            }
            Err(error) => self.set_message(format!("{error:#}"), true),
        }
    }
}

fn is_escape(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc)
}

fn find_matches(lines: &[String], query: &str) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let needle = lowercase_graphemes(query);
    if needle.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let haystack = lowercase_graphemes(line);
        if haystack.len() < needle.len() {
            continue;
        }

        let mut start = 0;
        while start + needle.len() <= haystack.len() {
            if haystack[start..].starts_with(&needle) {
                matches.push(SearchMatch {
                    line: line_index,
                    start,
                    end: start + needle.len(),
                });
                start += needle.len();
            } else {
                start += 1;
            }
        }
    }

    matches
}

fn lowercase_graphemes(text: &str) -> Vec<String> {
    text.graphemes(true).map(str::to_lowercase).collect()
}

fn progress(current: usize, viewport: usize, total: usize) -> f64 {
    let max = total.saturating_sub(viewport);
    if max == 0 {
        return 0.0;
    }
    (current.min(max) as f64) / (max as f64)
}

fn from_ratio(ratio: f64, viewport: usize, total: usize) -> usize {
    let max = total.saturating_sub(viewport);
    if max == 0 {
        return 0;
    }
    let scaled = (ratio * max as f64).round().clamp(0.0, max as f64);
    scaled as usize
}

fn max_scroll(view: &View) -> usize {
    view.lines.len().saturating_sub(view.viewport_height)
}

fn clamp_view(view: &mut View) {
    view.scroll = view.scroll.min(max_scroll(view));
}

fn is_text_input(key: &KeyEvent) -> bool {
    !key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT)
}

fn remove_last_grapheme(text: &mut String) {
    if let Some((index, _)) = text.grapheme_indices(true).next_back() {
        text.truncate(index);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;

    use super::{App, PromptKind, WHEEL_LINES, find_matches};
    use crate::editor::Editor;
    use crate::keys::Settings;
    use crate::theme::Theme;

    fn vim_app(text: &str) -> App {
        let settings = Settings::for_test("keymap = \"vim\"", None);
        App::with_options(Editor::from_text(text, None), Theme::ansi(), settings)
    }

    fn press(app: &mut App, character: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }

    #[test]
    fn pide_un_nombre_y_guarda_un_archivo_nuevo() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        let mut app = App::new(Editor::from_text("Hola", None));

        app.begin_save();
        let prompt = app.prompt.as_mut().unwrap();
        prompt.input = path.to_string_lossy().into_owned();
        app.confirm_save_as();

        assert_eq!(app.message(), Some("Guardado."));
        assert!(app.prompt_input().is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Hola");
        assert_eq!(app.editor.path(), Some(path.as_path()));
    }

    #[test]
    fn rechaza_un_nombre_vacio() {
        let mut app = App::new(Editor::from_text("Hola", None));

        app.begin_save();
        app.prompt.as_mut().unwrap().input.clear();
        app.confirm_save_as();

        assert!(app.prompt_input().is_none());
        assert!(app.message().is_some());
    }

    #[test]
    fn alterna_entre_fuente_y_vista() {
        let mut app = App::new(Editor::from_text("# Título", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert!(app.is_view_mode());
        assert_eq!(app.editor.content(), "# Título");

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert!(!app.is_view_mode());
    }

    #[test]
    fn escape_vuelve_de_la_vista_a_la_fuente() {
        let mut app = App::new(Editor::from_text("# Título", None));

        app.toggle_view();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!app.is_view_mode());
    }

    #[test]
    fn guardar_limpia_la_marca_de_cambios() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        let mut app = App::new(Editor::from_text("Hola", None));

        app.editor.move_end();
        app.editor.insert_char('!');
        assert!(app.editor.is_modified());

        app.save_to(&path);

        assert_eq!(app.message(), Some("Guardado."));
        assert!(!app.editor.is_modified());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Hola!");
    }

    #[test]
    fn salir_sin_cambios_no_pide_confirmacion() {
        let mut app = App::new(Editor::from_text("Hola", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));

        assert!(!app.running);
    }

    #[test]
    fn pide_confirmacion_antes_de_salir_con_cambios() {
        let mut app = App::new(Editor::from_text("Hola", None));
        app.editor.move_end();
        app.editor.insert_char('!');

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(app.running);
        assert!(app.confirm_quit);
        assert!(app.message().is_some());

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(!app.running);
    }

    #[test]
    fn escape_cancela_la_confirmacion_de_salida() {
        let mut app = App::new(Editor::from_text("Hola", None));
        app.editor.move_end();
        app.editor.insert_char('!');

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.running);
        assert!(!app.confirm_quit);
        assert!(app.message().is_none());
        assert_eq!(app.editor.content(), "Hola!");
    }

    #[test]
    fn guardar_desde_la_confirmacion_no_sale() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        let mut app = App::new(Editor::from_text("Hola", None));
        app.editor.move_end();
        app.editor.insert_char('!');
        app.editor.set_path(path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(app.running);
        assert!(!app.confirm_quit);
        assert!(!app.editor.is_modified());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Hola!");
    }

    #[test]
    fn desplaza_la_vista_y_acota_el_final() {
        let text = (0..40)
            .map(|index| format!("línea {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut app = App::new(Editor::from_text(&text, None));

        app.toggle_view();
        app.ensure_view(30, 5);

        app.scroll_view_down();
        assert_eq!(app.view_scroll(), 1);

        app.view_end();
        let maximum = app.view_lines().len().saturating_sub(5);
        assert_eq!(app.view_scroll(), maximum);

        app.view_home();
        assert_eq!(app.view_scroll(), 0);
    }

    #[test]
    fn deshace_y_rehace_con_los_atajos() {
        let mut app = App::new(Editor::from_text("", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(app.editor.content(), "ab");

        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(app.editor.content(), "");

        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert_eq!(app.editor.content(), "ab");
    }

    #[test]
    fn control_mas_shift_z_tambien_rehace() {
        let mut app = App::new(Editor::from_text("", None));

        app.editor.insert_char('a');
        app.editor.undo();
        assert_eq!(app.editor.content(), "");

        app.handle_key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(app.editor.content(), "a");
    }

    #[test]
    fn deshacer_no_cambia_el_modo_vista() {
        let mut app = App::new(Editor::from_text("hola", None));
        app.editor.insert_char('!');
        app.toggle_view();

        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));

        assert!(app.is_view_mode());
        assert_eq!(app.editor.content(), "!hola");
    }

    #[test]
    fn busca_y_salta_entre_coincidencias() {
        let mut app = App::new(Editor::from_text("uno dos uno dos", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(matches!(app.prompt_kind(), Some(PromptKind::Search)));

        for character in "dos".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        assert_eq!(app.editor.cursor_column(), 4);
        assert_eq!(app.search_status().as_deref(), Some("1/2"));
        assert_eq!(app.current_match(), Some((0, 4, 7)));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 12);
        assert_eq!(app.search_status().as_deref(), Some("2/2"));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 4);

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(app.editor.cursor_column(), 12);
        assert_eq!(app.search_status().as_deref(), Some("2/2"));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 4);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.prompt_kind().is_none());
        assert_eq!(app.current_match(), None);
        assert_eq!(app.editor.cursor_column(), 4);
    }

    #[test]
    fn f3_repite_la_ultima_busqueda() {
        let mut app = App::new(Editor::from_text("uno dos uno dos", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for character in "dos".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 4);

        app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 12);
        assert_eq!(app.message(), Some("Coincidencia 2 de 2"));

        app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::SHIFT));
        assert_eq!(app.editor.cursor_column(), 4);
    }

    #[test]
    fn buscar_desde_vista_vuelve_a_fuente() {
        let mut app = App::new(Editor::from_text("# Título", None));
        app.toggle_view();

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));

        assert!(!app.is_view_mode());
        assert!(matches!(app.prompt_kind(), Some(PromptKind::Search)));
    }

    #[test]
    fn avisa_cuando_la_busqueda_no_encuentra_nada() {
        let mut app = App::new(Editor::from_text("hola", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for character in "zzz".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        assert_eq!(app.search_status().as_deref(), Some("sin coincidencias"));
        assert_eq!(app.current_match(), None);
    }

    #[test]
    fn encuentra_coincidencias_sin_distinguir_mayusculas() {
        let lines = vec!["Hola Mundo".to_owned(), "mundo mundo".to_owned()];

        let matches = find_matches(&lines, "mundo");

        assert_eq!(matches.len(), 3);
        assert_eq!(
            (matches[0].line, matches[0].start, matches[0].end),
            (0, 5, 10)
        );
        assert_eq!(
            (matches[1].line, matches[1].start, matches[1].end),
            (1, 0, 5)
        );
        assert_eq!(
            (matches[2].line, matches[2].start, matches[2].end),
            (1, 6, 11)
        );
    }

    #[test]
    fn busca_acentos_por_grafemas() {
        let lines = vec!["El año pasado".to_owned()];

        let matches = find_matches(&lines, "AÑO");

        assert_eq!(matches.len(), 1);
        assert_eq!((matches[0].start, matches[0].end), (3, 6));
    }

    #[test]
    fn vim_mueve_el_cursor_en_modo_normal() {
        let mut app = vim_app("hola\nmundo");
        assert_eq!(app.mode_label(), "NORMAL");

        press(&mut app, 'l');
        assert_eq!(app.editor.cursor_column(), 1);

        press(&mut app, 'j');
        assert_eq!(app.editor.cursor_line(), 1);

        press(&mut app, '$');
        assert_eq!(app.editor.cursor_column(), 5);

        press(&mut app, '0');
        assert_eq!(app.editor.cursor_column(), 0);
    }

    #[test]
    fn vim_inserta_y_vuelve_a_normal() {
        let mut app = vim_app("hola");

        press(&mut app, 'i');
        assert_eq!(app.mode_label(), "INSERT");
        for character in "xyz".chars() {
            press(&mut app, character);
        }
        assert_eq!(app.editor.content(), "xyzhola");

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode_label(), "NORMAL");

        press(&mut app, 'x');
        assert_eq!(app.editor.content(), "xyzola");
    }

    #[test]
    fn vim_abrir_linea_inserta_debajo() {
        let mut app = vim_app("uno\ndos");
        press(&mut app, 'l');
        press(&mut app, 'o');

        assert_eq!(app.editor.content(), "uno\n\ndos");
        assert_eq!(app.mode_label(), "INSERT");
    }

    #[test]
    fn vim_guarda_y_sale_con_linea_de_comandos() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        let mut app = vim_app("hola");
        app.editor.set_path(path.clone());
        app.editor.insert_char('!');

        press(&mut app, ':');
        press(&mut app, 'w');
        assert_eq!(fs::read_to_string(&path).unwrap(), "!hola");
        assert!(!app.editor.is_modified());

        press(&mut app, ':');
        press(&mut app, 'q');
        assert!(!app.running);
    }

    #[test]
    fn vim_busca_y_repite_la_busqueda() {
        let mut app = vim_app("uno dos uno dos");

        press(&mut app, '/');
        assert_eq!(app.prompt_kind(), Some(PromptKind::Search));
        for character in "dos".chars() {
            press(&mut app, character);
        }
        assert_eq!(app.editor.cursor_column(), 4);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode_label(), "NORMAL");

        press(&mut app, 'n');
        assert_eq!(app.message(), Some("Coincidencia 2 de 2"));
        press(&mut app, 'N');
        assert_eq!(app.editor.cursor_column(), 4);
    }

    #[test]
    fn vim_deshace_y_rehace() {
        let mut app = vim_app("hola");
        press(&mut app, 'i');
        press(&mut app, '!');
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        press(&mut app, 'u');
        assert_eq!(app.editor.content(), "hola");

        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(app.editor.content(), "!hola");
    }

    #[test]
    fn los_atajos_personalizados_sustituyen_a_los_predeterminados() {
        let settings = Settings::for_test("[bindings]\nsave = [\"Ctrl+Shift+S\"]", None);
        let mut app = App::with_options(Editor::from_text("hola", None), Theme::ansi(), settings);

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.prompt_input().is_none());

        app.handle_key(KeyEvent::new(
            KeyCode::Char('S'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(app.prompt_kind(), Some(PromptKind::SaveAs));
    }

    #[test]
    fn el_clic_coloca_el_cursor_en_la_fuente() {
        let mut app = App::new(Editor::from_text("hola\nmundo", None));
        app.set_text_area(Rect::new(0, 0, 10, 3));
        app.editor.ensure_visible(10, 3);

        app.click_at(3, 1);

        assert_eq!(app.editor.cursor_line(), 1);
        assert_eq!(app.editor.cursor_column(), 3);
    }

    #[test]
    fn el_clic_respeta_el_desplazamiento_de_la_fuente() {
        let text = (0..10)
            .map(|index| format!("linea{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = App::new(Editor::from_text(&text, None));
        app.set_text_area(Rect::new(0, 0, 10, 3));
        app.editor.ensure_visible(10, 3);
        app.editor.set_scroll_row(5);

        app.click_at(0, 0);

        assert_eq!(app.editor.cursor_line(), 5);
    }

    #[test]
    fn la_rueda_desplaza_la_fuente_y_la_vista() {
        let text = (0..40)
            .map(|index| format!("línea {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut app = App::new(Editor::from_text(&text, None));
        app.set_text_area(Rect::new(0, 0, 30, 5));
        app.editor.ensure_visible(30, 5);

        app.wheel(WHEEL_LINES);
        assert_eq!(app.editor.scroll_row(), 3);

        app.toggle_view();
        app.ensure_view(30, 5);
        let before = app.view_scroll();
        app.wheel(WHEEL_LINES);
        assert_eq!(app.view_scroll(), before + 3);
    }

    #[test]
    fn conserva_la_posicion_al_cambiar_de_vista_y_volver() {
        let text = (0..40)
            .map(|index| format!("línea {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut app = App::new(Editor::from_text(&text, None));
        app.editor.ensure_visible(30, 5);
        let source_end = app.editor.rows().len().saturating_sub(5);
        app.editor.set_scroll_row(source_end);

        app.toggle_view();
        app.ensure_view(30, 5);
        let view_end = app.view_lines().len().saturating_sub(5);
        assert_eq!(app.view_scroll(), view_end);

        app.toggle_view();
        assert_eq!(app.editor.scroll_row(), source_end);
    }

    #[test]
    fn conserva_una_posicion_intermedia_al_cambiar_de_vista() {
        let text = (0..40)
            .map(|index| format!("línea {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut app = App::new(Editor::from_text(&text, None));
        app.editor.ensure_visible(30, 5);
        let source_end = app.editor.rows().len().saturating_sub(5);
        app.editor.set_scroll_row(source_end / 2);

        app.toggle_view();
        app.ensure_view(30, 5);
        let view_scroll = app.view_scroll();
        let view_end = app.view_lines().len().saturating_sub(5);
        assert!(view_scroll > 0 && view_scroll < view_end);

        app.toggle_view();
        let expected = (view_scroll as f64 / view_end as f64 * source_end as f64).round() as usize;
        assert_eq!(app.editor.scroll_row(), expected);
    }

    #[test]
    fn el_clic_en_la_vista_vuelve_a_la_fuente_cerca_de_la_linea() {
        let text = (0..40)
            .map(|index| format!("línea {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut app = App::new(Editor::from_text(&text, None));
        app.set_text_area(Rect::new(0, 0, 30, 5));
        app.editor.ensure_visible(30, 5);
        app.toggle_view();
        app.ensure_view(30, 5);
        app.view_end();
        let view_total = app.view_lines().len();
        let view_row = 4 + app.view_scroll();
        let ratio = view_row as f64 / view_total as f64;
        let source_rows = app.editor.rows().len();
        let viewport = app.editor.viewport_height();
        let target = (ratio * source_rows.saturating_sub(viewport) as f64).round() as usize;
        let expected = app.editor.rows()[target].line;

        app.click_at(0, 4);

        assert!(!app.is_view_mode());
        assert!(app.editor.scroll_row() > 0);
        assert_eq!(app.editor.cursor_line(), expected);
    }
}
