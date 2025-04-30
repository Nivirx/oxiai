use crossterm::terminal;
use ratatui::CompletedFrame;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, Event as CEvent, EventStream, KeyCode,
    MouseButton,
};

use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use ui::OxiTerminal;

use std::borrow::Cow;

use chat::{Action, Message};
use clap::Parser;
use futures_util::StreamExt;

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

pub struct Queues {
    pub tx_msg: mpsc::UnboundedSender<Msg>, // worker → UI   (already exists)
    pub rx_msg: mpsc::UnboundedReceiver<Msg>,

    pub tx_cmd: mpsc::UnboundedSender<Cmd>, // UI → worker   (NEW)
    pub rx_cmd: mpsc::UnboundedReceiver<Cmd>,
}

impl Queues {
    pub fn new() -> Self {
        let (tx_msg, rx_msg) = mpsc::unbounded_channel();
        let (tx_cmd, rx_cmd) = mpsc::unbounded_channel();
        Queues {
            tx_msg,
            rx_msg,
            tx_cmd,
            rx_cmd,
        }
    }
}

struct AppState {
    args: Args,
    queues: Queues,
    prompt: String,
    messages: Vec<Message>,
    waiting: bool,
    system_prompt: String,
}

impl AppState {
    const HEADER_PROMPT: &str = r#"SYSTEM: You are "OxiAI", A personal assistant with access to tools. You answer *only* via valid, UTF-8 JSON."#;

    const TOOLS_LIST: &str = include_str!("data/tools_list.json");

    const RULES_PROMPT: &str = r#"Rules:
1. Think silently, Never reveal your chain-of-thought.
2. To use a tool: {"action":"<tool>","arguments":{...}}
3. To reply directly: {"action":"chat","arguments":{"response":"..."}
4. If a question is vague, comparative, descriptive, or about ideas rather than specifics: use the web_search tool.
5. If a question clearly names a specific entity, place, or period of time: use the wiki_search tool.
6. Base claims strictly on provided data or tool results. If unsure, say so.
7. Check your output; If you reach four consecutive newlines: *stop*"#;

    pub fn default(args: Args) -> AppState {
        AppState {
            args,
            queues: Queues::new(),
            prompt: String::new(),
            messages: vec![],
            waiting: false,
            system_prompt: AppState::get_system_prompt(),
        }
    }

    pub fn get_system_prompt() -> String {
        format!(
            "{}\n{}\n\n{}\n",
            AppState::HEADER_PROMPT,
            AppState::TOOLS_LIST,
            AppState::RULES_PROMPT
        )
    }

    pub fn handle_http_done(
        &mut self,
        result: Result<String, reqwest::Error>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn handle_input(&mut self, ev: Event) -> anyhow::Result<Option<Cmd>> {
        match ev {
            Event::FocusGained => { /* do nothing */ }
            Event::FocusLost => { /* do nothing */ }
            Event::Key(key_event) => {
                match key_event.code {
                    KeyCode::Char(c) => self.prompt.push(c),
                    KeyCode::Backspace => {
                        let _ = self.prompt.pop();
                    }
                    KeyCode::Enter => {
                        //TODO: refactor to a parser function to take the contents of the app.prompt vec and do fancy stuff with it (like commands)
                        let message_args = args_builder! {
                            "response" => self.prompt.clone(),
                        };
                        self.prompt.clear();

                        self.messages.push(chat::Message::new(
                            chat::MessageRoles::User,
                            chat::Action::Chat,
                            message_args,
                        ));

                        let mut prompts = vec![chat::Prompt {
                            role: "system".to_string(),
                            content: self.system_prompt.clone(),
                        }];
                        prompts.extend(
                            self.messages
                                .iter()
                                .map(|msg| chat::Prompt::from(msg.clone())),
                        );

                        let req = chat::ChatRequest {
                            model: self.args.model.clone(),
                            stream: self.args.stream,
                            format: "json".to_string(),
                            stop: vec!["\n\n\n\n".to_string()],
                            options: Some(chat::ChatOptions {
                                temperature: Some(0.3),
                                top_p: Some(0.92),
                                top_k: Some(50),
                                repeat_penalty: Some(1.1),
                                seed: None,
                            }),
                            messages: prompts,
                        };

                        self.waiting = true;
                        return Ok(Some(Cmd::RunChat { req }));
                    }
                    _ => { /* ignore all other keys */ }
                }
            }
            Event::Mouse(mouse_event) => match mouse_event.kind {
                event::MouseEventKind::Up(MouseButton::Left) => {}
                _ => {}
            },
            Event::Paste(_) => { /* do nothing */ }
            Event::Resize(_, _) => { /* do nothing */ }
        }

        Ok(None)
    }
}

/// Cmds that can arrive in the command event queue
enum Cmd {
    RunChat { req: chat::ChatRequest },
    GetAddr,
    Quit,
}

/// Messages that can arrive in the UI loop
enum Msg {
    Input(CEvent),
    HttpDone(Result<String, reqwest::Error>),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // parse arguments from Clap
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            e.print().expect("Error writing clap error");
            std::process::exit(0);
        }
    };

    // UI Event Loop

    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(33));
    let mut terminal = OxiTerminal::setup();
    let mut state = AppState::default(args);

    'uiloop: loop {
        // first – non-blocking drain of all pending messages
        while let Ok(msg) = state.queues.rx_msg.try_recv() {
            match msg {
                Msg::Input(ev) => match ev.as_key_event() {
                    Some(ke) => {
                        if ke.code == KeyCode::Esc {
                            return terminal.term_cleanup();
                        } else {
                            if let Some(cmd) = state.handle_input(ev)? {
                                if state.queues.tx_cmd.send(cmd).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    None => {}
                },
                Msg::HttpDone(r) => state.handle_http_done(r)?,
            };
        }

        // block until either next tick or next user input
        tokio::select! {
            _ = ticker.tick() => { terminal.do_draw(&state); },

            maybe_ev = events.next() => {
                if let Some(Ok(ev)) = maybe_ev {
                    if state.queues.tx_msg.send(Msg::Input(ev)).is_err() { break 'uiloop }
                }
            }
        }
    }

    Ok(())
}

async fn run_workers(
    mut rx_cmd: mpsc::UnboundedReceiver<Cmd>,
    tx_msg: mpsc::UnboundedSender<Msg>,
    model: String,
) {
    while let Some(cmd) = rx_cmd.recv().await {
        match cmd {
            Cmd::RunChat { req } => {
                let tx_msg = tx_msg.clone();
                tokio::spawn(async move {
                    let res = ollama_call(req).await; // see next section
                    let _ = tx_msg.send(Msg::HttpDone(res));
                });
            }
            Cmd::GetAddr => {
                // --- Kick off an HTTP worker as a proof-of-concept ----
                let tx_msg = tx_msg.clone();
                tokio::spawn(async move {
                    let res: Result<String, reqwest::Error> = async {
                        let resp = reqwest::get("https://ifconfig.me/all").await?;
                        resp.text().await
                    }
                    .await;
                    let _ = tx_msg.send(Msg::HttpDone(res));
                });
            }
            Cmd::Quit => break,
        }
    }
}

async fn ollama_call(req: chat::ChatRequest) -> Result<String, reqwest::Error> {
    let client = reqwest::Client::new();
    client
        .post("http://localhost:11434/api/chat")
        .json(&req)
        .send()
        .await?
        .text()
        .await
}
