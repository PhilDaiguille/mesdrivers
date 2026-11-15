//! Thread de travail : tout le COM (WMI + Windows Update) et le réseau vivent hors de l'UI.
use crate::model::{BrokenDevice, Catalog, Device, Update};
use std::sync::mpsc::{Receiver, Sender, channel};

pub enum Cmd {
    Scan,
    Install { picks: Vec<usize>, skip_restore: bool },
}

pub enum Msg {
    Inventory(Vec<Device>, Vec<BrokenDevice>),
    Updates(Vec<Update>),
    /// Résultat Catalogue pour un HardwareID (appliqué à tous les périphériques qui l'ont).
    Catalog(String, Catalog),
    Progress(String),
    /// Le point de restauration a échoué : l'utilisateur doit confirmer pour continuer.
    NeedForce(String),
    Installed(String),
    Error(String),
    /// Fin de la commande en cours.
    Done,
}

pub fn spawn() -> (Sender<Cmd>, Receiver<Msg>) {
    let (cmd_tx, cmd_rx) = channel();
    let (msg_tx, msg_rx) = channel();
    std::thread::spawn(move || run(cmd_rx, msg_tx));
    (cmd_tx, msg_rx)
}

/// Lance la vérification Catalogue de chaque HardwareID utile sur 4 threads ; les réponses arrivent au fil de l'eau.
pub fn check_catalog(devices: &mut [Device], tx: &Sender<Msg>) {
    let mut hwids: Vec<String> = Vec::new();
    for d in devices.iter_mut().filter(|d| crate::catalog::worth_checking(&d.hwid, &d.provider)) {
        d.catalog = Catalog::Pending;
        if !hwids.contains(&d.hwid) {
            hwids.push(d.hwid.clone());
        }
    }
    let agent = crate::catalog::agent();
    for t in 0..4 {
        let mine: Vec<String> = hwids.iter().skip(t).step_by(4).cloned().collect();
        let (tx, agent) = (tx.clone(), agent.clone());
        std::thread::spawn(move || {
            for h in mine {
                let r = crate::catalog::check(&agent, &h);
                if tx.send(Msg::Catalog(h, r)).is_err() {
                    return; // l'UI est fermée
                }
            }
        });
    }
}

#[cfg(not(windows))]
fn run(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    for _ in rx {
        let _ = tx.send(Msg::Error("Cette application ne fonctionne que sous Windows.".into()));
        let _ = tx.send(Msg::Done);
    }
}

#[cfg(not(windows))]
pub fn cli_scan() -> i32 {
    eprintln!("--scan ne fonctionne que sous Windows.");
    1
}

#[cfg(windows)]
fn com_init() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()
}

#[cfg(windows)]
fn restore_point() -> Result<(), String> {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Checkpoint-Computer -Description 'MesDrivers' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop",
        ])
        .output()
        .map_err(|e| format!("PowerShell introuvable : {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("échec inconnu").trim().to_string())
}

#[cfg(windows)]
fn run(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    use crate::wu::Wu;
    // Les objets COM (IUpdate) ne sont pas Send : Windows Update reste dans ce thread.
    let mut wu = match com_init().and_then(|_| Wu::new()) {
        Ok(w) => w,
        Err(e) => {
            let _ = tx.send(Msg::Error(format!("Windows Update indisponible : {e}")));
            let _ = tx.send(Msg::Done);
            return;
        }
    };
    let search = |wu: &mut Wu| {
        let _ = tx.send(match wu.search() {
            Ok(u) => Msg::Updates(u),
            Err(e) => Msg::Error(format!("Recherche Windows Update : {e}")),
        });
    };

    for cmd in rx {
        match cmd {
            Cmd::Scan => {
                // WMI dans son propre thread pendant que Windows Update cherche : durée = max des deux.
                let inv_tx = tx.clone();
                let inv = std::thread::spawn(move || {
                    let r = wmi::WMIConnection::new().and_then(|c| crate::inventory::scan(&c));
                    match r {
                        Ok((mut d, b)) => {
                            check_catalog(&mut d, &inv_tx);
                            let _ = inv_tx.send(Msg::Inventory(d, b));
                        }
                        Err(e) => {
                            let _ = inv_tx.send(Msg::Error(format!("Inventaire WMI : {e}")));
                        }
                    }
                });
                search(&mut wu);
                if inv.join().is_err() {
                    let _ = tx.send(Msg::Error("Inventaire WMI : erreur interne".into()));
                }
            }
            Cmd::Install { picks, skip_restore } => {
                if !skip_restore {
                    let _ = tx.send(Msg::Progress("Création du point de restauration…".into()));
                    if let Err(e) = restore_point() {
                        let _ = tx.send(Msg::NeedForce(e));
                        let _ = tx.send(Msg::Done);
                        continue;
                    }
                }
                let n = picks.len();
                let (mut ok, mut reboot, mut errors) = (0, false, Vec::new());
                for (k, &i) in picks.iter().enumerate() {
                    let title = wu.title(i);
                    let _ = tx.send(Msg::Progress(format!("{}/{n} : {title}…", k + 1)));
                    let line = match wu.install_one(i) {
                        Ok((true, r)) => {
                            ok += 1;
                            reboot |= r;
                            format!("OK      {title}")
                        }
                        Ok((false, _)) => format!("ÉCHEC   {title}"),
                        Err(e) => format!("ERREUR  {title} : {e}"),
                    };
                    if let Err(e) = crate::journal::log(&line) {
                        errors.push(format!("journal : {e}"));
                    }
                }
                let mut s = format!("{ok}/{n} pilote(s) installé(s).");
                if reboot {
                    s += " Redémarrage nécessaire.";
                }
                if !errors.is_empty() {
                    s += &format!(" ({})", errors[0]);
                }
                let _ = tx.send(Msg::Installed(s));
                search(&mut wu); // la liste change après installation
            }
        }
        let _ = tx.send(Msg::Done);
    }
}

/// `mesdrivers --scan` : tout le moteur sans interface, avec les durées. Sert de test réel en CI.
#[cfg(windows)]
pub fn cli_scan() -> i32 {
    use std::time::Instant;
    let mut code = 0;
    let t = Instant::now();
    let inv = std::thread::spawn(|| {
        let t = Instant::now();
        let r = wmi::WMIConnection::new().and_then(|c| crate::inventory::scan(&c)).map_err(|e| e.to_string());
        (r, t.elapsed())
    });
    let wu_t = Instant::now();
    let wu = com_init().and_then(|_| crate::wu::Wu::new()).and_then(|mut w| w.search());
    let wu_t = wu_t.elapsed();
    let (inv, inv_t) = inv.join().unwrap_or((Err("thread WMI planté".into()), Default::default()));

    let mut devices = match inv {
        Ok((d, b)) => {
            println!("== Inventaire WMI : {} pilotes, {} en erreur ({inv_t:.1?})", d.len(), b.len());
            for x in &b {
                println!("  ! {} (code {} : {})", x.name, x.code, crate::model::error_label(x.code));
            }
            for x in d.iter().filter(|x| !x.provider.eq_ignore_ascii_case("Microsoft")) {
                println!("  {} | {} | {} | {}", x.name, x.provider, x.version, x.date);
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

    let cat_t = Instant::now();
    let (tx, rx) = channel();
    check_catalog(&mut devices, &tx);
    drop(tx);
    let results: Vec<(String, Catalog)> = rx.into_iter().filter_map(|m| if let Msg::Catalog(h, c) = m { Some((h, c)) } else { None }).collect();
    println!("== Catalogue Microsoft : {} matériel(s) vérifié(s) ({:.1?})", results.len(), cat_t.elapsed());
    for (h, c) in &results {
        println!("  {h} -> {c:?}");
    }
    println!("== Total {:.1?}", t.elapsed());
    code
}
