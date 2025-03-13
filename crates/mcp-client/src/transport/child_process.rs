use futures::{Sink, Stream};
use mcp_core::schema::{ClientJsonRpcMessage, ServerJsonRpcMessage};

pub fn child_process(
    mut child: tokio::process::Child,
) -> std::io::Result<
    (tokio::process::Child, impl Sink<ClientJsonRpcMessage, Error = std::io::Error> + Stream<Item = ServerJsonRpcMessage>),
> {
    if child.stdin.is_none() {
        return Err(std::io::Error::other("std in was taken"));
    }
    if child.stdout.is_none() {
        return Err(std::io::Error::other("std out was taken"));
    }
    let child_stdin = child.stdin.take().expect("already checked");
    let child_stdout = child.stdout.take().expect("already checked");
    Ok((child, mcp_core::transport::io::async_rw(
        child_stdout,
        child_stdin,
    )))
}
