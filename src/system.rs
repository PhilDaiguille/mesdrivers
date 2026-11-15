//! Résumé de la configuration (WMI).
use std::collections::HashMap;
use wmi::{Variant, WMIConnection};

fn text(v: Option<&Variant>) -> String {
    match v {
        Some(Variant::String(s)) => s.trim().to_string(),
        Some(Variant::UI1(n)) => n.to_string(),
        Some(Variant::UI2(n)) => n.to_string(),
        Some(Variant::UI4(n)) => n.to_string(),
        Some(Variant::UI8(n)) => n.to_string(),
        Some(Variant::I4(n)) => n.to_string(),
        Some(Variant::I8(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Les uint64 WMI arrivent souvent en texte.
fn bytes(v: Option<&Variant>) -> String {
    text(v).parse::<u64>().map(crate::model::human_size).unwrap_or_default()
}

pub fn summary(con: &WMIConnection) -> Vec<(String, String)> {
    let q = |sql: &str| con.raw_query::<HashMap<String, Variant>>(sql).unwrap_or_default();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |k: &str, v: String| {
        if !v.trim().is_empty() {
            out.push((k.to_string(), v));
        }
    };

    for o in q("SELECT Caption, Version, OSArchitecture FROM Win32_OperatingSystem") {
        push("Windows", format!("{} {} ({})", text(o.get("Caption")), text(o.get("Version")), text(o.get("OSArchitecture"))));
    }
    for c in q("SELECT Manufacturer, Model, TotalPhysicalMemory FROM Win32_ComputerSystem") {
        push("Ordinateur", format!("{} {}", text(c.get("Manufacturer")), text(c.get("Model"))));
        push("Mémoire", bytes(c.get("TotalPhysicalMemory")));
    }
    for p in q("SELECT Name, NumberOfCores, NumberOfLogicalProcessors FROM Win32_Processor") {
        push(
            "Processeur",
            format!("{} ({} cœurs / {} threads)", text(p.get("Name")), text(p.get("NumberOfCores")), text(p.get("NumberOfLogicalProcessors"))),
        );
    }
    for b in q("SELECT Manufacturer, Product FROM Win32_BaseBoard") {
        push("Carte mère", format!("{} {}", text(b.get("Manufacturer")), text(b.get("Product"))));
    }
    for b in q("SELECT Manufacturer, SMBIOSBIOSVersion, ReleaseDate FROM Win32_BIOS") {
        push(
            "BIOS",
            format!("{} {} du {}", text(b.get("Manufacturer")), text(b.get("SMBIOSBIOSVersion")), crate::model::wmi_date(&text(b.get("ReleaseDate")))),
        );
    }
    for v in q("SELECT Name, DriverVersion FROM Win32_VideoController") {
        push("Carte graphique", format!("{} (pilote {})", text(v.get("Name")), text(v.get("DriverVersion"))));
    }
    for d in q("SELECT Model, Size FROM Win32_DiskDrive") {
        push("Disque", format!("{} ({})", text(d.get("Model")), bytes(d.get("Size"))));
    }
    out
}
