use std::{collections::HashMap, sync::Arc};

use rmcp::{
    handler::server::{
        router::{
            tool::{CallToolHandlerExt, ToolRoute, ToolRouteWithType, ToolRouter}, Router
        },
        tool::{schema_for_type, CallToolHandler, Parameters},
    }, model::{Extensions, Tool}, RoleServer, ServerHandler, Service
};

#[derive(Debug, Default)]
pub struct TestHandler<T: 'static = ()> {
    pub _marker: std::marker::PhantomData<fn(*const T)>,
}

impl<T: 'static> ServerHandler for TestHandler<T> {}
#[derive(Debug, schemars::JsonSchema, serde::Deserialize, serde::Serialize)]
pub struct Request {
    pub fields: HashMap<String, String>,
}

#[derive(Debug, schemars::JsonSchema, serde::Deserialize, serde::Serialize)]
pub struct Sum {
    pub a: i32,
    pub b: i32,
}

impl<T> TestHandler<T> {
    async fn async_method(&self, Parameters(Request { fields }): Parameters<Request>) {
        drop(fields)
    }
    fn sync_method(&self, Parameters(Request { fields }): Parameters<Request>) {
        drop(fields)
    }
}

fn sync_function(Parameters(Request { fields }): Parameters<Request>) {
    drop(fields)
}

// #[rmcp(tool(description = "async method", parameters = Request, name = "async_method"))]
//    ^
//    |_____ this is a macro will generates a function with the same name but return ToolRoute<TestHandler>
fn async_function<T>(
    _callee: &TestHandler<T>,
    Parameters(Request { fields }): Parameters<Request>,
) -> impl Future<Output = ()> + 'static {
    async move { drop(fields) }
}

fn async_function2<T>(
    _callee: &TestHandler<T>,
) -> impl Future<Output = ()> + 'static {
    async move {  }
}

fn assert_service<S: Service<RoleServer>>(service: S) {
    drop(service);
}

#[test]
fn test_tool_router() {
    let test_handler = TestHandler::<()>::default();
    fn tool(name: &'static str) -> Tool {
        Tool::new(name, name, schema_for_type::<Request>())
    }
    let tool_router = ToolRouter::<TestHandler<()>>::new()
        .with(tool("sync_method"), TestHandler::sync_method)
        // .with(tool("async_method"), TestHandler::async_method)
        .with(tool("sync_function"), sync_function);
        // .with(tool("async_function"), async_function::<TestHandler>);
    // assert_fn_once::<_, _>(async_function2);
    assert_handler::<TestHandler<()>, _, _,>(async_function);
    assert_handler::<TestHandler<()>, _, _,>(sync_function);
    // let router = Router::new(test_handler)
    //     .with_tool(
    //         TestHandler::sync_method
    //             .name("sync_method")
    //             .description("a sync method tool")
    //             .parameters::<Request>(),
    //     )
    //     .with_tool(
    //         (|Parameters(Sum { a, b }): Parameters<Sum>| (a + b).to_string())
    //             .name("add")
    //             .parameters::<Sum>(),
    //     )
    //     .with_tool(attr_generator_fn)
    //     .with_tools(tool_router);
    // assert_service(router);
}

fn assert_handler<S, H, A>(_handler: H) 
where H: CallToolHandler<S, A>
{

}

fn assert_fn_once<F, Fut>(f: F)
where F: FnOnce(&'_ TestHandler) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static
{
    
}