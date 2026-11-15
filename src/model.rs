//! Données affichées et petites conversions pures (compilé et testé sur toutes les plateformes).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub provider: String,
    pub version: String,
    pub date: String,
    pub hwid: String,
    /// oemNN.inf utilisé (lien avec le DriverStore).
    pub inf: String,
    pub catalog: Catalog,
}

/// Résultat de la vérification dans le Catalogue Microsoft Update.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum Catalog {
    /// Pas vérifié (Microsoft, bus ignoré) ou pas encore de réponse.
    #[default]
    Skipped,
    Pending,
    NotFound,
    Found { id: String, version: String, date: String, title: String },
    Error(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrokenDevice {
    pub name: String,
    pub code: u32,
    /// HardwareID le plus précis (vide si Windows n'en donne pas).
    pub hwid: String,
    pub catalog: Catalog,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Index dans la dernière recherche Windows Update.
    Wu(usize),
    /// Ids du Catalogue Microsoft : Intel publie le même paquet sous un id par matériel visé.
    Catalog(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Update {
    pub source: Source,
    pub title: String,
    pub provider: String,
    pub hwid: String,
    pub date: String,
    /// Connue pour le Catalogue, vide pour Windows Update (elle est dans le titre).
    pub version: String,
    /// Périphériques concernés (Catalogue).
    pub devices: Vec<String>,
}

impl Update {
    /// Clé stable pour mémoriser les cases cochées pendant que la liste se remplit.
    pub fn key(&self) -> String {
        match &self.source {
            Source::Wu(i) => format!("wu:{i}"),
            Source::Catalog(_) => format!("cat:{}|{}", self.title, self.version),
        }
    }
}

/// Paquet de pilote tiers présent dans le DriverStore (oemNN.inf).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Package {
    pub oem: String,
    /// Chemin complet de l'INF dans FileRepository (pour un retour arrière).
    pub path: String,
    pub original: String,
    pub provider: String,
    pub class: String,
    pub version: String,
    pub date: String,
    pub size: u64,
    pub in_use: bool,
    pub obsolete: bool,
}

/// Mises à jour Catalogue plus récentes que l'installé, regroupées par (titre, version) :
/// 6 contrôleurs I2C qui partagent le même pilote donnent une seule ligne.
pub fn catalog_updates(devices: &[Device], broken: &[BrokenDevice]) -> Vec<Update> {
    // Un périphérique sans pilote = version installée vide : toute version trouvée est « plus récente ».
    let missing = broken.iter().map(|b| Device {
        name: format!("{} (sans pilote)", b.name),
        hwid: b.hwid.clone(),
        catalog: b.catalog.clone(),
        ..Default::default()
    });
    let all: Vec<Device> = devices.iter().cloned().chain(missing).collect();
    let mut out: Vec<Update> = Vec::new();
    for d in &all {
        let Catalog::Found { id, version, date, title } = &d.catalog else { continue };
        if !cmp_version(version, &d.version).is_gt() {
            continue;
        }
        match out.iter_mut().find(|u| u.title == *title && u.version == *version) {
            Some(u) => {
                if let Source::Catalog(ids) = &mut u.source
                    && !ids.contains(id)
                {
                    ids.push(id.clone());
                }
                u.devices.push(d.name.clone());
            }
            None => out.push(Update {
                source: Source::Catalog(vec![id.clone()]),
                title: title.clone(),
                provider: d.provider.clone(),
                hwid: d.hwid.clone(),
                date: date.clone(),
                version: version.clone(),
                devices: vec![d.name.clone()],
            }),
        }
    }
    out
}

/// Le Catalogue a une version plus récente que celle installée.
pub fn has_newer(d: &Device) -> bool {
    matches!(&d.catalog, Catalog::Found { version, .. } if cmp_version(version, &d.version).is_gt())
}

/// Les INF chipset Intel sont datés 1968 exprès (pour ne jamais passer devant un vrai pilote).
pub fn display_date(d: &str) -> String {
    match d.get(..4).and_then(|y| y.parse::<u32>().ok()) {
        Some(y) if y < 1990 => "non datée".into(),
        _ => d.to_string(),
    }
}

/// Obsolète = inutilisé ET une version plus récente du même INF d'origine est en service.
/// Un paquet seul (ex. imprimante débranchée) n'est donc jamais proposé à la suppression.
pub fn mark_obsolete(pkgs: &mut [Package]) {
    let used: Vec<(String, String)> =
        pkgs.iter().filter(|p| p.in_use).map(|p| (p.original.to_lowercase(), p.version.clone())).collect();
    for p in pkgs.iter_mut() {
        let orig = p.original.to_lowercase();
        p.obsolete = !p.in_use && used.iter().any(|(o, v)| *o == orig && cmp_version(v, &p.version).is_gt());
    }
}

/// 123456789 -> "117,7 Mo"
pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    let (v, unit) = if b >= 1e9 { (b / 1073741824.0, "Go") } else if b >= 1e6 { (b / 1048576.0, "Mo") } else { (b / 1024.0, "Ko") };
    format!("{v:.1} {unit}").replace('.', ",")
}

/// "20230115000000.000000-000" (CIM_DATETIME) -> "2023-01-15"
pub fn wmi_date(s: &str) -> String {
    match (s.get(0..4), s.get(4..6), s.get(6..8)) {
        (Some(y), Some(m), Some(d)) if s[..8].bytes().all(|b| b.is_ascii_digit()) => format!("{y}-{m}-{d}"),
        _ => String::new(),
    }
}

/// DATE OLE (jours depuis 1899-12-30) -> "AAAA-MM-JJ"
pub fn ole_date(days: f64) -> String {
    // Algorithme civil_from_days de Howard Hinnant, base 1970-01-01 = 25569 en DATE OLE.
    let z = days.floor() as i64 - 25569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Version actuellement installée pour le matériel visé par une mise à jour.
// shortcut: compare au seul HardwareID principal de WMI (le plus précis) par préfixe ;
// passer par SetupAPI (toutes les HardwareIDs) si des correspondances manquent.
pub fn current_version<'a>(devices: &'a [Device], update_hwid: &str) -> Option<&'a str> {
    let want = update_hwid.to_ascii_lowercase();
    if want.is_empty() {
        return None;
    }
    devices
        .iter()
        .find(|d| d.hwid.to_ascii_lowercase().starts_with(&want))
        .map(|d| d.version.as_str())
}

/// Compare deux versions "a.b.c.d" segment par segment (numérique, segments absents = 0).
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| s.trim().split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parse(a), parse(b));
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Index des lignes à afficher : `rows` = (texte, fourni par Microsoft ?).
pub fn visible<'a>(rows: impl Iterator<Item = (&'a str, bool)>, query: &str, hide_ms: bool) -> Vec<usize> {
    let q = query.to_lowercase();
    rows.enumerate()
        .filter(|(_, (text, ms))| !(hide_ms && *ms) && (q.is_empty() || text.to_lowercase().contains(&q)))
        .map(|(i, _)| i)
        .collect()
}

/// Secondes Unix -> "AAAA-MM-JJ HH:MM UTC" (pour le journal).
pub fn stamp(unix_secs: u64) -> String {
    let (day, rem) = (unix_secs / 86_400, unix_secs % 86_400);
    format!("{} {:02}:{:02} UTC", ole_date(day as f64 + 25569.0), rem / 3600, rem % 3600 / 60)
}

/// Code d'erreur du Gestionnaire de périphériques -> libellé court.
pub fn error_label(code: u32) -> &'static str {
    match code {
        1 => "mal configuré",
        10 => "ne peut pas démarrer",
        18 | 28 => "pilote non installé",
        22 => "désactivé",
        31 => "ne fonctionne pas correctement",
        43 => "arrêté (problème signalé)",
        45 => "non connecté",
        52 => "signature du pilote invalide",
        _ => "en erreur",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(wmi_date("20230115000000.******+***"), "2023-01-15");
        assert_eq!(wmi_date(""), "");
        assert_eq!(wmi_date("abc"), "");
        assert_eq!(ole_date(25569.0), "1970-01-01");
        assert_eq!(ole_date(45000.5), "2023-03-15");
        assert_eq!(ole_date(36526.0), "2000-01-01");
    }

    #[test]
    fn versions() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_version("31.0.101.5590", "31.0.101.4502"), Greater);
        assert_eq!(cmp_version("24.40.0.4", "24.40.0.4"), Equal);
        assert_eq!(cmp_version("1.2", "1.2.0.0"), Equal);
        assert_eq!(cmp_version("1.10", "1.9"), Greater);
        assert_eq!(cmp_version("abc", "0.0.1"), Less);
    }

    #[test]
    fn filtering() {
        let rows = [("Intel Wi-Fi", false), ("Volume", true), ("Realtek Audio", false)];
        assert_eq!(visible(rows.into_iter(), "", false), [0, 1, 2]);
        assert_eq!(visible(rows.into_iter(), "", true), [0, 2]);
        assert_eq!(visible(rows.into_iter(), "AUDIO", true), [2]);
        assert_eq!(visible(rows.into_iter(), "vol", true), [] as [usize; 0]);
    }

    #[test]
    fn obsolete_rule() {
        let p = |oem: &str, orig: &str, v: &str, used: bool| Package {
            oem: oem.into(),
            original: orig.into(),
            version: v.into(),
            in_use: used,
            ..Default::default()
        };
        let mut pkgs = vec![
            p("oem1.inf", "nvlti.inf", "32.0.15.5590", true),
            p("oem2.inf", "nvlti.inf", "31.0.15.3000", false), // ancienne NVIDIA -> obsolète
            p("oem3.inf", "NVLTI.INF", "33.0.0.1", false),     // plus récente mais inutilisée -> garder
            p("oem4.inf", "printer.inf", "1.0", false),        // seule, appareil débranché -> garder
            p("oem5.inf", "netwtw.inf", "1.0", true),
        ];
        mark_obsolete(&mut pkgs);
        assert_eq!(pkgs.iter().map(|p| p.obsolete).collect::<Vec<_>>(), [false, true, false, false, false]);
        assert_eq!(human_size(1_500_000_000), "1,4 Go");
        assert_eq!(human_size(2048), "2,0 Ko");
    }

    #[test]
    fn groups_catalog_updates() {
        let dev = |name: &str, id: &str, title: &str, cur: &str, new: &str| Device {
            name: name.into(),
            version: cur.into(),
            catalog: Catalog::Found { id: id.into(), version: new.into(), date: "2025-07-29".into(), title: title.into() },
            ..Default::default()
        };
        let t = "Intel Corporation System Driver Update (30.100.2531.31)";
        let devs = vec![
            dev("I2C 7A4C", "a", t, "30.100.2417.30", "30.100.2531.31"),
            dev("I2C 7A4D", "b", t, "30.100.2417.30", "30.100.2531.31"),
            dev("I2C 7A4E", "b", t, "30.100.2417.30", "30.100.2531.31"),
            dev("PEG10", "c", "INTEL System Driver Update (10.1.45.10)", "10.1.45.9", "10.1.45.10"),
            dev("SMBus", "d", "déjà à jour", "10.1.46.5", "10.1.46.5"),
        ];
        let broken = [BrokenDevice {
            name: "Inconnu".into(),
            code: 28,
            hwid: r"ACPI\INT3472".into(),
            catalog: Catalog::Found { id: "e".into(), version: "1.0".into(), date: "2025-01-01".into(), title: "Intel Camera".into() },
        }];
        assert_eq!(catalog_updates(&devs, &broken).len(), 3);
        assert_eq!(catalog_updates(&devs, &broken)[2].devices, ["Inconnu (sans pilote)"]);
        let u = catalog_updates(&devs, &[]);
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].source, Source::Catalog(vec!["a".into(), "b".into()]));
        assert_eq!(u[0].devices.len(), 3);
        assert_eq!(u[0].key(), format!("cat:{t}|30.100.2531.31"));
        assert_eq!(display_date("1968-07-18"), "non datée");
        assert_eq!(display_date("2025-07-29"), "2025-07-29");
        assert_eq!(display_date(""), "");
    }

    #[test]
    fn journal_stamp() {
        assert_eq!(stamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(stamp(1_763_221_800), "2025-11-15 15:50 UTC");
    }

    #[test]
    fn matching() {
        let devs = vec![Device {
            hwid: r"PCI\VEN_8086&DEV_A0E8&SUBSYS_0A1F1028&REV_20".into(),
            version: "1.2.3".into(),
            ..Default::default()
        }];
        assert_eq!(current_version(&devs, r"pci\ven_8086&dev_a0e8"), Some("1.2.3"));
        assert_eq!(current_version(&devs, r"PCI\VEN_8086"), Some("1.2.3"));
        assert_eq!(current_version(&devs, r"USB\VID_046D"), None);
        assert_eq!(current_version(&devs, ""), None);
    }
}
