mod connection;
mod error;
mod msg;

use tree_sitter::{Parser, Point, Node};
use std::error::Error;
use connection::Connection;
use msg::{Message, Response};
use serde::{Deserialize, Serialize};


#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub line: usize,
    pub character: usize,
}


#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseAstInRangeParams {
    pub language: String,
    pub cursor_position: Position,
    pub code: String,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AstBlock {
    pub ast_result: String,
    pub start_point: Position,
    pub end_point: Position,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseAstInRangeResponse {
    pub ast_result: String,
    pub parent: Option<AstBlock>,
    pub start_point: Position,
    pub end_point: Position,
}

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    // Note that  we must have our logging only write out to stderr.
    eprintln!("<ast-rs> starting generic LSP server");

    // Create the transport. Includes the stdio (stdin and stdout) versions but this could
    // also be implemented to use sockets or HTTP.
    let (connection, io_threads) = Connection::stdio();

    main_loop(connection)?;
    io_threads.join()?;

    // Shut down gracefully.
    eprintln!("<ast-rs> shutting down server");
    Ok(())
}

fn format_node(node: Node) -> Option<AstBlock> {
    let start_point = node.start_position();
    let end_point = node.end_position();
    let result = AstBlock {
        ast_result: node.to_sexp(),
        start_point: Position {
            line: start_point.row,
            character: start_point.column,
        },
        end_point: Position {
            line: end_point.row,
            character: end_point.column
        }
    };
    return Some(result);
}

fn main_loop(
    connection: Connection,
) -> Result<(), Box<dyn Error + Sync + Send>> {

    let mut parser = Parser::new();

    eprintln!("<ast-rs> starting example main loop");
    for msg in &connection.receiver {
        eprintln!("<ast-rs> got msg: {msg:?}");
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                // eprintln!("got request: {req:?}");
                if req.method == "ParseAstInRange" {
                    let params: ParseAstInRangeParams = serde_json::from_value(req.params)?;
                    let language = params.language;

                    // TODO: 优化下写法
                    if language == "python" {
                        parser.set_language(tree_sitter_python::language()).unwrap();
                    } else if language == "c" {
                        parser.set_language(tree_sitter_c::language()).unwrap();
                    } else if language == "javascript" {
                        parser.set_language(tree_sitter_javascript::language()).unwrap();
                    } else if language == "typescript" {
                        parser.set_language(tree_sitter_typescript::language_typescript()).unwrap();
                    } else if language == "golang" {
                        parser.set_language(tree_sitter_go::language()).unwrap();
                    } else if language == "java" {
                        parser.set_language(tree_sitter_java::language()).unwrap();
                    } else if language == "cpp" {
                        parser.set_language(tree_sitter_cpp::language()).unwrap();
                    } else if language == "csharp" {
                        parser.set_language(tree_sitter_c_sharp::language()).unwrap();
                    } else if language == "rust" {
                        parser.set_language(tree_sitter_rust::language()).unwrap();
                    } else {
                        eprintln!("<ast-rs> invalid language");
                        let resp = Response::new_err(req.id, 1, "invalid language".to_string());
                        connection.sender.send(Message::Response(resp))?;
                        continue;
                    }

                    if params.code == "" {
                        let resp = Response::new_err(req.id, 1, "code is empty".to_string());
                        connection.sender.send(Message::Response(resp))?;
                        continue;
                    }

                    let tree = parser.parse(params.code, None).unwrap();
                    let root_node = tree.root_node();

                    let cursor_point = Point {
                        row: usize::try_from(params.cursor_position.line).unwrap(),
                        column: usize::try_from(params.cursor_position.character).unwrap()
                    };
                
                    match root_node.named_descendant_for_point_range(cursor_point, cursor_point) {
                        None => {
                            eprintln!("<ast-rs> ast parse None");
                            let resp = Response::new_err(req.id, 1, "ast parse fail".to_string());
                            connection.sender.send(Message::Response(resp))?;
                            continue;
                        },
                        Some(node) => {
                            // 生成结果
                            let start_point = node.start_position();
                            let end_point = node.end_position();
                            let result = Some(ParseAstInRangeResponse {
                                ast_result: node.to_sexp(),
                                parent: match node.parent() {
                                    Some(n) => format_node(n),
                                    None => None
                                },
                                start_point: Position {
                                    line: start_point.row,
                                    character: start_point.column,
                                },
                                end_point: Position {
                                    line: end_point.row,
                                    character: end_point.column
                                }
                            });
                            let result = serde_json::to_value(&result).unwrap();
                            let resp = Response { id: req.id, result: Some(result), error: None };
                            connection.sender.send(Message::Response(resp))?;
                            continue;
                        }
                    };
                } else {
                    eprintln!("<ast-rs> got invalid method: {}", req.method);
                    let resp = Response::new_err(req.id, 1, "invalid method".to_string());
                    connection.sender.send(Message::Response(resp))?;
                };
            }
            Message::Response(resp) => {
                eprintln!("<ast-rs> got response: {resp:?}");
            }
            Message::Notification(not) => {
                eprintln!("<ast-rs> got notification: {not:?}");
            }
        }
    }
    Ok(())
}
