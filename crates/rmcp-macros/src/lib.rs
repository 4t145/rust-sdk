#[allow(unused_imports)]
use proc_macro::TokenStream;

mod tool;
mod tool2;
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, input: TokenStream) -> TokenStream {
    tool::tool(attr.into(), input.into())
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
#[proc_macro_attribute]
pub fn tool2(attr: TokenStream, input: TokenStream) -> TokenStream {
    tool2::tool(attr.into(), input.into())
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
