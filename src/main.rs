// Hors Windows seul le squelette UI tourne : les champs remplis par WMI/WU y sont inutilisés.
#![cfg_attr(not(windows), allow(dead_code))]
mod model;
mod worker;
#[cfg(windows)]
mod inventory;
#[cfg(windows)]
mod wu;

use model::{BrokenDevice, Device, Update, current_version, error_label};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Row, Table, TableState, Tabs};
use ratatui::{DefaultTerminal, Frame};
use std::time::Duration;
use worker::{Cmd, Msg};

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
    confirm: bool,
    tick: usize,
}

impl App {
    fn len(&self) -> usize {
        if self.tab == 0 { self.broken.len() + self.devices.len() } else { self.updates.len() }
    }
    fn picks(&self) -> Vec<usize> {
        self.checked.iter().enumerate().filter(|(_, c)| **c).map(|(i, _)| i).collect()
    }
}

fn main() -> std::io::Result<()> {
    let (tx, rx) = worker::spawn();
    let mut app = App::default();
    let _ = tx.send(Cmd::Scan);
    app.busy = true;
    app.status = "Analyse du matériel et recherche sur Windows Update…".into();
    ratatui::run(|t| run(t, &mut app, &tx, &rx))
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    tx: &std::sync::mpsc::Sender<Cmd>,
    rx: &std::sync::mpsc::Receiver<Msg>,
) -> std::io::Result<()> {
    loop {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Inventory(d, b) => (app.devices, app.broken) = (d, b),
                Msg::Updates(u) => {
                    app.checked = vec![false; u.len()];
                    app.updates = u;
                    app.states[1].select((!app.updates.is_empty()).then_some(0));
                }
                Msg::Installed(s) | Msg::Error(s) => app.status = s,
                Msg::Done => {
                    app.busy = false;
                    if app.status.ends_with('…') {
                        app.status = format!("{} pilote(s) à mettre à jour.", app.updates.len());
                    }
                }
            }
        }
        if app.states[0].selected().is_none() && app.len() > 0 && app.tab == 0 {
            app.states[0].select(Some(0));
        }
        app.tick += 1;
        terminal.draw(|f| draw(f, app))?;

        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        // Windows envoie aussi les relâchements de touche : on ne garde que l'appui.
        let Event::Key(k) = event::read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if app.confirm {
            app.confirm = false;
            if matches!(k.code, KeyCode::Char('o' | 'O' | 'y' | 'Y')) {
                let picks = app.picks();
                app.status = format!("Téléchargement et installation de {} pilote(s)…", picks.len());
                app.busy = true;
                let _ = tx.send(Cmd::Install(picks));
            } else {
                app.status = "Installation annulée.".into();
            }
            continue;
        }
        let n = app.len();
        let st = &mut app.states[app.tab];
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => app.tab ^= 1,
            KeyCode::Down | KeyCode::Char('j') if n > 0 => st.select(Some(st.selected().map_or(0, |i| (i + 1).min(n - 1)))),
            KeyCode::Up | KeyCode::Char('k') if n > 0 => st.select(Some(st.selected().map_or(0, |i| i.saturating_sub(1)))),
            KeyCode::Char(' ') if app.tab == 1 => {
                if let Some(c) = st.selected().and_then(|i| app.checked.get_mut(i)) {
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
                    app.confirm = true;
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

fn draw(f: &mut Frame, app: &mut App) {
    let [top, body, bottom, help] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1), Constraint::Length(1)])
            .areas(f.area());

    let titles = [
        format!(" Matériel ({}) ", app.devices.len() + app.broken.len()),
        format!(" Mises à jour ({}) ", app.updates.len()),
    ];
    f.render_widget(
        Tabs::new(titles).select(app.tab).highlight_style(Style::new().bold().reversed()).divider("│"),
        top,
    );

    let hl = Style::new().reversed();
    let header = |cols: &[&'static str]| Row::new(cols.to_vec()).style(Style::new().bold().fg(Color::Cyan));
    if app.tab == 0 {
        let rows = app
            .broken
            .iter()
            .map(|b| {
                Row::new([b.name.clone(), String::new(), format!("Code {} : {}", b.code, error_label(b.code)), String::new()])
                    .style(Style::new().fg(Color::Red))
            })
            .chain(app.devices.iter().map(|d| Row::new([d.name.clone(), d.provider.clone(), d.version.clone(), d.date.clone()])));
        let t = Table::new(rows, [Constraint::Fill(3), Constraint::Fill(1), Constraint::Length(30), Constraint::Length(10)])
            .header(header(&["Périphérique", "Fabricant", "Version", "Date"]))
            .block(Block::bordered().title(" Pilotes installés "))
            .row_highlight_style(hl);
        f.render_stateful_widget(t, body, &mut app.states[0]);
    } else {
        let rows = app.updates.iter().zip(&app.checked).map(|(u, c)| {
            let cur = current_version(&app.devices, &u.hwid).unwrap_or("?");
            Row::new([if *c { "[x]" } else { "[ ]" }.to_string(), u.title.clone(), u.provider.clone(), cur.to_string(), u.date.clone()])
        });
        let t = Table::new(
            rows,
            [Constraint::Length(3), Constraint::Fill(3), Constraint::Fill(1), Constraint::Length(18), Constraint::Length(10)],
        )
        .header(header(&["", "Pilote proposé (Windows Update)", "Fabricant", "Version actuelle", "Date"]))
        .block(Block::bordered().title(" Mises à jour disponibles "))
        .row_highlight_style(hl);
        f.render_stateful_widget(t, body, &mut app.states[1]);
    }

    let spin = if app.busy { ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][app.tick % 10] } else { "•" };
    let status = if app.confirm {
        Line::from(format!(" Installer {} pilote(s) ? (o/N)", app.picks().len())).yellow().bold()
    } else {
        Line::from(format!(" {spin} {}", app.status))
    };
    f.render_widget(Paragraph::new(status), bottom);
    f.render_widget(
        Paragraph::new(" Tab onglet · ↑↓ naviguer · Espace cocher · a tout · i installer · r rescanner · q quitter").dark_gray(),
        help,
    );
}
