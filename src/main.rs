use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, Event as CEvent, EventStream, KeyCode,
    MouseButton,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use ratatui::{Frame, Terminal, backend::CrosstermBackend};

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

pub struct AppStateQueue {
    tx: UnboundedSender<Msg>,
    rx: UnboundedReceiver<Msg>,
}

impl From<(UnboundedSender<Msg>, UnboundedReceiver<Msg>)> for AppStateQueue {
    fn from(value: (UnboundedSender<Msg>, UnboundedReceiver<Msg>)) -> Self {
        AppStateQueue {
            tx: value.0,
            rx: value.1,
        }
    }
}

struct AppState {
    args: Args,
    event_queue: AppStateQueue,
    prompt: String,
    messages: Vec<Message>,
    waiting: bool,
    system_prompt: String,
}

impl AppState {
    const HEADER_PROMPT: &str = r#"SYSTEM: You are "OxiAI", a logical, personal assistant that answers *only* via valid, minified, UTF-8 JSON."#;

    const TOOLS_LIST: &str = include_str!("data/tools_list.json");

    const RULES_PROMPT: &str = r#"Rules:
1. Think silently, Never reveal your chain-of-thought.
2. To use a tool: {"action":"<tool>","arguments":{...}}
3. To reply directly: {"action":"chat","arguments":{"response":"..."}
4. If a question is vague, comparative, descriptive, or about ideas rather than specifics: use the web_search tool.
5. If a question clearly names a specific object, animal, person, place: use the wiki_search tool.
6. Base claims strictly on provided data or tool results. If unsure, say so.
7. Check your output; If you reach four consecutive newlines: *stop*"#;

    pub fn default(args: Args) -> AppState {
        AppState {
            args,
            event_queue: AppStateQueue::from(mpsc::unbounded_channel::<Msg>()),
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

    pub fn handle_input(&mut self, ev: Event) -> anyhow::Result<()> {
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
                            role: Cow::Borrowed("system"),
                            content: Cow::Borrowed(&self.system_prompt),
                        }];
                        prompts.extend(
                            self.messages
                                .iter()
                                .map(|msg| chat::Prompt::from(msg.clone())),
                        );

                        let req = chat::ChatRequest {
                            model: &self.args.model.clone(),
                            stream: self.args.stream,
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

                        self.waiting = true;
                        match self.args.stream {
                            true => {
                                todo!();
                            }
                            false => {
                                todo!();
                            }
                        }
                    }
                    _ => { /* ignore all other keys */ }
                }
            }
            Event::Mouse(mouse_event) => {
                match mouse_event.kind {
                    event::MouseEventKind::Up(MouseButton::Left) => {
                        // --- Kick off an HTTP worker as a proof-of-concept ----
                        let tx = self.event_queue.tx.clone();
                        tokio::spawn(async move {
                            let res: Result<String, reqwest::Error> = async {
                                let resp = reqwest::get("https://ifconfig.me/all").await?;
                                resp.text().await
                            }
                            .await;
                            let _ = tx.send(Msg::HttpDone(res));
                        });
                    }
                    _ => {}
                }
            }
            Event::Paste(_) => { /* do nothing */ }
            Event::Resize(_, _) => { /* do nothing */ }
        }

        Ok(())
    }
}

/// Messages that can arrive in the UI loop
enum Msg {
    Input(CEvent),
    HttpDone(Result<String, reqwest::Error>),
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

    // ---- UI LOOP ----------------------------------------------------------
    enable_raw_mode()?; // crossterm
    let mut stdout_handle = std::io::stdout();
    crossterm::execute!(stdout_handle, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout_handle);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(33));
    let mut state = AppState::default(args);

    'uiloop: loop {
        // first – non-blocking drain of all pending messages
        'drain_event_loop: while let Ok(msg) = state.event_queue.rx.try_recv() {
            match msg {
                Msg::Input(ev) => match ev.as_key_event() {
                    Some(ke) => {
                        if ke.code == KeyCode::Esc {
                            term_cleanup(&mut terminal)?;
                            return Ok(());
                        } else {
                            state.handle_input(ev)?
                        }
                    }
                    None => break 'drain_event_loop,
                },
                Msg::HttpDone(r) => state.handle_http_done(r)?,
            };
        }

        // draw a new frame
        terminal.draw(|f| ui::chat_ui(f, &state))?;

        // block until either next tick or next user input
        tokio::select! {
            _ = ticker.tick() => { /* redraw ui per tick rate */},

            maybe_ev = events.next() => {
                if let Some(Ok(ev)) = maybe_ev {
                    if state.event_queue.tx.send(Msg::Input(ev)).is_err() { break 'uiloop }
                }
            }
        }
    }

    Ok(())
}

fn term_cleanup<B: ratatui::backend::Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
