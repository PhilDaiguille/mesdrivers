//! Thread de travail : tout le COM (WMI + Windows Update) vit ici, l'UI reste réactive.
use crate::model::{BrokenDevice, Device, Update};
use std::sync::mpsc::{Receiver, Sender, channel};

pub enum Cmd {
    Scan,
    Install(Vec<usize>),
}

pub enum Msg {
    Inventory(Vec<Device>, Vec<BrokenDevice>),
    Updates(Vec<Update>),
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

#[cfg(not(windows))]
fn run(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    for _ in rx {
        let _ = tx.send(Msg::Error("Cette application ne fonctionne que sous Windows.".into()));
        let _ = tx.send(Msg::Done);
    }
}

#[cfg(windows)]
fn run(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    // Les objets COM (IUpdate) ne sont pas Send : ils restent dans ce thread.
    if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        let _ = tx.send(Msg::Error(format!("COM : {e}")));
        return;
    }
    let mut wu = match crate::wu::Wu::new() {
        Ok(w) => w,
        Err(e) => {
            let _ = tx.send(Msg::Error(format!("Windows Update indisponible : {e}")));
            return;
        }
    };
    let search = |wu: &mut crate::wu::Wu| {
        let _ = tx.send(match wu.search() {
            Ok(u) => Msg::Updates(u),
            Err(e) => Msg::Error(format!("Recherche Windows Update : {e}")),
        });
    };

    for cmd in rx {
        match cmd {
            Cmd::Scan => {
                let inv = wmi::WMIConnection::new().and_then(|c| crate::inventory::scan(&c));
                let _ = tx.send(match inv {
                    Ok((d, b)) => Msg::Inventory(d, b),
                    Err(e) => Msg::Error(format!("Inventaire WMI : {e}")),
                });
                search(&mut wu);
            }
            Cmd::Install(picks) => {
                let _ = tx.send(match wu.install(&picks) {
                    Ok(s) => Msg::Installed(s),
                    Err(e) => Msg::Error(format!("Installation : {e}")),
                });
                search(&mut wu); // la liste change après installation
            }
        }
        let _ = tx.send(Msg::Done);
    }
}
