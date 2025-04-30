use std::io::IsTerminal;

use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, Event as CEvent, EventStream, KeyCode,
    MouseButton,
};

use ratatui::CompletedFrame;
use ratatui::prelude::Backend;
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::AppState;

pub struct OxiTerminal {
    handle: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl OxiTerminal {
    pub fn setup() -> Self {
        enable_raw_mode(); // crossterm
        let mut stdout_handle = std::io::stdout();
        crossterm::execute!(stdout_handle, EnterAlternateScreen, EnableMouseCapture);
        let backend = CrosstermBackend::new(stdout_handle);
        let mut handle = Terminal::new(backend).expect("unable to open a terminal");
        handle.clear();

        OxiTerminal { handle }
    }

    pub fn do_draw(&mut self, app: &AppState) -> CompletedFrame {
        self.handle
            .draw(|f| OxiTerminal::chat_ui(f, app))
            .expect("failed to draw to framebuffer")
    }

    pub fn term_cleanup(&mut self) -> anyhow::Result<()> {
        disable_raw_mode()?;
        crossterm::execute!(
            self.handle.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.handle.show_cursor()?;

        Ok(())
    }

    //FIXME: awaiting refactor
    pub fn chat_ui(f: &mut ratatui::Frame, app: &crate::AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
            .split(f.area());

        let chat_messages: Vec<Line> = app
            .messages
            .iter()
            .map(|m| {
                Line::from(Span::raw(format!(
                    "{}: {}",
                    m.role.to_string(),
                    m.to_string()
                )))
            })
            .collect();

        let messages_block = Paragraph::new(ratatui::text::Text::from(chat_messages))
            .block(Block::default().borders(Borders::ALL).title("Chat"))
            .wrap(ratatui::widgets::Wrap { trim: true })
            .scroll((
                app.messages
                    .len()
                    .saturating_sub((chunks[0].height - 2) as usize) as u16,
                0,
            ));

        f.render_widget(messages_block, chunks[0]);

        let input_text = if app.waiting {
            format!("> {} (waiting...)", &app.prompt)
        } else {
            format!("> {}", app.prompt)
        };

        let input = Paragraph::new(input_text)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title("Input"));
        f.render_widget(input, chunks[1]);

        use ratatui::layout::Position;
        f.set_cursor_position(Position::new(
            // the +3 comes from the 3 'characters' of space between the terminal edge and the text location
            // this places the text cursor after the last entered character
            chunks[1].x + app.prompt.len() as u16 + 3,
            chunks[1].y + 1,
        ));
    }
}
