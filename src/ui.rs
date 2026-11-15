//! Rendu de l'interface (aucune logique métier ici).
use crate::model::{Catalog, cmp_version, current_version, error_label};
use crate::{App, Confirm};
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, Tabs};

pub fn draw(f: &mut Frame, app: &mut App) {
    let [top, body, details, bottom, help] = ratatui::layout::Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let titles = [
        format!(" Matériel ({}) ", app.devices.len() + app.broken.len()),
        format!(" Mises à jour ({}) ", app.updates.len()),
    ];
    f.render_widget(Tabs::new(titles).select(app.tab).highlight_style(Style::new().bold().reversed()).divider("│"), top);

    let rows = app.rows(app.tab);
    let hl = Style::new().reversed();
    let header = |cols: &[&'static str]| Row::new(cols.to_vec()).style(Style::new().bold().fg(Color::Cyan));
    let mut title = if app.tab == 0 { " Pilotes installés ".to_string() } else { " Mises à jour Windows Update ".to_string() };
    if !app.filter.is_empty() || app.typing {
        title += &format!("· filtre « {} » ", app.filter);
    }
    if app.tab == 0 && app.hidden_ms() > 0 {
        title += &format!("· {} pilotes Microsoft masqués (m) ", app.hidden_ms());
    }

    if app.tab == 0 {
        let nb = app.broken.len();
        let table_rows = rows.iter().map(|&i| match i.checked_sub(nb) {
            None => {
                let b = &app.broken[i];
                Row::new([b.name.clone(), String::new(), format!("Code {} : {}", b.code, error_label(b.code)), String::new(), String::new()])
                    .style(Style::new().fg(Color::Red))
            }
            Some(j) => {
                let d = &app.devices[j];
                Row::new([
                    Cell::from(d.name.clone()),
                    Cell::from(d.provider.clone()),
                    Cell::from(d.version.clone()),
                    Cell::from(d.date.clone()),
                    catalog_cell(&d.catalog, &d.version),
                ])
            }
        });
        let t = Table::new(
            table_rows,
            [Constraint::Fill(3), Constraint::Fill(1), Constraint::Length(22), Constraint::Length(10), Constraint::Length(20)],
        )
        .header(header(&["Périphérique", "Fabricant", "Version", "Date", "Catalogue MS"]))
        .block(Block::bordered().title(title))
        .row_highlight_style(hl);
        f.render_stateful_widget(t, body, &mut app.states[0]);
    } else {
        let table_rows = rows.iter().map(|&i| {
            let u = &app.updates[i];
            let cur = current_version(&app.devices, &u.hwid).unwrap_or("?");
            let mark = if app.checked.get(i).copied().unwrap_or(false) { "[x]" } else { "[ ]" };
            Row::new([mark.to_string(), u.title.clone(), u.provider.clone(), cur.to_string(), u.date.clone()])
        });
        let t = Table::new(
            table_rows,
            [Constraint::Length(3), Constraint::Fill(3), Constraint::Fill(1), Constraint::Length(18), Constraint::Length(10)],
        )
        .header(header(&["", "Pilote proposé", "Fabricant", "Version actuelle", "Date"]))
        .block(Block::bordered().title(title))
        .row_highlight_style(hl);
        f.render_stateful_widget(t, body, &mut app.states[1]);
    }

    f.render_widget(Paragraph::new(detail_lines(app)).block(Block::bordered().title(" Détails ")), details);

    let spin = if app.busy { ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][app.tick % 10] } else { "•" };
    let status = match app.confirm {
        Confirm::Install => Line::from(format!(" Installer {} pilote(s) ? Un point de restauration sera créé avant. (o/N)", app.picks().len())).yellow().bold(),
        Confirm::Force => Line::from(format!(" {} — installer quand même sans point de restauration ? (o/N)", app.status)).red().bold(),
        Confirm::No => Line::from(format!(" {spin} {}", app.status)),
    };
    f.render_widget(Paragraph::new(status), bottom);

    let keys = if app.typing {
        " Tapez pour filtrer · Entrée valider · Échap effacer"
    } else if app.tab == 0 {
        " Tab onglet · ↑↓ naviguer · / filtrer · m pilotes Microsoft · o Catalogue · r rescanner · q quitter"
    } else {
        " Tab onglet · ↑↓ naviguer · Espace cocher · a tout · i installer · / filtrer · o Catalogue · q quitter"
    };
    f.render_widget(Paragraph::new(keys).dark_gray(), help);
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

fn detail_lines(app: &App) -> Vec<Line<'static>> {
    let label = |l: &'static str| Span::from(l).dark_gray();
    let Some(i) = app.selected() else { return vec![Line::from("Aucune sélection.").dark_gray()] };
    if app.tab == 1 {
        let u = &app.updates[i];
        let cur = current_version(&app.devices, &u.hwid).unwrap_or("inconnue");
        return vec![
            Line::from(u.title.clone()).bold(),
            Line::from(vec![label("HardwareID  "), Span::from(u.hwid.clone())]),
            Line::from(vec![
                label("Fabricant   "),
                Span::from(format!("{}   ", u.provider)),
                label("Version actuelle "),
                Span::from(format!("{cur}   ")),
                label("Date proposée "),
                Span::from(u.date.clone()),
            ]),
        ];
    }
    let nb = app.broken.len();
    let Some(j) = i.checked_sub(nb) else {
        let b = &app.broken[i];
        return vec![
            Line::from(b.name.clone()).bold().red(),
            Line::from(format!("Code {} : {}", b.code, error_label(b.code))),
            Line::from("Cherchez un pilote dans l'onglet Mises à jour, ou le Gestionnaire de périphériques.").dark_gray(),
        ];
    };
    let d = &app.devices[j];
    let cat = match &d.catalog {
        Catalog::Skipped => "non vérifié (pilote Microsoft ou bus ignoré)".to_string(),
        Catalog::Pending => "vérification en cours…".to_string(),
        Catalog::NotFound => "aucun pilote publié pour ce matériel".to_string(),
        Catalog::Error(e) => format!("erreur : {e}"),
        Catalog::Found { version, date, title } => format!("{version} du {date} ({title}) · o pour ouvrir"),
    };
    vec![
        Line::from(vec![Span::from(d.name.clone()).bold(), Span::from(format!("   {} · {} · {}", d.provider, d.version, d.date))]),
        Line::from(vec![label("HardwareID  "), Span::from(d.hwid.clone())]),
        Line::from(vec![label("Catalogue   "), Span::from(cat)]),
    ]
}
