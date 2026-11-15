use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result};

const BOM: &[u8] = b"\xEF\xBB\xBF";

pub fn read_file(path: &Path) -> Result<Option<String>> {
    let mut bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("No se pudo leer el archivo «{}»", path.display()));
        }
    };

    if bytes.starts_with(BOM) {
        bytes.drain(..BOM.len());
    }

    let text = String::from_utf8(bytes).map_err(|_| {
        anyhow::anyhow!(
            "El archivo «{}» no está codificado en UTF-8 válido.",
            path.display()
        )
    })?;

    Ok(Some(normalize_newlines(&text)))
}

pub fn save_file(path: &Path, contents: &str) -> Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };

    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());

    let mut builder = tempfile::Builder::new();
    builder.prefix(".ppr-");

    #[cfg(unix)]
    if existing_permissions.is_none() {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }

    let mut temporary = builder.tempfile_in(parent).with_context(|| {
        format!(
            "No se pudo crear un archivo temporal en «{}»",
            parent.display()
        )
    })?;

    temporary
        .write_all(contents.as_bytes())
        .with_context(|| format!("No se pudo escribir «{}»", path.display()))?;
    temporary.flush().with_context(|| {
        format!(
            "No se pudo vaciar el archivo temporal de «{}»",
            path.display()
        )
    })?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("No se pudo sincronizar «{}»", path.display()))?;

    if let Some(permissions) = existing_permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .with_context(|| {
                format!(
                    "No se pudieron conservar los permisos de «{}»",
                    path.display()
                )
            })?;
    }

    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("No se pudo guardar «{}»", path.display()))?;

    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }

    Ok(())
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{normalize_newlines, read_file, save_file};

    #[test]
    fn convierte_crlf_y_cr_a_lf() {
        assert_eq!(normalize_newlines("uno\r\ndos\rtres\n"), "uno\ndos\ntres\n");
    }

    #[test]
    fn conserva_el_texto_sin_cambios() {
        assert_eq!(normalize_newlines("a\nb\n"), "a\nb\n");
    }

    #[test]
    fn lee_archivos_y_elimina_el_bom() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        fs::write(&path, b"\xEF\xBB\xBF# Hola\r\n").unwrap();

        assert_eq!(read_file(&path).unwrap().as_deref(), Some("# Hola\n"));
    }

    #[test]
    fn devuelve_none_si_el_archivo_no_existe() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("no-existe.md");

        assert!(read_file(&path).unwrap().is_none());
    }

    #[test]
    fn rechaza_utf8_invalido() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalido.md");
        fs::write(&path, b"\xFF\xFE").unwrap();

        assert!(read_file(&path).is_err());
    }

    #[test]
    fn guarda_un_archivo_nuevo_sin_dejar_temporales() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");

        save_file(&path, "# Hola\n").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "# Hola\n");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn sobrescribe_el_archivo_existente() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        fs::write(&path, "viejo").unwrap();

        save_file(&path, "nuevo").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "nuevo");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn conserva_los_permisos_al_sobrescribir() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        fs::write(&path, "viejo").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        save_file(&path, "nuevo").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn informa_si_el_directorio_no_existe() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("carpeta-inexistente").join("nota.md");

        assert!(save_file(&path, "contenido").is_err());
    }
}
