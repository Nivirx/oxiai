use std::time::{Duration, Instant};

use clap::Parser;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

#[derive(Parser)]
struct Args {
    #[arg(
        short,
        long,
        default_value = "mixtral:8x7b-instruct-v0.1-q5_K_M",
        help = "Model name to use"
    )]
    model: String,

    #[arg(
        short,
        long,
        help = "Should the response be streamed from ollama or sent all at once"
    )]
    stream: bool,
}

#[derive(Deserialize, Debug)]
struct StreamChunk {
    message: StreamMessage,
}

#[derive(Deserialize, Debug)]
struct StreamMessage {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize)]
struct Prompt<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Prompt<'a>>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    model: String,
    created_at: String,
    message: Message,
    done_reason: String,
    done: bool,
    total_duration: u64,
    eval_count: u64,
    eval_duration: u64,
    prompt_eval_count: u64,
    prompt_eval_duration: u64,
}

struct App {
    prompt: String,
    messages: Vec<String>,
    waiting: bool
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // setup crossterm
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App {
        prompt: String::new(),
        messages: vec![String::from("Welcome to the OxiAI TUI Interface!")],
        waiting: false
    };

    // parse arguments
    let args = Args::parse();

    let client = Client::new();
    let model_name = &args.model;

    let system_prompt =
        "[INST]You are a helpful, logical and extremely technical AI assistant.[INST]";

    loop {
        terminal.draw(|f| chat_ui(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char(c) => app.prompt.push(c),
                    KeyCode::Backspace => {
                        app.prompt.pop();
                    }
                    KeyCode::Enter => {
                        //TODO: refactor to a parser function to take the contents of the app.prompt vec and do fancy stuff with it (like commands)
                        let prompt = app.prompt.clone();
                        app.messages.push(format!("[INST]{}[INST]", prompt));
                        app.prompt.clear();

                        let user_prompt = app.messages.pop()
                            .expect("No user prompt received (empty user_prompt)");

                        let req = ChatRequest {
                            model: model_name,
                            stream: args.stream,
                            messages: vec![
                                Prompt {
                                    role: "system",
                                    content: system_prompt,
                                },
                                Prompt {
                                    role: "user",
                                    content: &user_prompt,
                                },
                            ],
                        };

                        match args.stream {
                            true => {
                                stream_ollama_response(&mut app, client.clone(), args.model.clone(), prompt, req)
                                    .await?;
                            }
                            false => {
                                ollama_response(&mut app, client.clone(), args.model.clone(), prompt, req)
                                    .await?;
                            }
                        }
                    }
                    KeyCode::Esc => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn stream_ollama_response(
    app: &mut App,
    client: Client,
    model: String,
    user_prompt: String,
    req: ChatRequest<'_>,
) -> anyhow::Result<()> {
    app.waiting = true;
    let mut resp = client
        .post("http://localhost:11434/api/chat")
        .json(&req)
        .send()
        .await?
        .bytes_stream();

    //TODO: since we haven't decoded the Steam we don't know if its sent the role part of the message
    // we'll need to figure out how to 'see the future' so to speak
    let mut assistant_line = String::from("Assistant : ");

    while let Some(chunk) = resp.next().await {
        let chunk = chunk?;
        for line in chunk.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let parsed: serde_json::Result<StreamChunk> = serde_json::from_slice(line);
            if let Ok(parsed) = parsed {
                assistant_line.push_str(&parsed.message.content);
            }
        }
    }

    app.messages.push(assistant_line);
    app.waiting = false;
    Ok(())
}

async fn ollama_response<'a>(
    app: &mut App,
    client: Client,
    model: String,
    user_prompt: String,
    req: ChatRequest<'a>,
) -> anyhow::Result<()> {

    app.waiting = true;

    let start = Instant::now();
    let resp: ChatResponse = client
        .post("http://localhost:11434/api/chat")
        .json(&req)
        .send()
        .await?
        .json()
        .await?;
    let elapsed = start.elapsed();
    

    app.messages.push(format!("{} : {}", resp.message.role, resp.message.content));
    app.messages.push(format!("System : Response generated via {} model with timestamp {}",
        resp.model, resp.created_at
    ));

    app.messages.push(format!("System : done_reason = {}, done = {}",
        resp.done_reason, resp.done
    ));

    app.messages.push(format!("System : Response timing statistics..."));

    app.messages.push(format!("System : Total elapsed wall time: {:.2?}", elapsed));
    app.messages.push(format!("System : Prompt tokens: {}", resp.prompt_eval_count));
    app.messages.push(format!("System : Prompt eval duration: {} ns", resp.prompt_eval_duration));
    app.messages.push(format!("System : Output tokens: {}", resp.eval_count));
    app.messages.push(format!("System : Output eval duration: {} ns", resp.eval_duration));
    app.messages.push(format!("System : Model 'warm up' time {}", (resp.total_duration - (resp.prompt_eval_duration + resp.eval_duration))));

    let token_speed = resp.eval_count as f64 / (resp.eval_duration as f64 / 1_000_000_000.0);
    app.messages.push(format!("System > Output generation speed: {:.2} tokens/sec", token_speed));

    app.waiting = false;

    Ok(())
}

fn chat_ui(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
        .split(f.area());

    let messages: Vec<Line> = app
        .messages
        .iter()
        .map(|m| Line::from(Span::raw(m.clone())))
        .collect();

    let messages_block = Paragraph::new(ratatui::text::Text::from(messages))
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
    f.set_cursor_position(
        Position::new(
            // the +3 comes from the 3 'characters' of space between the terminal edge and the text location
            // this places the text cursor after the last entered character
            chunks[1].x + app.prompt.len() as u16 + 3,
            chunks[1].y + 1,
        )
    );
}

