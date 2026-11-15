//! Appels aux outils Windows intégrés (pnputil, expand). Les codes retour font foi, pas le texte (traduit).
use std::path::Path;
use std::process::Command;

/// Lance un outil et renvoie (code retour, sortie complète).
fn run(exe: &str, args: &[&str]) -> Result<(i32, String), String> {
    let out = Command::new(exe).args(args).output().map_err(|e| format!("{exe} introuvable : {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.code().unwrap_or(-1), text))
}

fn last_line(text: &str) -> &str {
    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
}

/// Ajoute et installe tous les pilotes d'un dossier. Renvoie true si un redémarrage est demandé.
pub fn add_dir(dir: &Path) -> Result<bool, String> {
    let pattern = dir.join("*.inf");
    let (code, out) = run("pnputil", &["/add-driver", &pattern.to_string_lossy(), "/subdirs", "/install"])?;
    match code {
        0 => Ok(false),
        3010 => Ok(true), // ERROR_SUCCESS_REBOOT_REQUIRED
        259 => Err("pilote ajouté au magasin, mais aucun périphérique ne l'a pris (pas meilleur que l'actuel)".into()),
        _ => Err(format!("pnputil code {code} : {}", last_line(&out))),
    }
}

pub fn export_all(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (code, out) = run("pnputil", &["/export-driver", "*", &dir.to_string_lossy()])?;
    if code == 0 { Ok(()) } else { Err(format!("pnputil code {code} : {}", last_line(&out))) }
}

/// Supprime un paquet du DriverStore. Sans /force : Windows refuse si un périphérique l'utilise.
pub fn delete(oem: &str) -> Result<(), String> {
    let (code, out) = run("pnputil", &["/delete-driver", oem])?;
    if code == 0 { Ok(()) } else { Err(format!("{oem} : {}", last_line(&out))) }
}

pub fn expand_cab(cab: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (code, out) = run("expand", &[&cab.to_string_lossy(), "-F:*", &dir.to_string_lossy()])?;
    if code == 0 { Ok(()) } else { Err(format!("expand code {code} : {}", last_line(&out))) }
}

/// Tous les fichiers sous `dir` (récursif).
pub fn files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p) } else { out.push(p) }
        }
    }
    out
}

pub fn dir_size(dir: &Path) -> u64 {
    files(dir).iter().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum()
}

/// Vrai si un .inf du dossier déclare l'architecture de ce PC (NTamd64 / NTarm64).
pub fn has_inf_for_this_arch(dir: &Path) -> bool {
    let arch = if cfg!(target_arch = "aarch64") { "ntarm64" } else { "ntamd64" };
    files(dir)
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("inf")))
        .any(|f| crate::store::read_text(f).to_lowercase().contains(arch))
}
