use std::borrow::Cow;
use std::time::{Duration, Instant};

use chat::Message;
use clap::Parser;
use futures_util::StreamExt;
use reqwest::Client;

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
use serde::{Deserialize, Serialize};

mod chat;

#[derive(Parser)]
struct Args {
    #[arg(
        short,
        long,
        default_value = "mistral:latest",
        help = "Model name to use"
    )]
    model: String,

    #[arg(
        short,
        long,
        help = "Should the response be streamed from ollama or sent all at once"
    )]
    stream: bool,

    #[arg(short, long, help = "Show statistics in non-stream mode?")]
    nerd_stats: bool,
}

struct App {
    args: Args,
    prompt: String,
    messages: Vec<Message>,
    waiting: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // parse arguments
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            e.print().expect("Error writing clap error");
            std::process::exit(0);
        }
    };

    // setup crossterm
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App {
        args,
        prompt: String::new(),
        messages: vec![],
        waiting: false,
    };

    let client = Client::new();

    let header_prompt =
        r#"SYSTEM: You are "OxiAI", a logical, personal assistant that answers *only* via JSON"#;

    let tools_list = include_str!("tool/tools_list.json")
        .parse::<serde_json::Value>()?
        .to_string();

    let rules_prompt = r#"Rules:
1. Think silently, Never reveal your chain-of-thought.
2. To use a tool: {"action":"<tool>","arguments":{...}}
3. To reply directly: {"action":"chat","arguments":{"response":"..."}
4. If a question is vague, comparative, descriptive, or about ideas rather than specifics: use the web_search tool.
5. If a question clearly names a specific object, animal, person, place: use the wiki_search tool.
6. Base claims strictly on provided data or tool results. If unsure, say so.
7. Perform a JSON Self-check to ensure valid, minified, UTF-8 JSON.
8. Finish with a coherent sentence; if you reach four consecutive newlines: **STOP.**"#;

    let example_prompt = format!(
        "Example 1:{user_q_1}\n{assistant_tool_request_1}\n{tool_result_1}\n{assistant_a_1}",
        user_q_1 = r#"user: {"action":"chat", "arguments":{"response":"Provide a summary of the American Crow.", "source":"user"}}"#,
        assistant_tool_request_1 = format!(
            "assistant: {{ \"action\":\"wiki_search\",\"arguments\":{{\"query\":\"American Crow\"}} }}"
        ),
        tool_result_1 = format!(
            "tool: {{ \"action\":\"wiki_search\",\"arguments\":{{\"result\":\"{search_data}\"}} }}",
            search_data = include_str!("tool/american_crow_wikipedia.md").to_string()
        ),
        assistant_a_1 = format!(
            "assistant: {{ \"action\":\"chat\",\"arguments\":{{\"response\":\"{example1_assistant_message}\"}} }}",
            example1_assistant_message =
                include_str!("tool/american_crow_example1_message.md").to_string()
        )
    );

    //let user_info_prompt = r#""#;
    let system_prompt = format!(
        "{header_prompt}\n
        {tools_list}\n\n
        {rules_prompt}\n"
    );

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
                        let message_args = args_builder! {
                            "response" => app.prompt.clone(),
                        };
                        app.prompt.clear();

                        app.messages.push(chat::Message::new(
                            chat::MessageRoles::User,
                            chat::Action::ChatMessage,
                            message_args));

                        let mut prompts = vec![chat::Prompt {
                            role: Cow::Borrowed("system"),
                            content: Cow::Borrowed(&system_prompt),
                        }];
                        prompts.extend(
                            app.messages
                                .iter()
                                .map(|msg| chat::Prompt::from(msg.clone())),
                        );

                        let req = chat::ChatRequest {
                            model: &app.args.model.clone(),
                            stream: app.args.stream,
                            format: "json",
                            stop: vec!["\n\n\n\n"],
                            options: Some(chat::ChatOptions {
                                temperature: Some(0.3),
                                top_p: Some(0.92),
                                top_k: Some(50),
                                repeat_penalty: Some(1.1),
                                seed: None,
                            }),
                            messages: prompts,
                        };

                        app.waiting = true;
                        match app.args.stream {
                            true => {
                                todo!();
                                stream_ollama_response(&mut app, client.clone(), req).await?;
                            }
                            false => {
                                batch_ollama_response(&mut app, client.clone(), req).await?;
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

//FIXME: streaming replies are harder to work with for now, save this for the future
async fn stream_ollama_response(
    app: &mut App,
    client: Client,
    req: chat::ChatRequest<'_>,
) -> anyhow::Result<()> {
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
            let parsed: serde_json::Result<chat::StreamChunk> = serde_json::from_slice(line);
            if let Ok(parsed) = parsed {
                assistant_line.push_str(&parsed.message.content.to_string());
            }
        }
    }

    //FIXME: fix this later
    //app.messages.push(assistant_line);

    Ok(())
}

async fn batch_ollama_response<'a>(
    app: &mut App,
    client: Client,
    req: chat::ChatRequest<'a>,
) -> anyhow::Result<()> {
    let start = Instant::now();
    let resp = client
        .post("http://localhost:11434/api/chat")
        .json(&req)
        .send()
        .await?;
    let elapsed = start.elapsed();

    let status = resp.status();
    let headers = resp.headers().clone();
    let body_bytes = resp.bytes().await?;    
    
    match serde_json::from_slice::<chat::ChatResponse>(&body_bytes) {
        Ok(r) => app.messages.push(r.message),
        Err(e) => {
            println!("Failed to parse JSON: {}", e);
            println!("Status: {}", status);
            println!("Headers: {:#?}", headers);
            // Try to print the body as text for debugging
            if let Ok(body_text) = std::str::from_utf8(&body_bytes) {
                println!("Body text: {}", body_text);
            } else {
                println!("Body was not valid UTF-8");
            }
        }
    }

    /*
    if app.args.nerd_stats {
        app.messages.push(format!(
            "System : Response generated via {} model with timestamp {}",
            resp.model, resp.created_at
        ));

        app.messages.push(format!(
            "System : done_reason = {}, done = {}",
            resp.done_reason, resp.done
        ));

        app.messages
            .push(format!("System : Response timing statistics..."));

        app.messages
            .push(format!("System : Total elapsed wall time: {:.2?}", elapsed));
        app.messages.push(format!(
            "System : Prompt tokens: {}",
            resp.prompt_eval_count
        ));
        app.messages.push(format!(
            "System : Prompt eval duration: {} ns",
            resp.prompt_eval_duration
        ));
        app.messages
            .push(format!("System : Output tokens: {}", resp.eval_count));
        app.messages.push(format!(
            "System : Output eval duration: {} ns",
            resp.eval_duration
        ));
        app.messages.push(format!(
            "System : Model 'warm up' time {}",
            (resp.total_duration - (resp.prompt_eval_duration + resp.eval_duration))
        ));

        let token_speed = resp.eval_count as f64 / (resp.eval_duration as f64 / 1_000_000_000.0);
        app.messages.push(format!(
            "System > Output generation speed: {:.2} tokens/sec",
            token_speed
        ));
    }
     */
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
        .map(|m| Line::from(Span::raw(m.to_string())))
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
    f.set_cursor_position(Position::new(
        // the +3 comes from the 3 'characters' of space between the terminal edge and the text location
        // this places the text cursor after the last entered character
        chunks[1].x + app.prompt.len() as u16 + 3,
        chunks[1].y + 1,
    ));
}
