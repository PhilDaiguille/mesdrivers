//! Rendu de l'interface (aucune logique métier ici).
use crate::model::{Catalog, Source, cmp_version, current_version, display_date, error_label, human_size};
use crate::{App, Input, MAJ, MATERIEL, RESUME, SAUVEGARDE, SORTS, STORE};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, LineGauge, Paragraph, Row, Table, Tabs, Wrap};

pub fn draw(f: &mut Frame, app: &mut App) {
    let [top, body, details, bottom, help] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let updates = app.updates();
    let reclaim: u64 = app.store.iter().filter(|p| p.obsolete).map(|p| p.size).sum();
    let titles = [
        " 1 Résumé ".to_string(),
        format!(" 2 Matériel ({}) ", app.devices.len() + app.broken.len()),
        format!(" 3 Mises à jour ({}) ", updates.len()),
        format!(" 4 DriverStore ({}) ", human_size(reclaim)),
        " 5 Sauvegarde ".to_string(),
    ];
    // Positions des onglets pour la souris (Tabs met 1 espace de marge + séparateur « │ » entre chaque).
    let mut x = top.x;
    app.tab_spans = titles
        .iter()
        .map(|t| {
            let w = t.chars().count() as u16 + 2;
            let span = (x, x + w);
            x += w + 1;
            span
        })
        .collect();
    app.body = body;
    f.render_widget(Tabs::new(titles).select(app.tab).highlight_style(Style::new().bold().reversed()).divider("│"), top);

    let mut title = match app.tab {
        RESUME => " Votre PC ".to_string(),
        MATERIEL => " Pilotes installés ".to_string(),
        MAJ => " Mises à jour (Windows Update + Catalogue Microsoft) ".to_string(),
        STORE => format!(" DriverStore : {} paquets tiers, {} récupérables ", app.store.len(), human_size(reclaim)),
        _ => " Sauvegarde et restauration des pilotes ".to_string(),
    };
    if !app.filter.is_empty() || app.input == Some(Input::Filter) {
        title += &format!("· filtre « {} » ", app.filter);
    }
    if app.tab == MATERIEL && app.hidden_ms() > 0 {
        title += &format!("· {} pilotes Microsoft masqués (m) ", app.hidden_ms());
    }
    if let Some(sort) = SORTS[app.tab].get(app.sort[app.tab]).filter(|_| app.sort[app.tab] > 0) {
        title += &format!("· tri : {sort} ");
    }
    let block = Block::bordered().title(title);
    let hl = Style::new().reversed();
    let header = |cols: &[&'static str]| Row::new(cols.to_vec()).style(Style::new().bold().fg(Color::Cyan));
    let rows = app.rows(app.tab);
    let check = |on: bool| if on { "[x]" } else { "[ ]" }.to_string();

    match app.tab {
        RESUME => {
            let n_err = app.broken.len();
            let mut lines: Vec<Line> = app
                .summary
                .iter()
                .map(|(k, v)| Line::from(vec![Span::from(format!("{k:<17}")).dark_gray(), Span::from(v.clone())]))
                .collect();
            if lines.is_empty() {
                lines.push(Line::from("Analyse en cours…").dark_gray());
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::from(format!("{} pilotes installés · ", app.devices.len())),
                Span::from(format!("{n_err} périphérique(s) en erreur")).fg(if n_err > 0 { Color::Red } else { Color::Green }),
                Span::from(" · "),
                Span::from(format!("{} mise(s) à jour disponible(s)", updates.len())).fg(if updates.is_empty() { Color::Green } else { Color::Yellow }),
                Span::from(format!(" · {} récupérables dans le DriverStore", human_size(reclaim))),
            ]));
            if let Some(tag) = &app.new_version {
                lines.push(Line::from(format!("Nouvelle version de MesDrivers : {tag} (touche U)")).yellow().bold());
            }
            f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), body);
        }
        MATERIEL => {
            let nb = app.broken.len();
            let table_rows = rows.iter().map(|&i| match i.checked_sub(nb) {
                None => {
                    let b = &app.broken[i];
                    let cat = match &b.catalog {
                        Catalog::Found { version, .. } => Cell::from(format!("{version} dispo")).yellow(),
                        other => catalog_cell(other, ""),
                    };
                    Row::new([
                        Cell::from(b.name.clone()).red(),
                        Cell::from(error_label(b.code)).red(),
                        Cell::from(format!("code {}", b.code)).red(),
                        Cell::from(""),
                        cat,
                    ])
                }
                Some(j) => {
                    let d = &app.devices[j];
                    Row::new([
                        Cell::from(d.name.clone()),
                        Cell::from(d.provider.clone()),
                        Cell::from(d.version.clone()),
                        Cell::from(display_date(&d.date)).fg(if d.date.starts_with("19") { Color::DarkGray } else { Color::Reset }),
                        catalog_cell(&d.catalog, &d.version),
                    ])
                }
            });
            let widths = [Constraint::Fill(3), Constraint::Fill(1), Constraint::Length(22), Constraint::Length(10), Constraint::Length(20)];
            let t = Table::new(table_rows, widths)
                .header(header(&["Périphérique", "Fabricant", "Version", "Date", "Catalogue MS"]))
                .block(block)
                .row_highlight_style(hl);
            f.render_stateful_widget(t, body, &mut app.states[MATERIEL]);
        }
        MAJ => {
            let table_rows = rows.iter().map(|&i| {
                let u = &updates[i];
                let missing = !u.devices.is_empty() && u.devices.iter().all(|d| d.ends_with("(sans pilote)"));
                let cur = if missing { "aucune" } else { current_version(&app.devices, &u.hwid).unwrap_or("?") };
                let src = match u.source {
                    Source::Wu(_) => Cell::from("WU"),
                    Source::Catalog(_) => Cell::from("Catalogue").yellow(),
                };
                let new = if u.version.is_empty() { "voir titre".to_string() } else { u.version.clone() };
                let title = match u.devices.len() {
                    0 | 1 => u.title.clone(),
                    n => format!("{} · {n} périphériques", u.title),
                };
                Row::new([
                    Cell::from(check(app.checked.contains(&u.key()))),
                    src,
                    Cell::from(title),
                    Cell::from(cur.to_string()),
                    Cell::from(new),
                    Cell::from(display_date(&u.date)),
                ])
            });
            let widths = [
                Constraint::Length(3),
                Constraint::Length(9),
                Constraint::Fill(1),
                Constraint::Length(18),
                Constraint::Length(18),
                Constraint::Length(10),
            ];
            let t = Table::new(table_rows, widths)
                .header(header(&["", "Source", "Pilote proposé", "Version actuelle", "Nouvelle version", "Date"]))
                .block(block)
                .row_highlight_style(hl);
            f.render_stateful_widget(t, body, &mut app.states[MAJ]);
        }
        STORE => {
            let table_rows = rows.iter().map(|&i| {
                let p = &app.store[i];
                let (state, color) = if p.in_use {
                    ("en service", Color::Green)
                } else if p.obsolete {
                    ("obsolète", Color::Yellow)
                } else {
                    ("inutilisé", Color::DarkGray)
                };
                Row::new([
                    Cell::from(if p.in_use { "   ".into() } else { check(app.store_checked.contains(&p.oem)) }),
                    Cell::from(p.oem.clone()),
                    Cell::from(p.original.clone()),
                    Cell::from(p.provider.clone()),
                    Cell::from(p.class.clone()),
                    Cell::from(p.version.clone()),
                    Cell::from(display_date(&p.date)),
                    Cell::from(human_size(p.size)),
                    Cell::from(state).fg(color),
                ])
            });
            let widths = [
                Constraint::Length(3),
                Constraint::Length(11),
                Constraint::Fill(1),
                Constraint::Fill(1),
                Constraint::Length(12),
                Constraint::Length(16),
                Constraint::Length(10),
                Constraint::Length(9),
                Constraint::Length(10),
            ];
            let t = Table::new(table_rows, widths)
                .header(header(&["", "Paquet", "INF d'origine", "Fabricant", "Classe", "Version", "Date", "Taille", "État"]))
                .block(block)
                .row_highlight_style(hl);
            f.render_stateful_widget(t, body, &mut app.states[STORE]);
        }
        _ => draw_backup(f, app, body, block),
    }

    f.render_widget(Paragraph::new(detail_lines(app, &updates)).block(Block::bordered().title(" Détails ")), details);

    let spin = if app.busy { ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][app.tick % 10] } else { "•" };
    let status = if let Some((q, _)) = &app.confirm {
        Line::from(format!(" {q} (o/N)")).yellow().bold()
    } else if let Some(r) = app.gauge {
        let g = LineGauge::default().ratio(r.clamp(0.0, 1.0)).label(format!(" {spin} {} {:>3.0} %", app.status, r * 100.0)).filled_style(Style::new().cyan());
        f.render_widget(g, bottom);
        Line::from("")
    } else if let Some(Input::Path(_)) = app.input {
        Line::from(format!(" Dossier : {}▏", app.path)).cyan()
    } else {
        Line::from(format!(" {spin} {}", app.status))
    };
    if app.gauge.is_none() || app.confirm.is_some() {
        f.render_widget(Paragraph::new(status), bottom);
    }

    let keys = match (&app.input, app.tab) {
        (Some(Input::Filter), _) => " Tapez pour filtrer · Entrée valider · Échap effacer",
        (Some(Input::Path(_)), _) => " Modifiez le chemin · Entrée valider · Échap annuler",
        (None, RESUME) => " Tab/1-5 onglets · r rescanner · U mettre à jour MesDrivers · ? aide · q quitter",
        (None, MATERIEL) => " Entrée fiche · / filtrer · s trier · m pilotes Microsoft · o Catalogue · ? aide · q quitter",
        (None, MAJ) => " Espace cocher · a tout · i installer · / filtrer · s trier · o Catalogue · ? aide · q quitter",
        (None, STORE) => " Espace cocher · a obsolètes · d supprimer · / filtrer · s trier · ? aide · q quitter",
        (None, _) => " ↑↓ choisir · Entrée lancer · ? aide · q quitter",
    };
    f.render_widget(Paragraph::new(keys).dark_gray(), help);

    if app.sheet.is_some() {
        draw_sheet(f, app);
    }
    if app.help {
        draw_help(f);
    }
    if let Some((q, _)) = &app.confirm {
        let area = centered(f.area(), 70, 7);
        f.render_widget(Clear, area);
        let text = vec![Line::from(""), Line::from(q.clone()).bold(), Line::from(""), Line::from("o = oui    N / Échap = non").dark_gray()];
        let block = Block::bordered().title(" Confirmation ").border_style(Style::new().yellow());
        f.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: true }).centered(), area);
    }
}

/// Rectangle centré de `w` % de large et `h` lignes de haut.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let width = (area.width * w / 100).max(30).min(area.width);
    let height = h.min(area.height);
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height }
}

fn draw_help(f: &mut Frame) {
    let keys = [
        ("Tab / 1-5 / clic", "changer d'onglet"),
        ("↑↓ PgUp PgDn molette", "naviguer"),
        ("/", "filtrer (Échap efface)"),
        ("s", "changer le tri"),
        ("Espace / clic sur [ ]", "cocher"),
        ("a", "tout cocher (Mises à jour) / les obsolètes (DriverStore)"),
        ("i", "installer les mises à jour cochées"),
        ("d", "supprimer les paquets DriverStore cochés"),
        ("Entrée", "fiche du périphérique (Matériel) / lancer (Sauvegarde)"),
        ("m", "afficher/masquer les pilotes Microsoft"),
        ("o", "ouvrir le Catalogue Microsoft du périphérique"),
        ("r", "rescanner"),
        ("U", "mettre à jour MesDrivers"),
        ("q", "quitter"),
    ];
    let lines: Vec<Line> = keys.iter().map(|(k, v)| Line::from(vec![Span::from(format!(" {k:<24}")).cyan(), Span::from(*v)])).collect();
    let area = centered(f.area(), 70, lines.len() as u16 + 4);
    f.render_widget(Clear, area);
    let block = Block::bordered().title(" Aide · une touche pour fermer ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Fiche du périphérique : infos complètes + versions du même pilote dans le DriverStore (retour arrière).
fn draw_sheet(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 85, 22);
    f.render_widget(Clear, area);
    let Some(i) = app.selected() else { return };
    let label = |l: &'static str| Span::from(format!("{l:<14}")).dark_gray();
    let mut lines: Vec<Line> = Vec::new();
    match i.checked_sub(app.broken.len()) {
        None => {
            let b = &app.broken[i];
            lines.push(Line::from(b.name.clone()).bold().red());
            lines.push(Line::from(vec![label("Erreur"), Span::from(format!("code {} : {}", b.code, error_label(b.code)))]));
            lines.push(Line::from(vec![label("HardwareID"), Span::from(b.hwid.clone())]));
            lines.push(Line::from(vec![label("Catalogue"), Span::from(format!("{:?}", b.catalog))]));
            lines.push(Line::from(""));
            lines.push(Line::from("Si un pilote est trouvé, il apparaît dans l'onglet 3 Mises à jour.").dark_gray());
        }
        Some(j) => {
            let d = &app.devices[j];
            lines.push(Line::from(d.name.clone()).bold());
            lines.push(Line::from(vec![label("Fabricant"), Span::from(d.provider.clone())]));
            lines.push(Line::from(vec![label("Version"), Span::from(format!("{} ({})", d.version, display_date(&d.date)))]));
            lines.push(Line::from(vec![label("Paquet"), Span::from(d.inf.clone())]));
            lines.push(Line::from(vec![label("HardwareID"), Span::from(d.hwid.clone())]));
            lines.push(Line::from(vec![label("Catalogue"), catalog_text(&d.catalog)]));
            lines.push(Line::from(""));
            let versions = app.sheet_versions();
            if versions.is_empty() {
                lines.push(Line::from("Aucune autre version de ce pilote dans le DriverStore (pilote Windows ou unique).").dark_gray());
            } else {
                lines.push(Line::from("Versions présentes sur ce PC (Entrée = revenir à cette version) :").cyan());
                for (k, p) in versions.iter().enumerate() {
                    let state = if p.in_use { "en service" } else if p.obsolete { "ancienne" } else { "inutilisée" };
                    let mut l = Line::from(format!(
                        "  {} {:<16} {:<10} {:<10} {:>9}  {state}",
                        if Some(k) == app.sheet { ">" } else { " " },
                        p.version,
                        display_date(&p.date),
                        p.oem,
                        human_size(p.size)
                    ));
                    if Some(k) == app.sheet {
                        l = l.reversed();
                    }
                    lines.push(l);
                }
            }
        }
    }
    let block = Block::bordered().title(" Fiche · ↑↓ version · Entrée revenir · o Catalogue · Échap fermer ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn catalog_text(c: &Catalog) -> Span<'static> {
    match c {
        Catalog::Skipped => Span::from("non vérifié (pilote Microsoft ou bus ignoré)").dark_gray(),
        Catalog::Pending => Span::from("vérification en cours…").dark_gray(),
        Catalog::NotFound => Span::from("aucun pilote Windows 10/11 publié").dark_gray(),
        Catalog::Error(e) => Span::from(format!("erreur : {e}")).red(),
        Catalog::Found { version, date, title, .. } => Span::from(format!("{version} ({}) · {title}", display_date(date))),
    }
}

fn draw_backup(f: &mut Frame, app: &mut App, area: Rect, block: Block) {
    let items = [
        ("Exporter tous les pilotes tiers", "Copie les pilotes installés (hors Microsoft) dans un dossier ou une clé USB, avant une réinstallation de Windows."),
        ("Restaurer depuis un dossier", "Installe d'un coup tous les pilotes d'une sauvegarde (après un point de restauration)."),
    ];
    let rows = items.iter().map(|(t, d)| Row::new([Cell::from(*t).bold(), Cell::from(*d).dark_gray()]).height(1));
    let t = Table::new(rows, [Constraint::Length(34), Constraint::Fill(1)]).block(block).row_highlight_style(Style::new().reversed());
    f.render_stateful_widget(t, area, &mut app.states[SAUVEGARDE]);
}

fn catalog_cell(c: &Catalog, current: &str) -> Cell<'static> {
    match c {
        Catalog::Skipped => Cell::from(""),
        Catalog::Pending => Cell::from("…").dark_gray(),
        Catalog::NotFound => Cell::from("—").dark_gray(),
        Catalog::Error(_) => Cell::from("erreur").red(),
        Catalog::Found { version, .. } if cmp_version(version, current).is_gt() => Cell::from(format!("{version} dispo")).yellow(),
        Catalog::Found { .. } => Cell::from("à jour").green(),
    }
}

fn detail_lines(app: &App, updates: &[crate::model::Update]) -> Vec<Line<'static>> {
    let label = |l: &'static str| Span::from(l).dark_gray();
    let line = |l: &'static str, v: String| Line::from(vec![label(l), Span::from(v)]);
    let none = || vec![Line::from("Aucune sélection.").dark_gray()];
    match app.tab {
        RESUME => vec![
            Line::from(format!("MesDrivers {} · journal : {}", env!("CARGO_PKG_VERSION"), crate::journal::path().display())).dark_gray(),
            Line::from("Sources : Windows Update et Catalogue Microsoft Update (pilotes signés).").dark_gray(),
        ],
        MAJ => {
            let Some(u) = app.selected().and_then(|i| updates.get(i)) else { return none() };
            let cur = current_version(&app.devices, &u.hwid).unwrap_or("inconnue");
            let concerne = if u.devices.is_empty() { u.hwid.clone() } else { u.devices.join(", ") };
            let mut date = display_date(&u.date);
            if date != u.date {
                date += " : date volontairement ancienne (convention Intel chipset), seule la version compte";
            }
            vec![
                Line::from(u.title.clone()).bold(),
                line("Concerne    ", format!("{} : {concerne}", u.devices.len().max(1))),
                line("Version     ", format!("{cur} → {}   ({}, {date})", if u.version.is_empty() { "voir titre" } else { &u.version }, u.provider)),
            ]
        }
        STORE => {
            let Some(p) = app.selected().and_then(|i| app.store.get(i)) else { return none() };
            let why = if p.in_use {
                "utilisé par un périphérique présent : conservé"
            } else if p.obsolete {
                "une version plus récente du même pilote est en service : suppression sans risque"
            } else {
                "inutilisé mais seule version (appareil débranché ?) : conservé par prudence"
            };
            vec![
                Line::from(format!("{} ({})", p.original, p.oem)).bold(),
                line("Fabricant   ", format!("{} · {} · {} ({}) · {}", p.provider, p.class, p.version, display_date(&p.date), human_size(p.size))),
                line("État        ", why.to_string()),
            ]
        }
        SAUVEGARDE => vec![
            Line::from(format!("Dossier proposé : {}", crate::worker::default_backup_dir().display())),
            Line::from("Astuce : exportez sur une clé USB avant de réinstaller Windows, puis Restaurer.").dark_gray(),
        ],
        _ => {
            let Some(i) = app.selected() else { return none() };
            let Some(j) = i.checked_sub(app.broken.len()) else {
                let b = &app.broken[i];
                let hint = match &b.catalog {
                    Catalog::Found { .. } => Line::from("Pilote trouvé au Catalogue : il est proposé dans l'onglet 3 Mises à jour.").yellow(),
                    _ => Line::from("Aucun pilote trouvé automatiquement : Entrée pour la fiche, o pour chercher au Catalogue.").dark_gray(),
                };
                return vec![
                    Line::from(vec![Span::from(b.name.clone()).bold().red(), Span::from(format!("   code {} : {}", b.code, error_label(b.code)))]),
                    line("HardwareID  ", b.hwid.clone()),
                    hint,
                ];
            };
            let d = &app.devices[j];
            let cat = match &d.catalog {
                Catalog::Skipped => "non vérifié (pilote Microsoft ou bus ignoré)".to_string(),
                Catalog::Pending => "vérification en cours…".to_string(),
                Catalog::NotFound => "aucun pilote Windows 10/11 publié pour ce matériel".to_string(),
                Catalog::Error(e) => format!("erreur : {e}"),
                Catalog::Found { version, date, title, .. } => format!("{version} ({}) · {title} · o pour ouvrir", display_date(date)),
            };
            vec![
                Line::from(vec![Span::from(d.name.clone()).bold(), Span::from(format!("   {} · {} · {} · {}", d.provider, d.version, display_date(&d.date), d.inf))]),
                line("HardwareID  ", d.hwid.clone()),
                line("Catalogue   ", cat),
            ]
        }
    }
}
