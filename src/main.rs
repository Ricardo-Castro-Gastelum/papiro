//! Papiro: editor y lector de Markdown para la terminal.
//!
//! La lectura del archivo ocurre antes de entrar en la pantalla alterna, de
//! modo que cualquier error se muestra en la terminal normal. Una guardia y un
//! hook de pánico devuelven la terminal a su estado original al salir.

mod app;
mod editor;
mod file;
mod markdown;
mod render;

use std::env;
use std::io::{self, Stdout, stdout};
use std::panic;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use crossterm::cursor::Show;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::App;
use crate::editor::Editor;

const HELP: &str = "\
Papiro — editor y lector de Markdown para la terminal.

Uso:
  ppr [ARCHIVO]

Argumentos:
  ARCHIVO    Archivo Markdown que se abre. Si no existe, se crea al guardar.
             Sin argumento se abre un búfer sin título.

Opciones:
  -h, --help       Muestra esta ayuda y sale.
  -V, --version    Muestra la versión y sale.

Atajos:
  Ctrl+S guardar   Ctrl+V vista   Ctrl+F buscar   Ctrl+Q salir

Repositorio: https://github.com/Ricardo-Castro-Gastelum/papiro";

/// Lo que hay que hacer según los argumentos de la línea de comandos.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Edit(Option<PathBuf>),
    Help,
    Version,
}

fn main() -> ExitCode {
    install_panic_hook();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Papiro: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Ejecuta el programa completo. La terminal solo se inicializa después de
/// leer el archivo, para no alterar la sesión si la ruta falla.
fn run() -> Result<()> {
    let path = match parse_args()? {
        Action::Edit(path) => path,
        Action::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Action::Version => {
            println!("ppr {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    };

    let editor = Editor::open(path)?;
    let mut app = App::new(editor);

    let mut session = TerminalSession::new()?;
    app.run(session.terminal())?;
    drop(session);

    Ok(())
}

fn parse_args() -> Result<Action> {
    parse_args_from(env::args_os().skip(1))
}

/// Acepta como máximo una ruta, además de `--help` y `--version`. Los
/// argumentos se leen como `OsString` para admitir rutas que no sean UTF-8
/// válido.
fn parse_args_from<I>(args: I) -> Result<Action>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Action::Edit(None));
    };

    if let Some(flag) = first.to_str() {
        match flag {
            "-h" | "--help" => return Ok(Action::Help),
            "-V" | "--version" => return Ok(Action::Version),
            _ => {}
        }
    }

    if args.next().is_some() {
        bail!("Uso: ppr [ARCHIVO]\nPrueba «ppr --help» para más información.");
    }

    if first.is_empty() {
        bail!("La ruta del archivo no puede estar vacía.");
    }

    Ok(Action::Edit(Some(PathBuf::from(first))))
}

/// Mantiene la terminal en pantalla alterna y modo raw mientras vive.
struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn new() -> Result<Self> {
        enable_raw_mode().context("No se pudo activar el modo raw de la terminal")?;

        let mut output = stdout();
        if let Err(error) = execute!(output, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("No se pudo abrir la pantalla alterna");
        }

        let backend = CrosstermBackend::new(output);
        match Terminal::new(backend) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                restore_terminal();
                Err(error).context("No se pudo inicializar la interfaz de terminal")
            }
        }
    }

    fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Hook de pánico: restaura la terminal antes de que se imprima el mensaje.
fn install_panic_hook() {
    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}

/// Restaura la terminal. Es idempotente: puede llamarse desde el hook de
/// pánico y de nuevo desde `Drop` sin efectos adversos.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{Action, parse_args_from};

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn sin_argumentos_abre_un_buffer_sin_titulo() {
        assert_eq!(parse_args_from(args(&[])).ok(), Some(Action::Edit(None)));
    }

    #[test]
    fn acepta_una_ruta() {
        assert_eq!(
            parse_args_from(args(&["notas.md"])).ok(),
            Some(Action::Edit(Some(PathBuf::from("notas.md"))))
        );
    }

    #[test]
    fn reconoce_la_ayuda() {
        assert_eq!(parse_args_from(args(&["--help"])).ok(), Some(Action::Help));
        assert_eq!(parse_args_from(args(&["-h"])).ok(), Some(Action::Help));
    }

    #[test]
    fn reconoce_la_version() {
        assert_eq!(
            parse_args_from(args(&["--version"])).ok(),
            Some(Action::Version)
        );
        assert_eq!(parse_args_from(args(&["-V"])).ok(), Some(Action::Version));
    }

    #[test]
    fn rechaza_mas_de_una_ruta() {
        assert!(parse_args_from(args(&["uno.md", "dos.md"])).is_err());
    }

    #[test]
    fn rechaza_una_ruta_vacia() {
        assert!(parse_args_from(args(&[""])).is_err());
    }
}
