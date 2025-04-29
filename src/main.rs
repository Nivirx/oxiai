use std::borrow::Cow;
use std::pin::Pin;
use std::time::{Duration, Instant};

use chat::{Action, Message};
use clap::Parser;
use futures_util::StreamExt;
use reqwest::Client;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use ratatui::{Terminal, backend::CrosstermBackend};

mod chat;
mod ui;

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
        help = "(Broken) Should the response be streamed from ollama or sent all at once"
    )]
    stream: bool,

    #[arg(short, long, help = "(Broken) Show statistics in non-stream mode?")]
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
    let mut stdout_handle = std::io::stdout();
    crossterm::execute!(stdout_handle, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout_handle);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App {
        args,
        prompt: String::new(),
        messages: vec![],
        waiting: false,
    };

    let client = Client::new();

    let header_prompt = r#"SYSTEM: You are "OxiAI", a logical, personal assistant that answers *only* via valid, minified, UTF-8 JSON."#;

    let tools_list = include_str!("data/tools_list.json")
        .parse::<serde_json::Value>()?
        .to_string();

    let rules_prompt = r#"Rules:
1. Think silently, Never reveal your chain-of-thought.
2. To use a tool: {"action":"<tool>","arguments":{...}}
3. To reply directly: {"action":"chat","arguments":{"response":"..."}
4. If a question is vague, comparative, descriptive, or about ideas rather than specifics: use the web_search tool.
5. If a question clearly names a specific object, animal, person, place: use the wiki_search tool.
6. Base claims strictly on provided data or tool results. If unsure, say so.
7. Check your output; If you reach four consecutive newlines: *stop*"#;

    //let user_info_prompt = r#""#;
    let system_prompt = format!(
        "{header_prompt}\n
        {tools_list}\n\n
        {rules_prompt}\n"
    );

    loop {
        terminal.draw(|f| ui::chat_ui(f, &app))?;

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
                            chat::Action::Chat,
                            message_args,
                        ));

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
    batch_ollama_response_inner(app, client, req).await
}

fn batch_ollama_response_inner<'a>(
    app: &'a mut App,
    client: Client,
    req: chat::ChatRequest<'a>,
) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
    Box::pin(async move {
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
            Ok(r) => {
                match r.message.content.action {
                    chat::Action::Chat => app.messages.push(r.message),
                    chat::Action::Tool(assistant_tool) => {
                        match assistant_tool {
                            chat::AssistantTool::WikiSearch => {
                                //HACK: fake it for now, until I figure out how to grab a web page and display it in a way the model understands
                                let tool_args = r.message.content.arguments.clone();
                                app.messages.push(r.message);

                                let search_term = match tool_args.get("query") {
                                    Some(v) => v.as_str(),
                                    None => todo!(),
                                };

                                let tool_response = match search_term {
                                    "American Crow" => {
                                        let r = args_builder! {
                                            "result" => include_str!("data/american_crow_wikipedia.md")
                                        };
                                        r
                                    }
                                    "Black Bear" => {
                                        let r = args_builder! {
                                            "result" => include_str!("data/black_bear_wikipedia.md")
                                        };
                                        r
                                    }
                                    _ => {
                                        let r = args_builder! {
                                            "result" => "Search failed to return any valid data"
                                        };
                                        r
                                    }
                                };

                                let tool_message = Message::from((
                                    chat::MessageRoles::Tool,
                                    Action::Tool(chat::AssistantTool::WikiSearch),
                                    tool_response,
                                ));
                                app.messages.push(tool_message);
                                //FIXME: model could recurse forever
                                batch_ollama_response(app, client.clone(), req).await?;
                            }
                            chat::AssistantTool::WebSearch => todo!(),
                            chat::AssistantTool::GetDateTime => todo!(),
                            chat::AssistantTool::GetDirectoryTree => todo!(),
                            chat::AssistantTool::GetFileContents => todo!(),
                            chat::AssistantTool::InvalidTool => todo!(),
                        }
                    }
                }
            }
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

        app.waiting = false;
        Ok(())
    })
}
