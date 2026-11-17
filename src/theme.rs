use std::env;

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy)]
pub struct Theme {
    colors: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self::ansi()
    }
}

impl Theme {
    pub fn from_env() -> Self {
        if env::var_os("NO_COLOR").is_some() {
            Self::plain()
        } else {
            Self::ansi()
        }
    }

    pub fn ansi() -> Self {
        Self { colors: true }
    }

    pub fn plain() -> Self {
        Self { colors: false }
    }

    fn fg(&self, color: Color, modifiers: Modifier) -> Style {
        let style = Style::default().add_modifier(modifiers);
        if self.colors { style.fg(color) } else { style }
    }

    pub fn heading(&self, level: usize) -> Style {
        match level {
            1 => self.fg(Color::Cyan, Modifier::BOLD | Modifier::UNDERLINED),
            2 => self.fg(Color::Blue, Modifier::BOLD),
            _ => self.fg(Color::Magenta, Modifier::BOLD),
        }
    }

    pub fn code(&self) -> Style {
        self.fg(Color::Yellow, Modifier::DIM)
    }

    pub fn link(&self) -> Style {
        self.fg(Color::Blue, Modifier::UNDERLINED)
    }

    pub fn image(&self) -> Style {
        self.fg(Color::Magenta, Modifier::DIM | Modifier::ITALIC)
    }

    pub fn quote(&self) -> Style {
        self.fg(Color::Green, Modifier::DIM)
    }

    pub fn bullet(&self) -> Style {
        self.fg(Color::Green, Modifier::DIM)
    }

    pub fn task(&self) -> Style {
        self.fg(Color::Green, Modifier::DIM)
    }

    pub fn rule(&self) -> Style {
        self.fg(Color::DarkGray, Modifier::DIM)
    }

    pub fn table_header(&self) -> Style {
        self.fg(Color::Cyan, Modifier::BOLD)
    }

    pub fn html(&self) -> Style {
        self.fg(Color::DarkGray, Modifier::DIM)
    }

    pub fn emphasis(&self) -> Style {
        self.fg(Color::Magenta, Modifier::empty())
    }

    pub fn modified(&self) -> Style {
        self.fg(Color::Yellow, Modifier::empty())
    }

    pub fn error(&self) -> Style {
        self.fg(Color::Red, Modifier::empty())
    }

    pub fn prompt(&self) -> Style {
        self.fg(Color::Cyan, Modifier::DIM)
    }

    pub fn mode(&self) -> Style {
        self.fg(Color::Green, Modifier::DIM)
    }

    pub fn dim(&self) -> Style {
        Style::default().add_modifier(Modifier::DIM)
    }

    pub fn search(&self) -> Style {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::Theme;

    #[test]
    fn la_paleta_ansi_usa_colores() {
        let theme = Theme::ansi();
        assert_eq!(theme.heading(1).fg, Some(Color::Cyan));
        assert_eq!(theme.code().fg, Some(Color::Yellow));
        assert_eq!(theme.error().fg, Some(Color::Red));
    }

    #[test]
    fn no_color_quitta_los_colores_pero_conserva_atributos() {
        let theme = Theme::plain();
        assert_eq!(theme.heading(1).fg, None);
        assert!(
            theme
                .heading(1)
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
        assert_eq!(theme.code().fg, None);
        assert_eq!(theme.error().fg, None);
    }
}
