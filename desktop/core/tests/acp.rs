//! The chat wire: ACP v1 NDJSON over ~/.fermix/acp.sock, behind Fermix's bridge
//! handshake. Shapes follow the engine's `Channels.Acp.Peer`.

use fermix_client::acp::{
    cancel, handshake_line, initialize, new_session, parse_ack, parse_line, prompt,
    reply_unsupported, AckError, Incoming, StopReason, ToolStatus, Update,
};
use serde_json::{json, Value};

fn decode(line: &str) -> Value {
    assert!(line.ends_with('\n'), "every frame is one line: {line:?}");
    assert_eq!(
        line.matches('\n').count(),
        1,
        "exactly one newline: {line:?}"
    );
    serde_json::from_str(line.trim_end()).unwrap()
}

#[test]
fn the_handshake_names_the_bridge_version_and_the_app() {
    assert_eq!(
        decode(&handshake_line("0.2.0")),
        json!({"fermix_bridge": 1, "app_version": "0.2.0", "env": {}})
    );
}

#[test]
fn an_ok_ack_opens_the_bridge_and_an_error_ack_carries_its_message() {
    assert_eq!(
        parse_ack(r#"{"fermix_bridge_ack":{"status":"ok"}}"#),
        Ok(())
    );
    assert_eq!(
        parse_ack(r#"{"fermix_bridge_ack":{"status":"error","message":"too many connections"}}"#),
        Err(AckError::Refused("too many connections".into()))
    );
    assert!(matches!(parse_ack("{}"), Err(AckError::Malformed(_))));
    assert!(matches!(parse_ack("not json"), Err(AckError::Malformed(_))));
}

#[test]
fn requests_are_json_rpc_with_the_params_the_agent_requires() {
    let init = decode(&initialize(1, "0.2.0"));
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["id"], 1);
    assert_eq!(init["method"], "initialize");
    assert_eq!(init["params"]["protocolVersion"], 1);
    assert_eq!(init["params"]["clientInfo"]["name"], "fermix-desktop");

    let session = decode(&new_session(2, "/home/someone"));
    assert_eq!(session["method"], "session/new");
    assert_eq!(
        session["params"],
        json!({"cwd": "/home/someone", "mcpServers": []})
    );

    let ask = decode(&prompt(3, "s1", "Hello <there>\nsecond line"));
    assert_eq!(ask["method"], "session/prompt");
    assert_eq!(
        ask["params"],
        json!({"sessionId": "s1", "prompt": [{"type": "text", "text": "Hello <there>\nsecond line"}]})
    );

    let stop = decode(&cancel("s1"));
    assert_eq!(stop["method"], "session/cancel");
    assert!(stop.get("id").is_none(), "cancel is a notification");
    assert_eq!(stop["params"], json!({"sessionId": "s1"}));
}

#[test]
#[should_panic(expected = "absolute")]
fn a_session_needs_an_absolute_workspace() {
    new_session(2, "relative/dir");
}

#[test]
fn streamed_text_and_tool_steps_decode_as_updates() {
    let chunk = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hi"}}}}"#;
    assert_eq!(
        parse_line(chunk).unwrap(),
        Incoming::Update {
            session_id: "s1".into(),
            update: Update::MessageChunk("Hi".into())
        }
    );

    let tool = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call","toolCallId":"t1","title":"web_search","kind":"fetch","status":"in_progress"}}}"#;
    assert_eq!(
        parse_line(tool).unwrap(),
        Incoming::Update {
            session_id: "s1".into(),
            update: Update::ToolCall {
                id: "t1".into(),
                title: "web_search".into(),
                status: ToolStatus::Running
            }
        }
    );

    let done = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"failed"}}}"#;
    assert_eq!(
        parse_line(done).unwrap(),
        Incoming::Update {
            session_id: "s1".into(),
            update: Update::ToolUpdate {
                id: "t1".into(),
                status: ToolStatus::Failed
            }
        }
    );
}

#[test]
fn an_update_this_app_does_not_draw_is_kept_by_name() {
    let plan = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"plan","entries":[]}}}"#;
    assert_eq!(
        parse_line(plan).unwrap(),
        Incoming::Update {
            session_id: "s1".into(),
            update: Update::Other("plan".into())
        }
    );
}

#[test]
fn responses_and_errors_carry_their_request_id() {
    assert_eq!(
        parse_line(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#).unwrap(),
        Incoming::Response {
            id: 3,
            result: json!({"stopReason": "end_turn"})
        }
    );
    assert_eq!(
        parse_line(
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32603,"message":"the Fermix turn failed"}}"#
        )
        .unwrap(),
        Incoming::Error {
            id: Some(3),
            code: -32603,
            message: "the Fermix turn failed".into()
        }
    );
    assert_eq!(
        parse_line(
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}"#
        )
        .unwrap(),
        Incoming::Error {
            id: None,
            code: -32700,
            message: "parse error".into()
        }
    );
}

#[test]
fn a_prompt_ends_with_a_stop_reason() {
    assert_eq!(
        StopReason::from_result(&json!({"stopReason": "end_turn"})),
        Some(StopReason::EndTurn)
    );
    assert_eq!(
        StopReason::from_result(&json!({"stopReason": "cancelled"})),
        Some(StopReason::Cancelled)
    );
    assert_eq!(
        StopReason::from_result(&json!({"stopReason": "max_tokens"})),
        Some(StopReason::Other("max_tokens".into()))
    );
    assert_eq!(StopReason::from_result(&json!({})), None);
}

#[test]
fn a_request_from_the_agent_is_answered_so_it_never_waits() {
    let asked = r#"{"jsonrpc":"2.0","id":"r9","method":"session/request_permission","params":{}}"#;
    let Incoming::Request { id, method } = parse_line(asked).unwrap() else {
        panic!("a request with an id is a request");
    };
    assert_eq!(method, "session/request_permission");
    let reply = decode(&reply_unsupported(&id));
    assert_eq!(reply["id"], "r9");
    assert_eq!(reply["error"]["code"], -32601);
}

#[test]
fn a_line_that_is_not_json_rpc_is_refused_with_a_reason() {
    assert!(parse_line("garbage").is_err());
    assert!(parse_line(r#"{"jsonrpc":"2.0"}"#).is_err());
    assert!(
        parse_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{}}}"#)
            .is_err()
    );
}
