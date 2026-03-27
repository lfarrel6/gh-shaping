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

#[derive(Clone, Copy, PartialEq)]
enum View {
    Tags,
    Branches,
}

struct App<'a> {
    mode: &'a str,
    file: &'a str,
    action: &'a str,
    current_ref: &'a str,
    tags: &'a [TagEntry],
    branches: &'a [TagEntry],
    view: View,
    tags_state: ListState,
    branches_state: ListState,
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
        branches: &'a [TagEntry],
        context_lines: Vec<String>,
        context_highlight: usize,
        owner: &'a str,
        repo: &'a str,
    ) -> Self {
        // Start in branches view only if there are no tags but there are branches
        let view = if tags.is_empty() && !branches.is_empty() {
            View::Branches
        } else {
            View::Tags
        };

        let mut tags_state = ListState::default();
        if !tags.is_empty() {
            tags_state.select(Some(0));
        }
        let mut branches_state = ListState::default();
        if !branches.is_empty() {
            branches_state.select(Some(0));
        }

        App {
            mode,
            file,
            action,
            current_ref,
            tags,
            branches,
            view,
            tags_state,
            branches_state,
            context_lines,
            context_highlight,
            owner,
            repo,
        }
    }

    fn selected(&self) -> Option<&TagEntry> {
        match self.view {
            View::Tags => self.tags_state.selected().and_then(|i| self.tags.get(i)),
            View::Branches => self
                .branches_state
                .selected()
                .and_then(|i| self.branches.get(i)),
        }
    }

    fn move_up(&mut self) {
        match self.view {
            View::Tags => {
                let len = self.tags.len();
                if len == 0 {
                    return;
                }
                let i = self.tags_state.selected().unwrap_or(0);
                self.tags_state
                    .select(Some(if i == 0 { len - 1 } else { i - 1 }));
            }
            View::Branches => {
                let len = self.branches.len();
                if len == 0 {
                    return;
                }
                let i = self.branches_state.selected().unwrap_or(0);
                self.branches_state
                    .select(Some(if i == 0 { len - 1 } else { i - 1 }));
            }
        }
    }

    fn move_down(&mut self) {
        match self.view {
            View::Tags => {
                let len = self.tags.len();
                if len == 0 {
                    return;
                }
                let i = self.tags_state.selected().unwrap_or(0);
                self.tags_state.select(Some((i + 1) % len));
            }
            View::Branches => {
                let len = self.branches.len();
                if len == 0 {
                    return;
                }
                let i = self.branches_state.selected().unwrap_or(0);
                self.branches_state.select(Some((i + 1) % len));
            }
        }
    }

    fn toggle_view(&mut self) {
        if self.branches.is_empty() {
            return;
        }
        self.view = match self.view {
            View::Tags => View::Branches,
            View::Branches => View::Tags,
        };
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
/// - `mode`:              "migrate" or "update"
/// - `file`:              display path of the workflow file
/// - `action`:            e.g. "actions/checkout"
/// - `current_ref`:       current ref in the workflow (tag name or SHA)
/// - `tags`:              available versions sorted newest-first
/// - `branches`:          available branches sorted alphabetically; empty when
///                        `--include-branches` was not passed
/// - `context_lines`:     YAML lines surrounding the uses: directive
/// - `context_highlight`: index of the uses: line within context_lines
/// - `owner`/`repo`:      for building the GitHub compare URL
pub fn pick_version(
    mode: &str,
    file: &str,
    action: &str,
    current_ref: &str,
    tags: &[TagEntry],
    branches: &[TagEntry],
    context_lines: Vec<String>,
    context_highlight: usize,
    owner: &str,
    repo: &str,
) -> io::Result<Choice> {
    if tags.is_empty() && branches.is_empty() {
        eprintln!("no tags or branches found for {action} — skipping");
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
        branches,
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
                KeyCode::Tab => app.toggle_view(),
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
    render_help(frame, app, help_area);
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
    let (entries, title): (&[TagEntry], &str) = match app.view {
        View::Tags => (app.tags, " Tags "),
        View::Branches => (app.branches, " Branches "),
    };

    let items: Vec<ListItem> = entries
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
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    match app.view {
        View::Tags => frame.render_stateful_widget(list, area, &mut app.tags_state),
        View::Branches => frame.render_stateful_widget(list, area, &mut app.branches_state),
    }
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

fn render_help(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.branches.is_empty() {
        "  [↑↓ / jk] navigate   [Enter] pin   [c] changelog in browser   [s] skip   [q] quit"
    } else {
        "  [↑↓ / jk] navigate   [Enter] pin   [Tab] tags/branches   [c] changelog in browser   [s] skip   [q] quit"
    };
    let help = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
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
