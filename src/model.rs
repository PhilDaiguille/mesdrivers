//! Données affichées et petites conversions pures (compilé et testé sur toutes les plateformes).

#[derive(Debug, Clone, Default)]
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
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Catalog {
    /// Pas vérifié (Microsoft, bus ignoré) ou pas encore de réponse.
    #[default]
    Skipped,
    Pending,
    NotFound,
    Found { id: String, version: String, date: String, title: String },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct BrokenDevice {
    pub name: String,
    pub code: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Index dans la dernière recherche Windows Update.
    Wu(usize),
    /// Id de mise à jour du Catalogue Microsoft.
    Catalog(String),
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
}

impl Update {
    /// Clé stable pour mémoriser les cases cochées pendant que la liste se remplit.
    pub fn key(&self) -> String {
        match &self.source {
            Source::Wu(i) => format!("wu:{i}"),
            Source::Catalog(id) => format!("cat:{id}"),
        }
    }
}

/// Paquet de pilote tiers présent dans le DriverStore (oemNN.inf).
#[derive(Debug, Clone, Default)]
pub struct Package {
    pub oem: String,
    pub original: String,
    pub provider: String,
    pub class: String,
    pub version: String,
    pub date: String,
    pub size: u64,
    pub in_use: bool,
    pub obsolete: bool,
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
