// Hors Windows seul le squelette UI tourne : les champs remplis par WMI/WU y sont inutilisés.
#![cfg_attr(not(windows), allow(dead_code))]
mod catalog;
mod cli;
mod journal;
mod model;
mod pnputil;
mod selfupdate;
mod store;
mod ui;
mod worker;
#[cfg(windows)]
mod inventory;
#[cfg(windows)]
mod system;
#[cfg(windows)]
mod wu;

use model::{BrokenDevice, Catalog, Device, Package, Source, Update, cmp_version, visible};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::widgets::TableState;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;
use worker::{Cmd, Msg};

pub const RESUME: usize = 0;
pub const MATERIEL: usize = 1;
pub const MAJ: usize = 2;
pub const STORE: usize = 3;
pub const SAUVEGARDE: usize = 4;
pub const TABS: usize = 5;

/// Saisie de texte en cours.
#[derive(PartialEq)]
enum Input {
    Filter,
    /// Chemin pour l'action n° (0 = exporter, 1 = restaurer) de l'onglet Sauvegarde.
    Path(usize),
}

#[derive(Default)]
struct App {
    tab: usize,
    summary: Vec<(String, String)>,
    devices: Vec<Device>,
    broken: Vec<BrokenDevice>,
    wu_updates: Vec<Update>,
    checked: HashSet<String>,
    store: Vec<Package>,
    store_checked: HashSet<String>,
    states: [TableState; TABS],
    status: String,
    busy: bool,
    /// Une opération qui modifie le système est en cours : on ne quitte pas.
    locked: bool,
    confirm: Option<(String, Cmd)>,
    input: Option<Input>,
    filter: String,
    path: String,
    show_ms: bool,
    new_version: Option<String>,
    tick: usize,
}

impl App {
    /// Windows Update + pilotes plus récents trouvés au Catalogue (un par matériel).
    fn updates(&self) -> Vec<Update> {
        let mut out = self.wu_updates.clone();
        let mut seen = HashSet::new();
        for d in &self.devices {
            if let Catalog::Found { id, version, date, title } = &d.catalog
                && cmp_version(version, &d.version).is_gt()
                && seen.insert(id.clone())
            {
                out.push(Update {
                    source: Source::Catalog(id.clone()),
                    title: title.clone(),
                    provider: d.provider.clone(),
                    hwid: d.hwid.clone(),
                    date: date.clone(),
                    version: version.clone(),
                });
            }
        }
        out
    }
    /// Index d'origine des lignes visibles. Matériel : 0..broken.len() = en erreur, puis les pilotes.
    fn rows(&self, tab: usize) -> Vec<usize> {
        match tab {
            MATERIEL => {
                let broken = self.broken.iter().map(|b| (b.name.as_str(), false));
                let devs = self.devices.iter().map(|d| (d.name.as_str(), d.provider.eq_ignore_ascii_case("Microsoft")));
                visible(broken.chain(devs), &self.filter, !self.show_ms)
            }
            MAJ => {
                let u = self.updates();
                visible(u.iter().map(|u| (u.title.as_str(), false)), &self.filter, false)
            }
            STORE => {
                let text: Vec<String> = self.store.iter().map(|p| format!("{} {} {} {}", p.oem, p.original, p.provider, p.class)).collect();
                visible(text.iter().map(|t| (t.as_str(), false)), &self.filter, false)
            }
            SAUVEGARDE => vec![0, 1],
            _ => Vec::new(),
        }
    }
    fn selected(&self) -> Option<usize> {
        self.states[self.tab].selected().and_then(|i| self.rows(self.tab).get(i).copied())
    }
    fn selected_hwid(&self) -> Option<String> {
        let i = self.selected()?;
        match self.tab {
            MAJ => self.updates().get(i).map(|u| u.hwid.clone()),
            MATERIEL => self.devices.get(i.checked_sub(self.broken.len())?).map(|d| d.hwid.clone()),
            _ => None,
        }
    }
    fn install_cmd(&self) -> Cmd {
        let (mut wu, mut catalog) = (Vec::new(), Vec::new());
        for u in self.updates().into_iter().filter(|u| self.checked.contains(&u.key())) {
            match u.source {
                Source::Wu(i) => wu.push(i),
                Source::Catalog(id) => catalog.push((id, u.title)),
            }
        }
        Cmd::Install { wu, catalog, skip_restore: false }
    }
    /// Garde une sélection valide quand la liste change (filtre, nouvelle analyse…).
    fn clamp(&mut self) {
        for tab in 0..TABS {
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
    let arg = |i: usize| args.get(i).map(String::as_str);
    let need = |i: usize| -> String {
        arg(i).map(str::to_string).unwrap_or_else(|| {
            eprintln!("{}", cli::usage());
            std::process::exit(2)
        })
    };
    #[cfg(windows)]
    let code = match arg(0) {
        Some("--scan") => Some(cli::scan()),
        Some("--catalog-get") => Some(cli::catalog_get(&need(1), &PathBuf::from(need(2)))),
        Some("--backup") => Some(cli::backup(&PathBuf::from(need(1)))),
        Some("--restore") => Some(cli::restore(&PathBuf::from(need(1)))),
        _ => None,
    };
    #[cfg(not(windows))]
    let code = matches!(arg(0), Some("--scan" | "--catalog-get" | "--backup" | "--restore")).then(cli::windows_only);
    if let Some(code) = code {
        std::process::exit(code);
    }
    match arg(0) {
        Some("--version" | "-V") => {
            println!("mesdrivers {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--catalog") => std::process::exit(cli::catalog(&need(1))),
        Some(a) => {
            eprintln!("option inconnue : {a}\n{}", cli::usage());
            std::process::exit(2);
        }
        None => {}
    }

    selfupdate::cleanup();
    let (tx, rx) = worker::spawn();
    let mut app = App { busy: true, status: "Analyse du PC et recherche des mises à jour…".into(), ..Default::default() };
    let _ = tx.send(Cmd::Scan);
    ratatui::run(|t| run(t, &mut app, &tx, &rx))
}

fn handle(app: &mut App, msg: Msg) {
    match msg {
        Msg::Summary(s) => app.summary = s,
        Msg::Inventory(d, b) => (app.devices, app.broken) = (d, b),
        Msg::Updates(u) => {
            app.checked.retain(|k| k.starts_with("cat:"));
            app.wu_updates = u;
        }
        Msg::Catalog(h, c) => app.devices.iter_mut().filter(|d| d.hwid == h).for_each(|d| d.catalog = c.clone()),
        Msg::Store(p) => {
            app.store_checked = p.iter().filter(|p| p.obsolete).map(|p| p.oem.clone()).collect();
            app.store = p;
        }
        Msg::NewVersion(tag) => {
            app.status = format!("Nouvelle version {tag} disponible : U pour mettre à jour MesDrivers.");
            app.new_version = Some(tag);
        }
        Msg::Progress(s) | Msg::Status(s) => app.status = s,
        Msg::NeedForce(e, cmd) => {
            app.locked = false;
            app.confirm = Some((format!("Point de restauration impossible ({e}). Continuer sans ?"), cmd));
        }
        Msg::Done => {
            (app.busy, app.locked) = (false, false);
            if app.status.ends_with('…') {
                app.status = format!("{} mise(s) à jour disponible(s).", app.updates().len());
            }
        }
    }
    app.clamp();
}

fn send(app: &mut App, tx: &Sender<Cmd>, cmd: Cmd) {
    app.locked = !matches!(cmd, Cmd::Scan | Cmd::Backup(_));
    app.busy = true;
    app.status = "En cours…".into();
    let _ = tx.send(cmd);
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
                        (app.busy, app.locked) = (false, false);
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

        if let Some(input) = &app.input {
            let buf = if *input == Input::Filter { &mut app.filter } else { &mut app.path };
            match k.code {
                KeyCode::Char(c) => buf.push(c),
                KeyCode::Backspace => _ = buf.pop(),
                KeyCode::Esc => {
                    if *input == Input::Filter {
                        app.filter.clear();
                    }
                    app.input = None;
                }
                KeyCode::Enter => {
                    if let Input::Path(action) = *input {
                        let dir = PathBuf::from(app.path.trim());
                        app.confirm = Some(if action == 0 {
                            (format!("Exporter tous les pilotes tiers vers {} ?", dir.display()), Cmd::Backup(dir))
                        } else {
                            (format!("Installer tous les pilotes de {} ?", dir.display()), Cmd::Restore { dir, skip_restore: false })
                        });
                    }
                    app.input = None;
                }
                _ => {}
            }
            app.clamp();
            continue;
        }
        if let Some((_, cmd)) = app.confirm.take() {
            if matches!(k.code, KeyCode::Char('o' | 'O' | 'y' | 'Y')) {
                send(app, tx, cmd);
            } else {
                app.status = "Annulé.".into();
            }
            continue;
        }

        let n = app.rows(app.tab).len();
        let st = &mut app.states[app.tab];
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc if app.locked => app.status = "Opération en cours, patientez avant de quitter.".into(),
            KeyCode::Esc if !app.filter.is_empty() => {
                app.filter.clear();
                app.clamp();
            }
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Tab | KeyCode::Right => app.tab = (app.tab + 1) % TABS,
            KeyCode::BackTab | KeyCode::Left => app.tab = (app.tab + TABS - 1) % TABS,
            KeyCode::Char(c @ '1'..='5') => app.tab = c as usize - '1' as usize,
            KeyCode::Down | KeyCode::Char('j') if n > 0 => st.select(Some(st.selected().map_or(0, |i| (i + 1).min(n - 1)))),
            KeyCode::Up | KeyCode::Char('k') if n > 0 => st.select(Some(st.selected().map_or(0, |i| i.saturating_sub(1)))),
            KeyCode::PageDown if n > 0 => st.select(Some(st.selected().map_or(0, |i| (i + 15).min(n - 1)))),
            KeyCode::PageUp if n > 0 => st.select(Some(st.selected().map_or(0, |i| i.saturating_sub(15)))),
            KeyCode::Char('/') if matches!(app.tab, MATERIEL | MAJ | STORE) => app.input = Some(Input::Filter),
            KeyCode::Char('m') if app.tab == MATERIEL => {
                app.show_ms = !app.show_ms;
                app.clamp();
            }
            KeyCode::Char('o') if matches!(app.tab, MATERIEL | MAJ) => {
                app.status = app.selected_hwid().map_or_else(|| "Aucune ligne sélectionnée.".into(), |h| open_catalog(&h));
            }
            KeyCode::Char('U') if !app.busy => match app.new_version.clone() {
                Some(tag) => app.confirm = Some((format!("Mettre à jour MesDrivers vers {tag} ?"), Cmd::SelfUpdate(tag))),
                None => app.status = "MesDrivers est à jour.".into(),
            },
            KeyCode::Char('r') if !app.busy => send(app, tx, Cmd::Scan),

            KeyCode::Char(' ') if app.tab == MAJ => {
                if let Some(u) = app.selected().and_then(|i| app.updates().into_iter().nth(i)) {
                    let key = u.key();
                    if !app.checked.remove(&key) {
                        app.checked.insert(key);
                    }
                }
            }
            KeyCode::Char('a') if app.tab == MAJ => {
                let keys: Vec<String> = app.updates().iter().map(Update::key).collect();
                if keys.iter().all(|k| app.checked.contains(k)) {
                    app.checked.clear();
                } else {
                    app.checked.extend(keys);
                }
            }
            KeyCode::Char('i') if app.tab == MAJ && !app.busy => {
                let cmd = app.install_cmd();
                let Cmd::Install { wu, catalog, .. } = &cmd else { continue };
                if wu.is_empty() && catalog.is_empty() {
                    app.status = "Cochez au moins un pilote avec Espace.".into();
                } else {
                    let q = format!("Installer {} pilote(s) ? Un point de restauration sera créé avant.", wu.len() + catalog.len());
                    app.confirm = Some((q, cmd));
                }
            }

            KeyCode::Char(' ') if app.tab == STORE => {
                if let Some(p) = app.selected().and_then(|i| app.store.get(i)) {
                    if p.in_use {
                        app.status = format!("{} est en service : Windows refuserait de le supprimer.", p.oem);
                    } else if !app.store_checked.remove(&p.oem) {
                        app.store_checked.insert(p.oem.clone());
                    }
                }
            }
            KeyCode::Char('a') if app.tab == STORE => {
                let obsolete: HashSet<String> = app.store.iter().filter(|p| p.obsolete).map(|p| p.oem.clone()).collect();
                app.store_checked = if app.store_checked == obsolete { HashSet::new() } else { obsolete };
            }
            KeyCode::Char('d') if app.tab == STORE && !app.busy => {
                let oems: Vec<String> = app.store.iter().filter(|p| app.store_checked.contains(&p.oem)).map(|p| p.oem.clone()).collect();
                if oems.is_empty() {
                    app.status = "Cochez au moins un paquet avec Espace.".into();
                } else {
                    let size: u64 = app.store.iter().filter(|p| app.store_checked.contains(&p.oem)).map(|p| p.size).sum();
                    let q = format!("Supprimer {} paquet(s) du DriverStore ({}) ?", oems.len(), model::human_size(size));
                    app.confirm = Some((q, Cmd::DeletePackages(oems)));
                }
            }

            KeyCode::Enter if app.tab == SAUVEGARDE && !app.busy => {
                let action = app.selected().unwrap_or(0);
                app.path = worker::default_backup_dir().display().to_string();
                app.input = Some(Input::Path(action));
            }
            _ => {}
        }
    }
}
