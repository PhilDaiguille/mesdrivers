//! Données affichées et petites conversions pures (compilé et testé sur toutes les plateformes).

#[derive(Debug, Clone, Default)]
pub struct Device {
    pub name: String,
    pub provider: String,
    pub version: String,
    pub date: String,
    pub hwid: String,
}

#[derive(Debug, Clone)]
pub struct BrokenDevice {
    pub name: String,
    pub code: u32,
}

#[derive(Debug, Clone)]
pub struct Update {
    pub title: String,
    pub provider: String,
    pub hwid: String,
    pub date: String,
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
