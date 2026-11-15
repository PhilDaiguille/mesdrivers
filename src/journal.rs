//! Journal des installations : %LOCALAPPDATA%\mesdrivers\journal.txt
use std::io::Write;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from).join("mesdrivers").join("journal.txt")
}

/// Ajoute une ligne horodatée. Un échec d'écriture ne doit pas bloquer l'installation : on le signale seulement.
pub fn log(line: &str) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
    writeln!(f, "{}  {line}", crate::model::stamp(secs))
}
