use std::io::{self, Stdout};

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

pub enum Choice {
    Pin { sha: String, tag: String },
    Skip,
    Quit,
}

pub struct TagEntry {
    pub tag: String,
    pub sha: String,
}

impl TagEntry {
    pub fn from_pairs(pairs: Vec<(String, String)>) -> Vec<Self> {
        pairs
            .into_iter()
            .map(|(tag, sha)| TagEntry { tag, sha })
            .collect()
    }
}

struct App<'a> {
    mode: &'a str,
    file: &'a str,
    action: &'a str,
    current_ref: &'a str,
    tags: &'a [TagEntry],
    list_state: ListState,
    context_lines: Vec<String>,
    context_highlight: usize,
    owner: &'a str,
    repo: &'a str,
}

impl<'a> App<'a> {
    fn new(
        mode: &'a str,
        file: &'a str,
        action: &'a str,
        current_ref: &'a str,
        tags: &'a [TagEntry],
        context_lines: Vec<String>,
        context_highlight: usize,
        owner: &'a str,
        repo: &'a str,
    ) -> Self {
        let mut list_state = ListState::default();
        if !tags.is_empty() {
            list_state.select(Some(0));
        }
        App {
            mode,
            file,
            action,
            current_ref,
            tags,
            list_state,
            context_lines,
            context_highlight,
            owner,
            repo,
        }
    }

    fn selected(&self) -> Option<&TagEntry> {
        self.list_state.selected().and_then(|i| self.tags.get(i))
    }

    fn move_up(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        if i > 0 {
            self.list_state.select(Some(i - 1));
        }
    }

    fn move_down(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        if i + 1 < self.tags.len() {
            self.list_state.select(Some(i + 1));
        }
    }

    fn open_changelog(&self) {
        if let Some(entry) = self.selected() {
            let url = format!(
                "https://github.com/{}/{}/compare/{}...{}",
                self.owner, self.repo, self.current_ref, entry.tag
            );
            open_url(&url);
        }
    }
}

/// Show the interactive version picker TUI. Returns the user's choice.
///
/// - `mode`:             "migrate" or "update"
/// - `file`:             display path of the workflow file
/// - `action`:           e.g. "actions/checkout"
/// - `current_ref`:      current ref in the workflow (tag name or SHA)
/// - `tags`:             available versions sorted newest-first
/// - `context_lines`:    YAML lines surrounding the uses: directive
/// - `context_highlight`: index of the uses: line within context_lines
/// - `owner`/`repo`:     for building the GitHub compare URL
pub fn pick_version(
    mode: &str,
    file: &str,
    action: &str,
    current_ref: &str,
    tags: &[TagEntry],
    context_lines: Vec<String>,
    context_highlight: usize,
    owner: &str,
    repo: &str,
) -> io::Result<Choice> {
    if tags.is_empty() {
        eprintln!("no tags found for {action} — skipping");
        return Ok(Choice::Skip);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(
        mode,
        file,
        action,
        current_ref,
        tags,
        context_lines,
        context_highlight,
        owner,
        repo,
    );

    let result = run_loop(&mut terminal, &mut app);

    // Always restore the terminal, even on error
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<Choice> {
    loop {
        terminal.draw(|frame| render(frame, app))?;

        if let Event::Key(key) = event::read()? {
            // Ctrl+C / Ctrl+Q → quit
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
            {
                return Ok(Choice::Quit);
            }

            match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                KeyCode::Enter => {
                    if let Some(entry) = app.selected() {
                        return Ok(Choice::Pin {
                            sha: entry.sha.clone(),
                            tag: entry.tag.clone(),
                        });
                    }
                }
                KeyCode::Char('c') => app.open_changelog(),
                KeyCode::Char('s') => return Ok(Choice::Skip),
                KeyCode::Char('q') | KeyCode::Esc => return Ok(Choice::Quit),
                _ => {}
            }
        }
    }
}

fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let vertical = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(3), // info (file / action / current)
        Constraint::Min(6),    // content
        Constraint::Length(1), // help
    ]);
    let [title_area, info_area, content_area, help_area] = vertical.areas(area);

    render_title(frame, app, title_area);
    render_info(frame, app, info_area);
    render_content(frame, app, content_area);
    render_help(frame, help_area);
}

fn render_title(frame: &mut Frame, app: &App, area: Rect) {
    let label = match app.mode {
        "update" => " gh-shaping — interactive update ",
        _ => " gh-shaping — interactive migrate ",
    };
    let title = Paragraph::new(label).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(title, area);
}

fn render_info(frame: &mut Frame, app: &App, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled("  File:    ", Style::default().fg(Color::DarkGray)),
            Span::raw(app.file),
        ]),
        Line::from(vec![
            Span::styled("  Action:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                app.action,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Current: ", Style::default().fg(Color::DarkGray)),
            Span::styled(app.current_ref, Style::default().fg(Color::Green)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_content(frame: &mut Frame, app: &mut App, area: Rect) {
    let horizontal = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]);
    let [list_area, ctx_area] = horizontal.areas(area);

    render_version_list(frame, app, list_area);
    render_context(frame, app, ctx_area);
}

fn render_version_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .tags
        .iter()
        .map(|entry| {
            let sha_short = if entry.sha.len() >= 12 {
                &entry.sha[..12]
            } else {
                &entry.sha
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {:<18}", entry.tag),
                    Style::default().fg(Color::White),
                ),
                Span::styled(sha_short.to_string(), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Versions "))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_context(frame: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = if app.context_lines.is_empty() {
        vec![Line::from(Span::styled(
            "  (no context available)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.context_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if i == app.context_highlight {
                    Line::from(Span::styled(
                        format!("► {line}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("  {line}"),
                        Style::default().fg(Color::Gray),
                    ))
                }
            })
            .collect()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Workflow Context ");
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let help = Paragraph::new(
        "  [↑↓ / jk] navigate   [Enter] pin   [c] changelog in browser   [s] skip   [q] quit",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", url])
        .spawn();
}
