//! The pure parts of a tool call: stripping the injected arguments before the customer's handler
//! runs, the mint-back text and its structured mirror, the `get_more_tools` call and the shapes of
//! the `error` of an event. `call/tests.rs` checks them against `tests/vectors/call.json` and
//! `tests/vectors/call_cases.json`. [`run`] puts them together into the pipeline that
//! `Tracked::call_tool` runs.
use rmcp::model::{CallToolResult, ContentBlock, ErrorCode, ErrorData, JsonObject};
use serde_json::{Map, Value};

use crate::constants::{
    AGENT_ID_PARAM, GET_MORE_TOOLS_NAME, GET_MORE_TOOLS_RESULT_TEXT, MCP_SESSION_KEY,
    SESSION_ID_PARAM, report_missing_log_line,
};
use crate::inject::{CONTEXT_PARAM, ToolEntry};
use crate::wire;

pub(crate) mod run;
#[cfg(test)]
mod tests;

/// The `platform` of an event's `error`, on every error the SDK records.
pub(crate) const PLATFORM: &str = "rust";

/// The message of an error result without text.
const UNKNOWN_ERROR: &str = "Unknown error";

/// The message of the `error` of a cancelled call.
const CANCELLED_MESSAGE: &str = "tool call cancelled before it completed";

/// The `type` of the `error` of a cancelled call.
const CANCELLED_TYPE: &str = "cancelled";

/// The arguments the customer's handler receives: `arguments` without the parameters the SDK
/// injected into the tool. With a registry `entry` those are exactly the parameters it recorded.
/// Without one (a tool the listings never showed) they are `session_id`, `agent_id` and
/// `context`, and for `get_more_tools`, whose `context` is its own parameter, only `session_id`
/// and `agent_id`. The other arguments keep their order.
pub(crate) fn strip_arguments(
    arguments: &JsonObject,
    entry: Option<&ToolEntry>,
    tool_name: &str,
) -> JsonObject {
    let names: &[&str] = match entry {
        Some(entry) => &entry.injected,
        None if tool_name == GET_MORE_TOOLS_NAME => &[SESSION_ID_PARAM, AGENT_ID_PARAM],
        None => &[SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM],
    };
    arguments
        .iter()
        .filter(|(key, _)| !names.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Puts the mint-back `text` first in the content of `result`, for any result, error results
/// (`is_error`) too. An rmcp result always has a list for `content`.
pub(crate) fn prepend_mint_back(result: &mut CallToolResult, text: String) {
    result.content.insert(0, ContentBlock::text(text));
}

/// Mirrors the structured `mint` into `structured` as `mcp_session`, when `structured` is an
/// object without its own `mcp_session` key: the customer's value wins, and an array, a string, a
/// number, `null` or no value at all stays as it is.
///
/// The new object holds `mcp_session` and then the keys of `structured`, in the order of
/// [`wire::own_entries`]: the array-index keys ascending, then `mcp_session`, then the other keys
/// in their order.
pub(crate) fn mirror_mint_back(structured: &mut Option<Value>, mint: Map<String, Value>) {
    let Some(Value::Object(body)) = structured else {
        return;
    };
    if body.contains_key(MCP_SESSION_KEY) {
        return;
    }
    let mut spread = Map::with_capacity(body.len() + 1);
    spread.insert(MCP_SESSION_KEY.to_owned(), Value::Object(mint));
    spread.append(body);
    let order: Vec<String> = wire::own_entries(&spread)
        .map(|(key, _)| key.clone())
        .collect();
    for key in order {
        if let Some(value) = spread.remove(&key) {
            body.insert(key, value);
        }
    }
}

/// The `error` of an event whose tool returned the error result `result`: the text blocks joined
/// with a space and trimmed ([`wire::trim`]), or `Unknown error` when that is empty. The error has
/// no `type` key. The `platform` is `rust`.
pub(crate) fn result_error(result: &CallToolResult) -> Map<String, Value> {
    let texts: Vec<&str> = result
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    let joined = texts.join(" ");
    let message = match wire::trim(&joined) {
        "" => UNKNOWN_ERROR,
        trimmed => trimmed,
    };
    let mut error = Map::new();
    error.insert("message".to_owned(), Value::String(message.to_owned()));
    error.insert("platform".to_owned(), Value::String(PLATFORM.to_owned()));
    error
}

/// The `error` of an event whose tool returned the JSON-RPC error `error`: its message, the
/// [`code_name`] as `type`, and the `rust` platform.
pub(crate) fn error_data_error(error: &ErrorData) -> Map<String, Value> {
    let mut shape = Map::new();
    shape.insert(
        "message".to_owned(),
        Value::String(error.message.to_string()),
    );
    shape.insert("type".to_owned(), Value::String(code_name(error.code)));
    shape.insert("platform".to_owned(), Value::String(PLATFORM.to_owned()));
    shape
}

/// The `error` of a call whose future was dropped before it completed: the message
/// [`CANCELLED_MESSAGE`], the type [`CANCELLED_TYPE`] and the `rust` platform, in the key order of
/// [`error_data_error`].
pub(crate) fn cancelled_error() -> Map<String, Value> {
    let mut shape = Map::new();
    shape.insert(
        "message".to_owned(),
        Value::String(CANCELLED_MESSAGE.to_owned()),
    );
    shape.insert("type".to_owned(), Value::String(CANCELLED_TYPE.to_owned()));
    shape.insert("platform".to_owned(), Value::String(PLATFORM.to_owned()));
    shape
}

/// The name of a JSON-RPC error code, the `type` of an `error`: `ParseError`,
/// `InvalidRequest`, `MethodNotFound`, `InvalidParams`, `InternalError` and `ResourceNotFound`
/// for rmcp's codes, `JsonRpcError(<code>)` for any other.
pub(crate) fn code_name(code: ErrorCode) -> String {
    match code {
        ErrorCode::PARSE_ERROR => "ParseError".to_owned(),
        ErrorCode::INVALID_REQUEST => "InvalidRequest".to_owned(),
        ErrorCode::METHOD_NOT_FOUND => "MethodNotFound".to_owned(),
        ErrorCode::INVALID_PARAMS => "InvalidParams".to_owned(),
        ErrorCode::INTERNAL_ERROR => "InternalError".to_owned(),
        ErrorCode::RESOURCE_NOT_FOUND => "ResourceNotFound".to_owned(),
        ErrorCode(other) => format!("JsonRpcError({other})"),
    }
}

/// The answer to a `get_more_tools` call and its log line. The line carries the length of the
/// `context` argument: the UTF-16 length of a string, the length of an array, and 0 for anything
/// else, also for an object with a `length` key.
pub(crate) fn report_missing(context: Option<&Value>) -> (CallToolResult, String) {
    let length = match context {
        Some(Value::String(text)) => wire::utf16_len(text),
        Some(Value::Array(items)) => items.len(),
        _ => 0,
    };
    let mut result = CallToolResult::success(vec![ContentBlock::text(GET_MORE_TOOLS_RESULT_TEXT)]);
    // The result has no `isError` key; rmcp's `success` writes `false`.
    result.is_error = None;
    (result, report_missing_log_line(length))
}
