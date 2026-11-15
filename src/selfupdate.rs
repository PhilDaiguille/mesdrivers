//! Mise à jour de l'appli depuis les releases GitHub (exe vérifié par SHA-256).
use crate::model::cmp_version;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const REPO: &str = "PhilDaiguille/mesdrivers";

/// `"tag_name":"v0.3.0"` dans la réponse JSON de l'API GitHub.
// shortcut: recherche de texte au lieu de serde_json, suffit pour un seul champ plat.
pub fn tag_from_json(js: &str) -> Option<String> {
    let rest = &js[js.find("\"tag_name\"")? + 10..];
    let rest = &rest[rest.find('"')? + 1..];
    Some(rest[..rest.find('"')?].to_string())
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    cmp_version(tag.trim_start_matches('v'), current).is_gt()
}

/// Tag de la dernière release si elle est plus récente que cette version.
pub fn newer_release(agent: &ureq::Agent) -> Option<String> {
    let js = agent
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .ok()?;
    tag_from_json(&js).filter(|t| is_newer(t, env!("CARGO_PKG_VERSION")))
}

fn old_path(exe: &std::path::Path) -> PathBuf {
    let mut p = exe.as_os_str().to_owned();
    p.push(".old");
    p.into()
}

/// Supprime l'exe remplacé lors d'une mise à jour précédente.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

/// Télécharge la release `tag`, vérifie son SHA-256, puis remplace l'exe en cours.
pub fn apply(agent: &ureq::Agent, tag: &str) -> Result<String, String> {
    let base = format!("https://github.com/{REPO}/releases/download/{tag}/mesdrivers.exe");
    let get = |url: &str| -> Result<Vec<u8>, String> {
        agent
            .get(url)
            .call()
            .and_then(|mut r| r.body_mut().with_config().limit(200 << 20).read_to_vec())
            .map_err(|e| format!("Téléchargement {url} : {e}"))
    };
    let exe_bytes = get(&base)?;
    let expected = String::from_utf8_lossy(&get(&format!("{base}.sha256"))?).split_whitespace().next().unwrap_or("").to_lowercase();
    let actual = format!("{:x}", Sha256::digest(&exe_bytes));
    if expected != actual {
        return Err("Mise à jour refusée : l'empreinte SHA-256 ne correspond pas.".into());
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    // Windows autorise à renommer un exe en cours d'exécution, pas à l'écraser.
    std::fs::rename(&exe, &old).map_err(|e| format!("Renommage impossible : {e}"))?;
    if let Err(e) = std::fs::write(&exe, &exe_bytes) {
        let _ = std::fs::rename(&old, &exe); // on remet l'ancienne version en place
        return Err(format!("Écriture impossible : {e}"));
    }
    Ok(format!("Mis à jour en {tag}. Quittez (q) et relancez mesdrivers."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(tag_from_json(r#"{"url":"x","tag_name": "v0.3.1","name":"v0.3.1"}"#), Some("v0.3.1".into()));
        assert_eq!(tag_from_json("{}"), None);
        assert!(is_newer("v0.3.1", "0.3.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("v0.2.9", "0.3.0"));
    }

    #[test]
    #[ignore]
    fn live_latest_tag() {
        let js = crate::catalog::agent()
            .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
            .call()
            .and_then(|mut r| r.body_mut().read_to_string())
            .unwrap();
        assert!(tag_from_json(&js).unwrap().starts_with('v'));
    }
}

