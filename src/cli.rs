//! Mode ligne de commande (sans interface). Sert aussi de test réel en CI.
use crate::model::Catalog;
#[cfg(windows)]
use crate::model::human_size;
#[cfg(windows)]
use std::path::Path;

pub fn usage() -> &'static str {
    "usage : mesdrivers [--scan | --catalog <HardwareID> | --catalog-get <HardwareID> <dossier> | --backup <dossier> | --restore <dossier> | --version]"
}

/// Dernière version Catalogue d'un matériel.
pub fn catalog(hwid: &str) -> i32 {
    let r = crate::catalog::check(&crate::catalog::agent(), hwid);
    println!("{r:?}");
    i32::from(!matches!(r, Catalog::Found { .. }))
}

#[cfg(not(windows))]
pub fn windows_only() -> i32 {
    eprintln!("Cette commande ne fonctionne que sous Windows.");
    1
}

/// Télécharge, vérifie et extrait le meilleur paquet Catalogue, sans l'installer.
#[cfg(windows)]
pub fn catalog_get(hwid: &str, dir: &Path) -> i32 {
    let agent = crate::catalog::agent();
    let Catalog::Found { id, title, version, .. } = crate::catalog::check(&agent, hwid) else {
        eprintln!("Rien trouvé au Catalogue pour {hwid}");
        return 1;
    };
    println!("Trouvé : {title} ({version})");
    match crate::worker::fetch_catalog_package(&agent, &id) {
        Ok(out) => {
            let target = dir.join(&id);
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::remove_dir_all(&target);
            if let Err(e) = std::fs::rename(&out, &target) {
                eprintln!("Copie vers {} impossible : {e}", target.display());
                return 1;
            }
            let files = crate::pnputil::files(&target);
            let infs = files.iter().filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("inf"))).count();
            println!("OK : SHA-1 vérifié, {} fichiers dont {infs} .inf pour cette architecture -> {}", files.len(), target.display());
            0
        }
        Err(e) => {
            eprintln!("ERREUR : {e}");
            1
        }
    }
}

#[cfg(windows)]
pub fn backup(dir: &Path) -> i32 {
    match crate::pnputil::export_all(dir) {
        Ok(()) => {
            let n = crate::pnputil::files(dir).iter().filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("inf"))).count();
            println!("{n} pilote(s) exporté(s) ({}) dans {}", human_size(crate::pnputil::dir_size(dir)), dir.display());
            i32::from(n == 0)
        }
        Err(e) => {
            eprintln!("Export échoué : {e}");
            1
        }
    }
}

#[cfg(windows)]
pub fn restore(dir: &Path) -> i32 {
    if let Err(e) = crate::worker::restore_point() {
        eprintln!("Point de restauration impossible : {e}. Abandon.");
        return 1;
    }
    match crate::pnputil::add_dir(dir) {
        Ok(reboot) => {
            println!("Pilotes restaurés.{}", if reboot { " Redémarrage nécessaire." } else { "" });
            0
        }
        Err(e) => {
            eprintln!("Restauration : {e}");
            1
        }
    }
}

/// `--scan` : tout le moteur, avec la durée de chaque phase.
#[cfg(windows)]
pub fn scan() -> i32 {
    use std::time::Instant;
    let mut code = 0;
    let t = Instant::now();
    let inv = std::thread::spawn(|| {
        let t = Instant::now();
        let r = wmi::WMIConnection::new()
            .and_then(|c| crate::inventory::scan(&c).map(|r| (r, crate::system::summary(&c))))
            .map_err(|e| e.to_string());
        (r, t.elapsed())
    });
    let wu_t = Instant::now();
    let wu = crate::worker::com_init().and_then(|_| crate::wu::Wu::new()).and_then(|mut w| w.search());
    let wu_t = wu_t.elapsed();
    let (inv, inv_t) = inv.join().unwrap_or((Err("thread WMI planté".into()), Default::default()));

    let mut devices = match inv {
        Ok(((d, b), summary)) => {
            println!("== Résumé");
            for (k, v) in &summary {
                println!("  {k:<16} {v}");
            }
            println!("== Inventaire WMI : {} pilotes, {} en erreur ({inv_t:.1?})", d.len(), b.len());
            for x in &b {
                println!("  ! {} (code {} : {})", x.name, x.code, crate::model::error_label(x.code));
            }
            for x in d.iter().filter(|x| !x.provider.eq_ignore_ascii_case("Microsoft")) {
                println!("  {} | {} | {} | {} | {}", x.name, x.provider, x.version, x.date, x.inf);
            }
            d
        }
        Err(e) => {
            println!("== Inventaire WMI : ERREUR {e}");
            code = 1;
            Vec::new()
        }
    };
    match wu {
        Ok(u) => {
            println!("== Windows Update : {} pilote(s) proposé(s) ({wu_t:.1?})", u.len());
            for x in &u {
                println!("  {} | {} | {}", x.title, x.hwid, x.date);
            }
        }
        Err(e) => {
            println!("== Windows Update : ERREUR {e}");
            code = 1;
        }
    }

    let st = Instant::now();
    let in_use: Vec<String> = devices.iter().map(|d| d.inf.clone()).collect();
    let pkgs = crate::store::scan(&in_use);
    let reclaim: u64 = pkgs.iter().filter(|p| p.obsolete).map(|p| p.size).sum();
    println!(
        "== DriverStore : {} paquet(s), {} en service, {} obsolète(s), {} récupérables ({:.1?})",
        pkgs.len(),
        pkgs.iter().filter(|p| p.in_use).count(),
        pkgs.iter().filter(|p| p.obsolete).count(),
        human_size(reclaim),
        st.elapsed()
    );
    for p in &pkgs {
        println!("  {} | {} | {} | {} | {} | {}", p.oem, p.original, p.provider, p.version, human_size(p.size), if p.in_use { "en service" } else if p.obsolete { "OBSOLÈTE" } else { "inutilisé" });
    }

    let cat_t = Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    crate::worker::check_catalog(&mut devices, &tx);
    drop(tx);
    let results: Vec<(String, Catalog)> =
        rx.into_iter().filter_map(|m| if let crate::worker::Msg::Catalog(h, c) = m { Some((h, c)) } else { None }).collect();
    println!("== Catalogue Microsoft : {} matériel(s) vérifié(s) ({:.1?})", results.len(), cat_t.elapsed());
    for (h, c) in &results {
        println!("  {h} -> {c:?}");
    }
    println!("== Total {:.1?}", t.elapsed());
    code
}
