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
    let path = path_from_args()?;
    let editor = Editor::open(path)?;
    let mut app = App::new(editor);

    let mut session = TerminalSession::new()?;
    app.run(session.terminal())?;
    drop(session);

    Ok(())
}

/// Acepta como máximo una ruta. Los argumentos se leen como `OsString` para
/// admitir rutas que no sean UTF-8 válido.
fn path_from_args() -> Result<Option<PathBuf>> {
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next() else {
        return Ok(None);
    };

    if args.next().is_some() {
        bail!("Uso: ppr [archivo.md]");
    }

    if path.is_empty() {
        bail!("La ruta del archivo no puede estar vacía.");
    }

    Ok(Some(PathBuf::from(path)))
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
