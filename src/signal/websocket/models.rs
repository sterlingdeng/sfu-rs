use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum JsonRpcKind {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Serialize, Deserialize, Debug)]
enum Id {
    Num(i32),
    String(String),
}

// jsonrpc request message
#[derive(Serialize, Deserialize, Debug)]
struct Request {
    pub id: Id,
    // todo: try to turn this into an enum that can be switched
    pub method: String, // join, offer
    pub params: Map<String, Value>,
}

// jsonrpc response message
#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    pub id: Id,
    pub result: Map<String, Value>,
    pub error: Option<Value>,
}

// jsonrcp notification message
#[derive(Serialize, Deserialize, Debug)]
pub struct Notification {
    pub method: String,
    pub params: Map<String, Value>,
}
