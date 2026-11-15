//! DriverStore : paquets tiers (C:\Windows\INF\oem*.inf), lecture des INF.
#[cfg(windows)]
use crate::model::Package;
use std::path::Path;

/// Lit un fichier texte INF : UTF-16 LE (BOM FF FE, fréquent) ou UTF-8/ANSI.
pub fn read_text(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> String {
    if let [0xFF, 0xFE, rest @ ..] = bytes {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).into_owned()
}

/// Valeur `key` de la section `section` (insensible à la casse), commentaires et guillemets retirés.
fn value(inf: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in inf.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line.trim_matches(|c| c == '[' || c == ']').trim().eq_ignore_ascii_case(section);
            continue;
        }
        if !inside {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim().eq_ignore_ascii_case(key) {
            let v = if v.matches('"').count() >= 2 { v } else { v.split(';').next().unwrap_or("") };
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Résout `%Cle%` via la section [Strings].
fn resolve(inf: &str, v: String) -> String {
    match v.strip_prefix('%').and_then(|k| k.strip_suffix('%')) {
        Some(key) => value(inf, "Strings", key).unwrap_or(v),
        None => v,
    }
}

/// (fabricant, classe, date AAAA-MM-JJ, version) depuis [Version].
pub fn version_info(inf: &str) -> (String, String, String, String) {
    let provider = value(inf, "Version", "Provider").map(|v| resolve(inf, v)).unwrap_or_default();
    let class = value(inf, "Version", "Class").unwrap_or_default();
    let dv = value(inf, "Version", "DriverVer").unwrap_or_default();
    let (date, version) = dv.split_once(',').unwrap_or((&dv, ""));
    let date = match date.trim().split('/').collect::<Vec<_>>()[..] {
        [m, d, y] => format!("{y}-{m:0>2}-{d:0>2}"),
        _ => String::new(),
    };
    (provider, class, date, version.trim().to_string())
}

/// "nvlti.inf_amd64_8a5bd3c3e8f8f7c9" -> "nvlti.inf"
pub fn original_name(store_dir: &str) -> String {
    let lower = store_dir.to_lowercase();
    match lower.find(".inf_") {
        Some(i) => lower[..i + 4].to_string(),
        None => lower,
    }
}

/// Chemin de l'INF du paquet dans FileRepository (API SetupAPI).
#[cfg(windows)]
fn store_location(oem_inf: &Path) -> Option<std::path::PathBuf> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::SetupGetInfDriverStoreLocationW;
    use windows::core::{HSTRING, PCWSTR};
    let mut buf = [0u16; 1024];
    let mut needed = 0u32;
    unsafe {
        SetupGetInfDriverStoreLocationW(&HSTRING::from(oem_inf.as_os_str()), None, PCWSTR::null(), &mut buf, Some(&mut needed)).ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// Installe de force `inf` pour les périphériques `hwid`, même si la version est plus ancienne.
/// Renvoie true si un redémarrage est nécessaire.
#[cfg(windows)]
pub fn force_install(hwid: &str, inf: &str) -> Result<bool, String> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{INSTALLFLAG_FORCE, UpdateDriverForPlugAndPlayDevicesW};
    use windows::core::{BOOL, HSTRING};
    let mut reboot = BOOL(0);
    unsafe { UpdateDriverForPlugAndPlayDevicesW(None, &HSTRING::from(hwid), &HSTRING::from(inf), INSTALLFLAG_FORCE, Some(&mut reboot)) }
        .map_err(|e| e.message())?;
    Ok(reboot.as_bool())
}

/// Inventaire du DriverStore. `in_use` = noms oemNN.inf utilisés par un périphérique présent (WMI InfName).
#[cfg(windows)]
pub fn scan(in_use: &[String]) -> Vec<Package> {
    let windir = std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from);
    let mut pkgs: Vec<Package> = std::fs::read_dir(windir.join("INF"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.starts_with("oem") && n.ends_with(".inf")
        })
        .map(|p| {
            let oem = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            let (provider, class, date, version) = version_info(&read_text(&p));
            let inf = store_location(&p);
            let dir = inf.as_deref().and_then(Path::parent);
            Package {
                path: inf.as_deref().map(|i| i.display().to_string()).unwrap_or_default(),
                in_use: in_use.iter().any(|u| u.eq_ignore_ascii_case(&oem)),
                original: dir.and_then(|d| d.file_name()).map_or_else(|| oem.clone(), |n| original_name(&n.to_string_lossy())),
                size: dir.map_or(0, crate::pnputil::dir_size),
                oem,
                provider,
                class,
                date,
                version,
                obsolete: false,
            }
        })
        .collect();
    crate::model::mark_obsolete(&mut pkgs);
    pkgs.sort_by(|a, b| b.obsolete.cmp(&a.obsolete).then(b.size.cmp(&a.size)));
    pkgs
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: &str = "; commentaire\r\n[Version]\r\nSignature=\"$WINDOWS NT$\"\r\nClass=Display ; écran\r\n\
        Provider=%NVIDIA%\r\nDriverVer = 04/12/2026,32.0.15.5590\r\n\r\n[Strings]\r\nNVIDIA = \"NVIDIA; Corp\"\r\n";

    #[test]
    fn reads_inf() {
        assert_eq!(
            version_info(INF),
            ("NVIDIA; Corp".into(), "Display".into(), "2026-04-12".into(), "32.0.15.5590".into())
        );
        assert_eq!(version_info("rien"), (String::new(), String::new(), String::new(), String::new()));
    }

    #[test]
    fn decodes_utf16() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("[Version]\r\nClass=Net".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(version_info(&decode(&bytes)).1, "Net");
        assert_eq!(decode(b"\xEF\xBB\xBFabc"), "abc");
    }

    #[test]
    fn original_names() {
        assert_eq!(original_name("nvlti.inf_amd64_8a5bd3c3e8f8f7c9"), "nvlti.inf");
        assert_eq!(original_name("Netwtw10.INF_amd64_1"), "netwtw10.inf");
        assert_eq!(original_name("bizarre"), "bizarre");
    }
}
