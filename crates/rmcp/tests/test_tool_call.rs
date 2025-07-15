mod common;
pub use common::calculator::Calculator;
use rmcp::{ErrorData, ServiceExt, model::CallToolRequestParam, object};

#[tokio::test]
async fn test_tool_call() -> anyhow::Result<()> {
    use rmcp::{
        ServerHandler,
        handler::server::{router::tool::ToolRouter, tool::Parameters},
        model::{ServerCapabilities, ServerInfo},
        tool, tool_handler, tool_router,
    };

    let calculator = Calculator::new();
    let (server_transport, client_transport) = tokio::io::duplex(1024);
    tokio::spawn(async move {
        let server = calculator.serve(server_transport).await?;
        server.waiting().await?;
        anyhow::Ok(())
    });
    let client = ().serve(client_transport).await?;
    let tool_call_result = client
        .call_tool(CallToolRequestParam {
            name: "sum".into(),
            arguments: Some(object! {{
                "a": 1,
                "b": 2,
            }}),
        })
        .await?;
    println!("Tool call result: {:?}", tool_call_result);

    let tool_call_error = client
        .call_tool(CallToolRequestParam {
            name: "add".into(),
            arguments: Some(object! {{
                "a": 1,
                "b": 2,
            }}),
        })
        .await
        .expect_err("we don't have such tool");
    match tool_call_error {
        rmcp::ServiceError::McpError(ErrorData {
            code: rmcp::model::ErrorCode::INVALID_PARAMS,
            message,
            ..
        }) => {
            println!("Tool not found: {}", message);
        }
        _ => panic!("Expected ToolNotFound error, got {:?}", tool_call_error),
    }
    let tool_call_error = client
        .call_tool(CallToolRequestParam {
            name: "error_call".into(),
            arguments: None,
        })
        .await
        .expect_err("we don't have such tool");
    match tool_call_error {
        rmcp::ServiceError::McpError(ErrorData {
            code: rmcp::model::ErrorCode::INTERNAL_ERROR,
            message,
            ..
        }) => {
            println!("error call message: {}", message);
        }
        _ => panic!("Expected ToolNotFound error, got {:?}", tool_call_error),
    }
    client.cancel().await?;
    Ok(())
}
