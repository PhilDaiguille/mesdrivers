// Hors Windows seul le squelette UI tourne : les champs remplis par WMI/WU y sont inutilisés.
#![cfg_attr(not(windows), allow(dead_code))]
mod cache;
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

use model::{BrokenDevice, Device, Package, Source, Update, visible};
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
/// Tris proposés par onglet (touche `s`).
pub const SORTS: [&[&str]; TABS] = [
    &[],
    &["nom", "fabricant", "date récente", "Catalogue d'abord"],
    &["source", "titre", "date récente"],
    &["obsolètes d'abord", "taille", "nom", "date récente"],
    &[],
];

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
    /// Avancement du téléchargement en cours (0..1).
    gauge: Option<f64>,
    sort: [usize; TABS],
    help: bool,
    /// Fiche du périphérique sélectionné ouverte ; index dans la liste des versions du DriverStore.
    sheet: Option<usize>,
    /// Dernière installation : proposer ensuite de supprimer les anciennes versions.
    after_install: bool,
    /// Zones cliquables mémorisées au rendu (souris) : onglets et corps du tableau.
    tab_spans: Vec<(u16, u16)>,
    body: ratatui::layout::Rect,
}

impl App {
    /// Windows Update + pilotes plus récents trouvés au Catalogue (regroupés par paquet).
    fn updates(&self) -> Vec<Update> {
        let mut out = self.wu_updates.clone();
        out.extend(model::catalog_updates(&self.devices, &self.broken));
        out
    }
    /// Index d'origine des lignes visibles. Matériel : 0..broken.len() = en erreur, puis les pilotes.
    fn rows(&self, tab: usize) -> Vec<usize> {
        let mode = self.sort[tab];
        match tab {
            MATERIEL => {
                let nb = self.broken.len();
                let broken = self.broken.iter().map(|b| (b.name.as_str(), false));
                let devs = self.devices.iter().map(|d| (d.name.as_str(), d.provider.eq_ignore_ascii_case("Microsoft")));
                let mut rows = visible(broken.chain(devs), &self.filter, !self.show_ms);
                // Les périphériques en erreur restent toujours en haut.
                rows.sort_by_cached_key(|&i| {
                    let Some(d) = i.checked_sub(nb).and_then(|j| self.devices.get(j)) else { return (false, String::new()) };
                    let key = match mode {
                        1 => d.provider.to_lowercase(),
                        2 => sort_desc(&d.date),
                        3 => (!model::has_newer(d)).to_string(),
                        _ => d.name.to_lowercase(),
                    };
                    (true, key)
                });
                rows
            }
            MAJ => {
                let u = self.updates();
                let mut rows = visible(u.iter().map(|u| (u.title.as_str(), false)), &self.filter, false);
                match mode {
                    1 => rows.sort_by_cached_key(|&i| u[i].title.to_lowercase()),
                    2 => rows.sort_by_cached_key(|&i| sort_desc(&u[i].date)),
                    _ => {}
                }
                rows
            }
            STORE => {
                let text: Vec<String> = self.store.iter().map(|p| format!("{} {} {} {}", p.oem, p.original, p.provider, p.class)).collect();
                let mut rows = visible(text.iter().map(|t| (t.as_str(), false)), &self.filter, false);
                let st = &self.store;
                match mode {
                    1 => rows.sort_by_key(|&i| std::cmp::Reverse(st[i].size)),
                    2 => rows.sort_by_cached_key(|&i| st[i].original.clone()),
                    3 => rows.sort_by_cached_key(|&i| sort_desc(&st[i].date)),
                    _ => {}
                }
                rows
            }
            SAUVEGARDE => vec![0, 1],
            _ => Vec::new(),
        }
    }
    /// Versions présentes dans le DriverStore pour le pilote du périphérique sélectionné (plus récente d'abord).
    fn sheet_versions(&self) -> Vec<&Package> {
        let Some(d) = self.selected().and_then(|i| self.devices.get(i.checked_sub(self.broken.len())?)) else { return Vec::new() };
        let Some(orig) = self.store.iter().find(|p| p.oem.eq_ignore_ascii_case(&d.inf)).map(|p| p.original.clone()) else {
            return Vec::new();
        };
        let mut v: Vec<&Package> = self.store.iter().filter(|p| p.original == orig).collect();
        v.sort_by(|a, b| model::cmp_version(&b.version, &a.version));
        v
    }
    fn to_cache(&self) -> cache::Cache {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        cache::Cache {
            saved: model::stamp(secs),
            summary: self.summary.clone(),
            devices: self.devices.clone(),
            broken: self.broken.clone(),
            store: self.store.clone(),
        }
    }
    fn selected(&self) -> Option<usize> {
        self.states[self.tab].selected().and_then(|i| self.rows(self.tab).get(i).copied())
    }
    fn selected_hwid(&self) -> Option<String> {
        let i = self.selected()?;
        match self.tab {
            MAJ => self.updates().get(i).map(|u| u.hwid.clone()),
            MATERIEL => match i.checked_sub(self.broken.len()) {
                Some(j) => self.devices.get(j).map(|d| d.hwid.clone()),
                None => self.broken.get(i).map(|b| b.hwid.clone()),
            },
            _ => None,
        }
    }
    fn install_cmd(&self) -> Cmd {
        let (mut wu, mut catalog) = (Vec::new(), Vec::new());
        for u in self.updates().into_iter().filter(|u| self.checked.contains(&u.key())) {
            match u.source {
                Source::Wu(i) => wu.push(i),
                Source::Catalog(ids) => catalog.extend(ids.into_iter().map(|id| (id, u.title.clone()))),
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

/// Clé de tri « plus récent d'abord » pour une date AAAA-MM-JJ.
fn sort_desc(date: &str) -> String {
    date.bytes().map(|b| if b.is_ascii_digit() { (b'9' - b + b'0') as char } else { b as char }).collect()
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
    let _ = std::fs::remove_dir_all(worker::temp_dir()); // restes d'un téléchargement interrompu
    let (tx, rx) = worker::spawn();
    let mut app = App { busy: true, status: "Analyse du PC et recherche des mises à jour…".into(), ..Default::default() };
    if let Some(c) = cache::load() {
        app.status = format!("Données du {} affichées, actualisation en cours…", c.saved);
        (app.summary, app.devices, app.broken, app.store) = (c.summary, c.devices, c.broken, c.store);
        app.store_checked = app.store.iter().filter(|p| p.obsolete).map(|p| p.oem.clone()).collect();
        app.clamp();
    }
    let _ = tx.send(Cmd::Scan);
    ratatui::run(|t| {
        use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
        let _ = ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture);
        let r = run(t, &mut app, &tx, &rx);
        let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        r
    })
}

fn handle(app: &mut App, msg: Msg) {
    match msg {
        Msg::Summary(s) => app.summary = s,
        Msg::Inventory(d, b) => (app.devices, app.broken) = (d, b),
        Msg::Updates(u) => {
            app.checked.retain(|k| k.starts_with("cat:"));
            app.wu_updates = u;
        }
        Msg::Catalog(h, c) => {
            app.devices.iter_mut().filter(|d| d.hwid == h).for_each(|d| d.catalog = c.clone());
            app.broken.iter_mut().filter(|b| b.hwid == h).for_each(|b| b.catalog = c.clone());
        }
        Msg::Store(p) => {
            app.store_checked = p.iter().filter(|p| p.obsolete).map(|p| p.oem.clone()).collect();
            app.store = p;
        }
        Msg::NewVersion(tag) => {
            app.status = format!("Nouvelle version {tag} disponible : U pour mettre à jour MesDrivers.");
            app.new_version = Some(tag);
        }
        Msg::Progress(s) | Msg::Status(s) => app.status = s,
        Msg::Gauge(g) => app.gauge = g,
        Msg::NeedForce(e, cmd) => {
            app.locked = false;
            app.confirm = Some((format!("Point de restauration impossible ({e}). Continuer sans ?"), cmd));
        }
        Msg::Done => {
            (app.busy, app.locked, app.gauge) = (false, false, None);
            if app.status.ends_with('…') {
                app.status = format!("{} mise(s) à jour disponible(s).", app.updates().len());
            }
            if !app.devices.is_empty() {
                cache::save(&app.to_cache());
            }
            // Après une mise à jour, l'ancienne version reste dans le DriverStore : proposer de la supprimer.
            if std::mem::take(&mut app.after_install) && app.confirm.is_none() {
                let old: Vec<&Package> = app.store.iter().filter(|p| p.obsolete).collect();
                if !old.is_empty() {
                    let size = old.iter().map(|p| p.size).sum();
                    let oems = old.iter().map(|p| p.oem.clone()).collect();
                    let q = format!("Supprimer les {} ancienne(s) version(s) devenues inutiles ({}) ?", old.len(), model::human_size(size));
                    app.confirm = Some((q, Cmd::DeletePackages(oems)));
                }
            }
        }
    }
    app.clamp();
}

fn send(app: &mut App, tx: &Sender<Cmd>, cmd: Cmd) {
    app.locked = !matches!(cmd, Cmd::Scan | Cmd::Backup(_));
    app.after_install = matches!(cmd, Cmd::Install { .. } | Cmd::Restore { .. });
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

/// Souris : molette = défilement, clic sur un onglet ou une ligne, clic sur [ ] = cocher.
fn mouse(app: &mut App, m: ratatui::crossterm::event::MouseEvent) {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    if app.confirm.is_some() || app.help || app.sheet.is_some() || app.input.is_some() {
        return;
    }
    let n = app.rows(app.tab).len();
    let st = &mut app.states[app.tab];
    match m.kind {
        MouseEventKind::ScrollDown if n > 0 => st.select(Some(st.selected().map_or(0, |i| (i + 3).min(n - 1)))),
        MouseEventKind::ScrollUp if n > 0 => st.select(Some(st.selected().map_or(0, |i| i.saturating_sub(3)))),
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(t) = app.tab_spans.iter().position(|&(x0, x1)| m.row == 0 && (x0..x1).contains(&m.column)) {
                app.tab = t;
                return;
            }
            let b = app.body;
            // bordure + ligne d'en-tête (sauf l'onglet Sauvegarde qui n'a pas d'en-tête)
            let top = b.y + if app.tab == SAUVEGARDE { 1 } else { 2 };
            if m.row < top || m.row >= b.y + b.height.saturating_sub(1) || m.column <= b.x || m.column >= b.x + b.width {
                return;
            }
            let i = st.offset() + usize::from(m.row - top);
            if i >= n {
                return;
            }
            let again = st.selected() == Some(i);
            st.select(Some(i));
            // Clic sur la case à cocher, ou second clic sur la ligne : cocher / décocher.
            if m.column < b.x + 5 || again {
                let key = match app.tab {
                    MAJ => app.selected().and_then(|i| app.updates().get(i).map(Update::key)),
                    STORE => app.selected().and_then(|i| app.store.get(i)).filter(|p| !p.in_use).map(|p| p.oem.clone()),
                    _ => None,
                };
                let set = if app.tab == MAJ { &mut app.checked } else { &mut app.store_checked };
                if let Some(k) = key
                    && !set.remove(&k)
                {
                    set.insert(k);
                }
            }
        }
        _ => {}
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
        let k = match event::read()? {
            // Windows envoie aussi les relâchements de touche : on ne garde que l'appui.
            Event::Key(k) if k.kind == KeyEventKind::Press => k,
            Event::Mouse(m) => {
                mouse(app, m);
                continue;
            }
            _ => continue,
        };

        if app.help {
            app.help = false;
            continue;
        }
        if let Some(sel) = app.sheet {
            let versions: Vec<Package> = app.sheet_versions().into_iter().cloned().collect();
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => app.sheet = Some((sel + 1).min(versions.len().saturating_sub(1))),
                KeyCode::Up | KeyCode::Char('k') => app.sheet = Some(sel.saturating_sub(1)),
                KeyCode::Enter if !app.busy => {
                    let hwid = app.selected().and_then(|i| app.devices.get(i.checked_sub(app.broken.len())?)).map(|d| d.hwid.clone());
                    match (versions.get(sel), hwid) {
                        (Some(p), _) if p.in_use => app.status = "Cette version est déjà celle en service.".into(),
                        (Some(p), Some(hwid)) if !p.path.is_empty() && !hwid.is_empty() => {
                            let label = format!("{} {}", p.original, p.version);
                            let q = format!("Revenir à {label} ? Un point de restauration sera créé avant.");
                            app.confirm = Some((q, Cmd::Rollback { hwid, inf: p.path.clone(), label, skip_restore: false }));
                            app.sheet = None;
                        }
                        _ => app.status = "Pas de version précédente utilisable pour ce périphérique.".into(),
                    }
                }
                KeyCode::Char('o') => app.status = app.selected_hwid().map_or_else(|| "Pas de HardwareID.".into(), |h| open_catalog(&h)),
                _ => app.sheet = None,
            }
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
            KeyCode::Char('?') | KeyCode::F(1) => app.help = true,
            KeyCode::Char('s') if !SORTS[app.tab].is_empty() => {
                app.sort[app.tab] = (app.sort[app.tab] + 1) % SORTS[app.tab].len();
                app.status = format!("Tri : {}", SORTS[app.tab][app.sort[app.tab]]);
            }
            KeyCode::Enter if app.tab == MATERIEL => app.sheet = Some(0),
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
