use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

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
