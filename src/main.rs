// Hors Windows seul le squelette UI tourne : les champs remplis par WMI/WU y sont inutilisés.
#![cfg_attr(not(windows), allow(dead_code))]
mod catalog;
mod journal;
mod model;
mod ui;
mod worker;
#[cfg(windows)]
mod inventory;
#[cfg(windows)]
mod wu;

use model::{BrokenDevice, Device, Update, visible};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::widgets::TableState;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;
use worker::{Cmd, Msg};

#[derive(Default, PartialEq)]
enum Confirm {
    #[default]
    No,
    Install,
    /// Le point de restauration a échoué : installer quand même ?
    Force,
}

#[derive(Default)]
struct App {
    tab: usize,
    devices: Vec<Device>,
    broken: Vec<BrokenDevice>,
    updates: Vec<Update>,
    checked: Vec<bool>,
    states: [TableState; 2],
    status: String,
    busy: bool,
    installing: bool,
    confirm: Confirm,
    tick: usize,
    filter: String,
    typing: bool,
    show_ms: bool,
}

impl App {
    /// Onglet Matériel : index 0..broken.len() = en erreur, puis les pilotes.
    fn rows(&self, tab: usize) -> Vec<usize> {
        if tab == 0 {
            let broken = self.broken.iter().map(|b| (b.name.as_str(), false));
            let devs = self.devices.iter().map(|d| (d.name.as_str(), d.provider.eq_ignore_ascii_case("Microsoft")));
            visible(broken.chain(devs), &self.filter, !self.show_ms)
        } else {
            visible(self.updates.iter().map(|u| (u.title.as_str(), false)), &self.filter, false)
        }
    }
    /// Index d'origine de la ligne sélectionnée.
    fn selected(&self) -> Option<usize> {
        self.states[self.tab].selected().and_then(|i| self.rows(self.tab).get(i).copied())
    }
    fn picks(&self) -> Vec<usize> {
        self.checked.iter().enumerate().filter(|(_, c)| **c).map(|(i, _)| i).collect()
    }
    fn selected_hwid(&self) -> Option<&str> {
        let i = self.selected()?;
        if self.tab == 1 {
            return self.updates.get(i).map(|u| u.hwid.as_str());
        }
        self.devices.get(i.checked_sub(self.broken.len())?).map(|d| d.hwid.as_str())
    }
    /// Garde une sélection valide quand la liste change (filtre, nouvelle analyse…).
    fn clamp(&mut self) {
        for tab in 0..2 {
            let n = self.rows(tab).len();
            let st = &mut self.states[tab];
            st.select((n > 0).then(|| st.selected().unwrap_or(0).min(n - 1)));
        }
    }
    fn hidden_ms(&self) -> usize {
        if self.show_ms { 0 } else { self.devices.iter().filter(|d| d.provider.eq_ignore_ascii_case("Microsoft")).count() }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("mesdrivers {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--scan") => std::process::exit(worker::cli_scan()),
        Some("--catalog") => {
            let Some(hwid) = args.get(1) else {
                eprintln!("usage : mesdrivers --catalog <HardwareID>");
                std::process::exit(2);
            };
            let r = catalog::check(&catalog::agent(), hwid);
            println!("{r:?}");
            std::process::exit(if matches!(r, model::Catalog::Found { .. }) { 0 } else { 1 });
        }
        Some(a) => {
            eprintln!("option inconnue : {a}\nusage : mesdrivers [--scan | --catalog <HardwareID> | --version]");
            std::process::exit(2);
        }
        None => {}
    }

    let (tx, rx) = worker::spawn();
    let mut app = App { busy: true, status: "Analyse du matériel et recherche sur Windows Update…".into(), ..Default::default() };
    let _ = tx.send(Cmd::Scan);
    ratatui::run(|t| run(t, &mut app, &tx, &rx))
}

fn handle(app: &mut App, msg: Msg) {
    match msg {
        Msg::Inventory(d, b) => (app.devices, app.broken) = (d, b),
        Msg::Updates(u) => {
            app.checked = vec![false; u.len()];
            app.updates = u;
        }
        Msg::Catalog(h, c) => app.devices.iter_mut().filter(|d| d.hwid == h).for_each(|d| d.catalog = c.clone()),
        Msg::Progress(s) => app.status = s,
        Msg::NeedForce(e) => {
            app.installing = false;
            app.status = format!("Point de restauration impossible : {e}");
            app.confirm = Confirm::Force;
        }
        Msg::Installed(s) => {
            app.installing = false;
            app.status = s;
        }
        Msg::Error(s) => app.status = s,
        Msg::Done => {
            app.busy = false;
            if app.status.ends_with('…') {
                app.status = format!("{} pilote(s) à mettre à jour.", app.updates.len());
            }
        }
    }
    app.clamp();
}

fn start_install(app: &mut App, tx: &Sender<Cmd>, skip_restore: bool) {
    let picks = app.picks();
    app.status = format!("Installation de {} pilote(s)…", picks.len());
    app.busy = true;
    app.installing = true;
    let _ = tx.send(Cmd::Install { picks, skip_restore });
}

fn open_catalog(hwid: &str) -> String {
    let Some(q) = catalog::queries(hwid).into_iter().next() else { return "Pas de HardwareID pour ce périphérique.".into() };
    let url = catalog::search_url(&q);
    // explorer.exe ouvre l'URL dans le navigateur par défaut sans passer par cmd (pas de souci d'échappement).
    let opener = if cfg!(windows) { "explorer" } else { "open" };
    match std::process::Command::new(opener).arg(&url).spawn() {
        Ok(_) => format!("Ouvert : {url}"),
        Err(e) => format!("Impossible d'ouvrir le navigateur ({e}) : {url}"),
    }
}

fn run(terminal: &mut DefaultTerminal, app: &mut App, tx: &Sender<Cmd>, rx: &Receiver<Msg>) -> std::io::Result<()> {
    loop {
        loop {
            match rx.try_recv() {
                Ok(msg) => handle(app, msg),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if app.busy {
                        app.status = "Le moteur s'est arrêté (erreur interne). Relancez l'application.".into();
                        (app.busy, app.installing) = (false, false);
                    }
                    break;
                }
            }
        }
        app.tick += 1;
        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(Duration::from_millis(120))? {
            continue;
        }
        // Windows envoie aussi les relâchements de touche : on ne garde que l'appui.
        let Event::Key(k) = event::read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }

        if app.typing {
            match k.code {
                KeyCode::Char(c) => app.filter.push(c),
                KeyCode::Backspace => _ = app.filter.pop(),
                KeyCode::Esc => {
                    app.filter.clear();
                    app.typing = false;
                }
                KeyCode::Enter | KeyCode::Down | KeyCode::Up => app.typing = false,
                _ => {}
            }
            app.clamp();
            continue;
        }
        if app.confirm != Confirm::No {
            let yes = matches!(k.code, KeyCode::Char('o' | 'O' | 'y' | 'Y'));
            let force = app.confirm == Confirm::Force;
            app.confirm = Confirm::No;
            if yes {
                start_install(app, tx, force);
            } else {
                app.status = "Installation annulée.".into();
            }
            continue;
        }

        let n = app.rows(app.tab).len();
        let st = &mut app.states[app.tab];
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc if app.installing => app.status = "Installation en cours, patientez avant de quitter.".into(),
            KeyCode::Esc if !app.filter.is_empty() => {
                app.filter.clear();
                app.clamp();
            }
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => app.tab ^= 1,
            KeyCode::Down | KeyCode::Char('j') if n > 0 => st.select(Some(st.selected().map_or(0, |i| (i + 1).min(n - 1)))),
            KeyCode::Up | KeyCode::Char('k') if n > 0 => st.select(Some(st.selected().map_or(0, |i| i.saturating_sub(1)))),
            KeyCode::PageDown if n > 0 => st.select(Some(st.selected().map_or(0, |i| (i + 15).min(n - 1)))),
            KeyCode::PageUp if n > 0 => st.select(Some(st.selected().map_or(0, |i| i.saturating_sub(15)))),
            KeyCode::Char('/') => app.typing = true,
            KeyCode::Char('m') => {
                app.show_ms = !app.show_ms;
                app.clamp();
            }
            KeyCode::Char('o') => {
                app.status = app.selected_hwid().map_or_else(|| "Aucune ligne sélectionnée.".into(), open_catalog);
            }
            KeyCode::Char(' ') if app.tab == 1 => {
                if let Some(c) = app.selected().and_then(|i| app.checked.get_mut(i)) {
                    *c = !*c;
                }
            }
            KeyCode::Char('a') if app.tab == 1 => {
                let all = app.checked.iter().all(|c| *c);
                app.checked.iter_mut().for_each(|c| *c = !all);
            }
            KeyCode::Char('i') if app.tab == 1 && !app.busy => {
                if app.picks().is_empty() {
                    app.status = "Cochez au moins un pilote avec Espace.".into();
                } else {
                    app.confirm = Confirm::Install;
                }
            }
            KeyCode::Char('r') if !app.busy => {
                app.busy = true;
                app.status = "Nouvelle analyse…".into();
                let _ = tx.send(Cmd::Scan);
            }
            _ => {}
        }
    }
}
