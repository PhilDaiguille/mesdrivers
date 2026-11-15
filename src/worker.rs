//! Thread de travail : tout le COM (WMI + Windows Update), le réseau et les outils Windows vivent hors de l'UI.
use crate::model::{BrokenDevice, Catalog, Device, Package, Update};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Debug)]
pub enum Cmd {
    Scan,
    /// `catalog` = (id de mise à jour, titre).
    Install { wu: Vec<usize>, catalog: Vec<(String, String)>, skip_restore: bool },
    Backup(PathBuf),
    Restore { dir: PathBuf, skip_restore: bool },
    DeletePackages(Vec<String>),
    SelfUpdate(String),
    /// Réinstalle de force une version présente dans le DriverStore (retour arrière).
    Rollback { hwid: String, inf: String, label: String, skip_restore: bool },
}

pub enum Msg {
    Summary(Vec<(String, String)>),
    Inventory(Vec<Device>, Vec<BrokenDevice>),
    /// Pilotes proposés par Windows Update.
    Updates(Vec<Update>),
    /// Résultat Catalogue pour un HardwareID (appliqué à tous les périphériques qui l'ont).
    Catalog(String, Catalog),
    Store(Vec<Package>),
    NewVersion(String),
    Progress(String),
    /// Avancement d'un téléchargement (0..1), `None` à la fin.
    Gauge(Option<f64>),
    /// Le point de restauration a échoué : la commande à relancer (sans point) si l'utilisateur confirme.
    NeedForce(String, Cmd),
    Status(String),
    /// Fin de la commande en cours.
    Done,
}

pub fn spawn() -> (Sender<Cmd>, Receiver<Msg>) {
    let (cmd_tx, cmd_rx) = channel();
    let (msg_tx, msg_rx) = channel();
    let up_tx = msg_tx.clone();
    std::thread::spawn(move || {
        if let Some(tag) = crate::selfupdate::newer_release(&crate::catalog::agent()) {
            let _ = up_tx.send(Msg::NewVersion(tag));
        }
    });
    std::thread::spawn(move || run(cmd_rx, msg_tx));
    (cmd_tx, msg_rx)
}

/// Lance la vérification Catalogue de chaque HardwareID utile sur 4 threads ; les réponses arrivent au fil de l'eau.
pub fn check_catalog(devices: &mut [Device], broken: &mut [BrokenDevice], tx: &Sender<Msg>) {
    let mut hwids: Vec<String> = Vec::new();
    let mut add = |hwid: &String, state: &mut Catalog| {
        *state = Catalog::Pending;
        if !hwids.contains(hwid) {
            hwids.push(hwid.clone());
        }
    };
    for d in devices.iter_mut().filter(|d| crate::catalog::worth_checking(&d.hwid, &d.provider)) {
        add(&d.hwid, &mut d.catalog);
    }
    for b in broken.iter_mut().filter(|b| crate::catalog::worth_checking_missing(&b.hwid)) {
        add(&b.hwid, &mut b.catalog);
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

/// Télécharge (SHA-1 vérifié) et extrait un paquet du Catalogue dans `base\<id>` ;
/// refuse s'il ne vise pas l'architecture du PC.
/// `info` = (URL, empreinte SHA-1) lue par `catalog::download_info`.
pub fn fetch_catalog_package(
    agent: &ureq::Agent,
    id: &str,
    info: &(String, String),
    base: &std::path::Path,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let dir = base.join(id);
    let _ = std::fs::remove_dir_all(&dir);
    let cab = crate::catalog::download(agent, &info.0, &info.1, &dir, progress)?;
    let out = dir.join("pilote");
    crate::pnputil::expand_cab(&cab, &out)?;
    if !crate::pnputil::has_inf_for_this_arch(&out) {
        return Err("ce paquet ne contient pas de pilote pour l'architecture de ce PC".into());
    }
    Ok(out)
}

/// Dossier des téléchargements du Catalogue, vidé après chaque installation et au démarrage.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("mesdrivers-telechargements")
}

/// Rappel de progression qui envoie le pourcentage à l'UI (au plus une fois par point de %).
fn gauge(tx: &Sender<Msg>) -> impl Fn(u64, Option<u64>) + '_ {
    let last = std::cell::Cell::new(u64::MAX);
    move |got, total| {
        if let Some(t) = total.filter(|t| *t > 0) {
            let pct = got * 100 / t;
            if pct != last.get() {
                last.set(pct);
                let _ = tx.send(Msg::Gauge(Some(got as f64 / t as f64)));
            }
        }
    }
}

fn backup_report(dir: &std::path::Path) -> String {
    let infs = crate::pnputil::files(dir).iter().filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("inf"))).count();
    format!("{infs} pilote(s) exporté(s) ({}) dans {}", crate::model::human_size(crate::pnputil::dir_size(dir)), dir.display())
}

fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    crate::model::stamp(secs)[..10].to_string()
}

/// Dossier proposé pour une sauvegarde : Bureau\Pilotes-<PC>-<date>.
pub fn default_backup_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    let desk = home.join("Desktop");
    let base = if desk.is_dir() { desk } else { home };
    let pc = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
    base.join(format!("Pilotes-{pc}-{}", today()))
}

#[cfg(not(windows))]
fn run(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    for cmd in rx {
        let msg = match cmd {
            Cmd::SelfUpdate(tag) => crate::selfupdate::apply(&crate::catalog::agent(), &tag).unwrap_or_else(|e| e),
            _ => "Cette application ne fonctionne que sous Windows.".into(),
        };
        let _ = tx.send(Msg::Status(msg));
        let _ = tx.send(Msg::Done);
    }
}

#[cfg(windows)]
pub fn com_init() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()
}

#[cfg(windows)]
pub fn restore_point() -> Result<(), String> {
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

/// Inventaire + résumé + DriverStore (thread WMI), Catalogue en tâche de fond.
#[cfg(windows)]
fn inventory_job(tx: Sender<Msg>, with_summary: bool) {
    let con = match wmi::WMIConnection::new() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Msg::Status(format!("Inventaire WMI : {e}")));
            return;
        }
    };
    match crate::inventory::scan(&con) {
        Ok((mut d, mut b)) => {
            check_catalog(&mut d, &mut b, &tx);
            let in_use: Vec<String> = d.iter().map(|x| x.inf.clone()).collect();
            let _ = tx.send(Msg::Inventory(d, b));
            if with_summary {
                let _ = tx.send(Msg::Summary(crate::system::summary(&con)));
            }
            let _ = tx.send(Msg::Store(crate::store::scan(&in_use)));
        }
        Err(e) => {
            let _ = tx.send(Msg::Status(format!("Inventaire WMI : {e}")));
        }
    }
}

#[cfg(windows)]
fn run(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    use crate::wu::Wu;
    // Les objets COM (IUpdate) ne sont pas Send : Windows Update reste dans ce thread.
    let mut wu = match com_init().and_then(|_| Wu::new()) {
        Ok(w) => Some(w),
        Err(e) => {
            let _ = tx.send(Msg::Status(format!("Windows Update indisponible : {e}")));
            None
        }
    };
    let agent = crate::catalog::agent();
    let say = |s: String| {
        let _ = tx.send(Msg::Progress(s));
    };
    // WMI dans son propre thread pendant que Windows Update cherche : durée = max des deux.
    let scan = |wu: &mut Option<Wu>, summary: bool| {
        let inv_tx = tx.clone();
        let inv = std::thread::spawn(move || inventory_job(inv_tx, summary));
        if let Some(w) = wu.as_mut() {
            let _ = tx.send(match w.search() {
                Ok(u) => Msg::Updates(u),
                Err(e) => Msg::Status(format!("Recherche Windows Update : {e}")),
            });
        }
        let _ = inv.join();
    };
    let journal = |line: String| {
        if let Err(e) = crate::journal::log(&line) {
            let _ = tx.send(Msg::Status(format!("Journal non écrit : {e}")));
        }
    };
    let mut first = true;

    for cmd in rx {
        match cmd {
            Cmd::Scan => {
                scan(&mut wu, first);
                first = false;
            }
            Cmd::Install { wu: picks, catalog, skip_restore } => {
                if !skip_restore {
                    say("Création du point de restauration…".into());
                    if let Err(e) = restore_point() {
                        let retry = Cmd::Install { wu: picks.clone(), catalog: catalog.clone(), skip_restore: true };
                        let _ = tx.send(Msg::NeedForce(e, retry));
                        let _ = tx.send(Msg::Done);
                        continue;
                    }
                }
                let n = picks.len() + catalog.len();
                let (mut ok, mut reboot, mut k) = (0, false, 0);
                for &i in &picks {
                    k += 1;
                    let Some(w) = wu.as_ref() else { break };
                    let title = w.title(i);
                    say(format!("{k}/{n} : {title} (Windows Update)…"));
                    journal(match w.install_one(i) {
                        Ok((true, r)) => {
                            ok += 1;
                            reboot |= r;
                            format!("OK      WU         {title}")
                        }
                        Ok((false, _)) => format!("ÉCHEC   WU         {title}"),
                        Err(e) => format!("ERREUR  WU         {title} : {e}"),
                    });
                }
                let tmp = temp_dir();
                // Le même .cab peut être publié sous plusieurs ids (un par matériel) : on ne l'installe qu'une fois.
                let mut done_digests = std::collections::HashSet::new();
                for (id, title) in &catalog {
                    k += 1;
                    let info = match crate::catalog::download_info(&agent, id) {
                        Ok(i) => i,
                        Err(e) => {
                            journal(format!("ERREUR  Catalogue  {title} : {e}"));
                            continue;
                        }
                    };
                    if !done_digests.insert(info.1.clone()) {
                        ok += 1; // même paquet, déjà installé dans ce lot
                        continue;
                    }
                    say(format!("{k}/{n} : {title} (Catalogue : téléchargement)…"));
                    let r = fetch_catalog_package(&agent, id, &info, &tmp, &gauge(&tx)).and_then(|dir| {
                        let _ = tx.send(Msg::Gauge(None));
                        say(format!("{k}/{n} : {title} (installation)…"));
                        crate::pnputil::add_dir(&dir)
                    });
                    let _ = std::fs::remove_dir_all(tmp.join(id));
                    journal(match r {
                        Ok(r) => {
                            ok += 1;
                            reboot |= r;
                            format!("OK      Catalogue  {title}")
                        }
                        Err(e) => format!("ERREUR  Catalogue  {title} : {e}"),
                    });
                }
                let mut s = format!("{ok}/{n} pilote(s) installé(s). Détails : {}", crate::journal::path().display());
                if reboot {
                    s = format!("Redémarrage nécessaire. {s}");
                }
                let _ = tx.send(Msg::Status(s));
                scan(&mut wu, false); // les versions ont changé
            }
            Cmd::Backup(dir) => {
                say(format!("Export des pilotes vers {}…", dir.display()));
                let s = match crate::pnputil::export_all(&dir) {
                    Ok(()) => backup_report(&dir),
                    Err(e) => format!("Export échoué : {e}"),
                };
                journal(format!("EXPORT  {s}"));
                let _ = tx.send(Msg::Status(s));
            }
            Cmd::Restore { dir, skip_restore } => {
                if !skip_restore {
                    say("Création du point de restauration…".into());
                    if let Err(e) = restore_point() {
                        let _ = tx.send(Msg::NeedForce(e, Cmd::Restore { dir, skip_restore: true }));
                        let _ = tx.send(Msg::Done);
                        continue;
                    }
                }
                say(format!("Installation des pilotes de {}…", dir.display()));
                let s = match crate::pnputil::add_dir(&dir) {
                    Ok(true) => "Pilotes restaurés. Redémarrage nécessaire.".to_string(),
                    Ok(false) => "Pilotes restaurés.".to_string(),
                    Err(e) => format!("Restauration : {e}"),
                };
                journal(format!("RESTAURATION {} : {s}", dir.display()));
                let _ = tx.send(Msg::Status(s));
                scan(&mut wu, false);
            }
            Cmd::DeletePackages(oems) => {
                let (mut ok, mut errs) = (0, Vec::new());
                for (k, oem) in oems.iter().enumerate() {
                    say(format!("{}/{} : suppression de {oem}…", k + 1, oems.len()));
                    match crate::pnputil::delete(oem) {
                        Ok(()) => {
                            ok += 1;
                            journal(format!("SUPPR   {oem}"));
                        }
                        Err(e) => {
                            journal(format!("REFUS   {e}"));
                            errs.push(e);
                        }
                    }
                }
                let mut s = format!("{ok}/{} paquet(s) supprimé(s) du DriverStore.", oems.len());
                if let Some(e) = errs.first() {
                    s += &format!(" Refusé : {e}");
                }
                let _ = tx.send(Msg::Status(s));
                let inv_tx = tx.clone();
                let _ = std::thread::spawn(move || inventory_job(inv_tx, false)).join();
            }
            Cmd::Rollback { hwid, inf, label, skip_restore } => {
                if !skip_restore {
                    say("Création du point de restauration…".into());
                    if let Err(e) = restore_point() {
                        let _ = tx.send(Msg::NeedForce(e, Cmd::Rollback { hwid, inf, label, skip_restore: true }));
                        let _ = tx.send(Msg::Done);
                        continue;
                    }
                }
                say(format!("Retour à {label}…"));
                let s = match crate::store::force_install(&hwid, &inf) {
                    Ok(true) => format!("Revenu à {label}. Redémarrage nécessaire."),
                    Ok(false) => format!("Revenu à {label}."),
                    Err(e) => format!("Retour arrière impossible : {e}"),
                };
                journal(format!("RETOUR  {hwid} -> {inf} : {s}"));
                let _ = tx.send(Msg::Status(s));
                scan(&mut wu, false);
            }
            Cmd::SelfUpdate(tag) => {
                say(format!("Téléchargement de {tag}…"));
                let _ = tx.send(Msg::Status(crate::selfupdate::apply(&agent, &tag).unwrap_or_else(|e| e)));
            }
        }
        let _ = tx.send(Msg::Done);
    }
}
