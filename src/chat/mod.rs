use serde::{Deserialize, Serialize};
use serde::de::{self, Deserializer};

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};

#[derive(Deserialize, Debug)]
pub struct StreamChunk {
    pub message: StreamMessage,
}

#[derive(Deserialize, Debug)]
pub struct StreamMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Prompt<'a> {
    pub role: Cow<'a, str>,
    pub content: Cow<'a, str>,
}

impl<'a> From<Message> for Prompt<'a> {
    fn from(message: Message) -> Self {
        Prompt {
            role: Cow::Owned(message.role),
            content: Cow::Owned(message.content.to_string()),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub repeat_penalty: Option<f32>,
    pub seed: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<Prompt<'a>>,
    pub stream: bool,
    pub format: &'a str,
    pub stop: Vec<&'a str>,
    pub options: Option<ChatOptions>,
}

pub enum MessageRoles {
    System = 0,
    Tool,
    User,
    Assistant,
    Other,
}

impl Display for MessageRoles {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let role: &str = match self {
            MessageRoles::System => "system",
            MessageRoles::Tool => "tool",
            MessageRoles::User => "user",
            MessageRoles::Assistant => "assistant",
            //HACK: Handle this cleanly, if the model hallucinates we crash :^)
            MessageRoles::Other => todo!(),
        };

        write!(f, "{}", role)
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Message {
    pub role: String,
    #[serde(deserialize_with = "Message::de_content")]
    pub content: ActionPacket,
}

impl Message {
    pub fn new(role: MessageRoles, action: Action, arguments: HashMap<String, String>) -> Self {
        Self {
            role: role.to_string(),
            content: ActionPacket::new(action, arguments),
        }
    }

    // Custom deserializer function
    fn de_content<'de, D>(deserializer: D) -> Result<ActionPacket, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        serde_json::from_str(&s).map_err(de::Error::custom)
    }
}

impl From<(MessageRoles, Action, HashMap<String, String>)> for Message {
    fn from((role, action, arguments): (MessageRoles, Action, HashMap<String, String>)) -> Self {
        Message::new(role, action, arguments)
    }
}

impl Display for Message {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.content)
    }
}

#[derive(Serialize, Deserialize, PartialEq)]
pub enum AssistantTool {
    WikiSearch,
    WebSearch,
    GetDateTime,
    GetDirectoryTree,
    GetFileContents,
    InvalidTool,
}

impl Display for AssistantTool {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let res = match self {
            AssistantTool::WikiSearch => "wiki_search",
            AssistantTool::WebSearch => "web_search",
            AssistantTool::GetDateTime => "get_datetime",
            AssistantTool::GetDirectoryTree => "get_dirtree",
            AssistantTool::GetFileContents => "get_file",
            //HACK: Handle this cleanly, if the model hallucinates we crash :^)
            AssistantTool::InvalidTool => todo!(),
        };
        write!(f, "{}", res)
    }
}

#[derive(Serialize, Deserialize, PartialEq)]
pub enum Action {
    ChatMessage,
    Tool(AssistantTool),
}

impl Display for Action {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Action::ChatMessage => write!(f, "{}", "chat"),
            Action::Tool(tool_name) => write!(f, "{tool_name}"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct ActionPacket {
    pub action: String,
    pub arguments: HashMap<String, String>,
}

impl ActionPacket {
    pub fn new(action: Action, arguments: HashMap<String, String>) -> Self {
        Self {
            action: action.to_string(),
            arguments,
        }
    }
}

impl Display for ActionPacket {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match serde_json::to_string(&self.arguments) {
            Ok(arguments_json) => write!(f, "{} {}", self.action, arguments_json),
            Err(_) => write!(f, "{} {{}}", self.action), // fallback to empty JSON if error
        }
    }
}

#[derive(Deserialize)]
pub struct ChatResponse {
    pub model: String,
    pub created_at: String,
    pub message: Message,
    pub done: bool,
    pub done_reason: Option<String>,
    pub total_duration: Option<u64>,
    pub eval_count: Option<u64>,
    pub eval_duration: Option<u64>,
    pub prompt_eval_count: Option<u64>,
    pub prompt_eval_duration: Option<u64>,
}

#[macro_export]
macro_rules! args_builder {
    ( $( $key:expr => $value:expr ),* $(,)? ) => {{
        let mut map = ::std::collections::HashMap::new();
        $(
            map.insert($key.into(), $value.into());
        )*
        map
    }};
}