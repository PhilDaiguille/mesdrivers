//! Dernier scan gardé sur disque : affiché instantanément au lancement, puis rafraîchi.
use crate::model::{BrokenDevice, Device, Package};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct Cache {
    pub saved: String,
    pub summary: Vec<(String, String)>,
    pub devices: Vec<Device>,
    pub broken: Vec<BrokenDevice>,
    pub store: Vec<Package>,
}

fn path() -> std::path::PathBuf {
    crate::journal::path().with_file_name("cache.json")
}

/// Un cache absent, illisible ou d'une autre version est simplement ignoré.
pub fn load() -> Option<Cache> {
    serde_json::from_slice(&std::fs::read(path()).ok()?).ok()
}

pub fn save(c: &Cache) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(js) = serde_json::to_vec(c) {
        // Écriture atomique : un fichier à moitié écrit ne doit jamais remplacer le bon.
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, js).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = Cache {
            saved: "2026-11-15 18:00 UTC".into(),
            devices: vec![Device { name: "Intel Wi-Fi".into(), catalog: crate::model::Catalog::NotFound, ..Default::default() }],
            ..Default::default()
        };
        let back: Cache = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        assert_eq!(back.devices[0].name, "Intel Wi-Fi");
        assert_eq!(back.devices[0].catalog, crate::model::Catalog::NotFound);
    }
}
