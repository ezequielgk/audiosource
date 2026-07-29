use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Gauge},
    Terminal,
};
use std::io::{stdout, Write};
use std::time::Duration;

pub fn run_tui() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut running = true;
    let mut status = "Idle";
    
    while running {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(5),
                        Constraint::Length(3),
                        Constraint::Length(3),
                    ]
                    .as_ref(),
                )
                .split(f.area());
                
            let title = Paragraph::new(" AudioSource TUI (Rust) ")
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(title, chunks[0]);
            
            let log_block = Paragraph::new("App running in background...\nPress [s] to Start, [m] to Mute, [q] to Quit")
                .block(Block::default().title(" Log ").borders(Borders::ALL));
            f.render_widget(log_block, chunks[1]);
            
            let gauge = Gauge::default()
                .block(Block::default().title(" Mic Volume ").borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Green))
                .percent(100);
            f.render_widget(gauge, chunks[2]);
            
            let controls = Paragraph::new("[S] Start  [R] Restart  [C] Stop  [M] Mute  [T] Hide  [Q] Quit")
                .style(Style::default().fg(Color::Yellow))
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(controls, chunks[3]);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        running = false;
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        // start logic
                        status = "Streaming";
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        // mute logic
                        let _ = crate::daemon::set_volume(None, "0%");
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    
    Ok(())
}
