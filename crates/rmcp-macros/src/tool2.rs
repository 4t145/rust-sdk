use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{Expr, ExprLit, Ident, ItemFn, Lit, LitBool, LitStr};
#[derive(FromMeta, Default)]
#[darling(default)]
pub struct ToolAttribute {
    /// The name of the tool
    pub name: Option<Expr>,
    pub description: Option<Expr>,
    /// A JSON Schema object defining the expected parameters for the tool
    pub input_schema: Option<Expr>,
    /// Optional additional tool information.
    pub annotations: Option<ToolAnnotationsAttribute>,
}

pub struct ResolvedToolAttribute {
    pub name: Expr,
    pub description: Expr,
    pub input_schema: Expr,
    pub annotations: Expr,
}

impl ResolvedToolAttribute {
    pub fn to_fn(self, fn_ident: Ident) -> syn::Result<ItemFn> {
        let Self {
            name,
            description,
            input_schema,
            annotations,
        } = self;
        let tokens = quote! {
            pub fn #fn_ident() -> rmcp::model::Tool {
                rmcp::model::Tool {
                    name: #name,
                    description: #description,
                    input_schema: #input_schema,
                    annotations: #annotations,
                }
            };
        };
        syn::parse2::<ItemFn>(tokens)
    }
}

#[derive(FromMeta)]
pub struct ToolAnnotationsAttribute {
    /// A human-readable title for the tool.
    pub title: Expr,

    /// If true, the tool does not modify its environment.
    ///
    /// Default: false
    pub read_only_hint: Expr,

    /// If true, the tool may perform destructive updates to its environment.
    /// If false, the tool performs only additive updates.
    ///
    /// (This property is meaningful only when `readOnlyHint == false`)
    ///
    /// Default: true
    /// A human-readable description of the tool's purpose.
    pub destructive_hint: Expr,

    /// If true, calling the tool repeatedly with the same arguments
    /// will have no additional effect on the its environment.
    ///
    /// (This property is meaningful only when `readOnlyHint == false`)
    ///
    /// Default: false.
    pub idempotent_hint: Expr,

    /// If true, this tool may interact with an "open world" of external
    /// entities. If false, the tool's domain of interaction is closed.
    /// For example, the world of a web search tool is open, whereas that
    /// of a memory tool is not.
    ///
    /// Default: true
    pub open_world_hint: Expr,
}

fn none_expr() -> Expr {
    syn::parse2::<Expr>(quote! { None }).unwrap()
}

impl Default for ToolAnnotationsAttribute {
    fn default() -> Self {
        ToolAnnotationsAttribute {
            title: none_expr(),
            read_only_hint: none_expr(),
            destructive_hint: none_expr(),
            idempotent_hint: none_expr(),
            open_world_hint: none_expr(),
        }
    }
}

// extract doc line from attribute
fn extract_doc_line(attr: &syn::Attribute) -> Option<String> {
    if !attr.path().is_ident("doc") {
        return None;
    }

    let syn::Meta::NameValue(name_value) = &attr.meta else {
        return None;
    };

    let syn::Expr::Lit(expr_lit) = &name_value.value else {
        return None;
    };

    let syn::Lit::Str(lit_str) = &expr_lit.lit else {
        return None;
    };

    let content = lit_str.value().trim().to_string();

    (!content.is_empty()).then_some(content)
}

pub fn tool(attr: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let attr_args = NestedMeta::parse_meta_list(attr)?;
    let attribute = ToolAttribute::from_list(&attr_args)?;
    let fn_item = syn::parse2::<ItemFn>(input.clone())?;
    let fn_ident = fn_item.sig.ident;

    let tool_attr_fn_ident = format_ident!("{}_tool_attr", fn_ident);
    let input_schema_expr = if let Some(input_schema) = attribute.input_schema {
        input_schema
    } else {
        // try to find some parameters wrapper in the function
        let params_ty = fn_item.sig.inputs.iter().find_map(|input| {
            if let syn::FnArg::Typed(pat_type) = input {
                if let syn::Type::Path(type_path) = &*pat_type.ty {
                    if type_path.path.is_ident("Parameters") {
                        return Some(pat_type.ty.clone());
                    }
                }
            }
            None
        });
        if let Some(params_ty) = params_ty {
            // if found, use the Parameters schema
            syn::parse2::<Expr>(quote! {
                rmcp::handler::server::tool::cached_schema_for_type::<#params_ty>()
            })?
        } else {
            // if not found, use the default EmptyObject schema
            syn::parse2::<Expr>(quote! {
                rmcp::handler::server::tool::cached_schema_for_type::<rmcp::model::EmptyObject>()
            })?
        }
    };

    let annotations_expr = if let Some(annotations) = attribute.annotations {
        let ToolAnnotationsAttribute {
            title,
            read_only_hint,
            destructive_hint,
            idempotent_hint,
            open_world_hint,
        } = annotations;
        let token_stream = quote! {
            rmcp::handler::ToolAnnotations {
                title: #title,
                read_only_hint: #read_only_hint,
                destructive_hint: #destructive_hint,
                idempotent_hint: #idempotent_hint,
                open_world_hint: #open_world_hint,
            }
        };
        syn::parse2::<Expr>(token_stream)?
    } else {
        none_expr()
    };
    let resolved_tool_attr = ResolvedToolAttribute {
        name: attribute.name.unwrap_or_else(|| {
            Expr::Lit(ExprLit {
                attrs: vec![],
                lit: Lit::Str(LitStr::new(&fn_ident.to_string(), fn_ident.span())),
            })
        }),
        description: attribute.description.unwrap_or_else(|| {
            let doc_content = fn_item
                .attrs
                .iter()
                .filter_map(extract_doc_line)
                .collect::<Vec<_>>()
                .join("\n");
            Expr::Lit(ExprLit {
                attrs: vec![],
                lit: Lit::Str(LitStr::new(&doc_content.to_string(), Span::call_site())),
            })
        }),
        input_schema: input_schema_expr,
        annotations: annotations_expr,
    };
    let tool_fn = resolved_tool_attr.to_fn(tool_attr_fn_ident)?;
    Ok(quote! {
        #tool_fn
        #input
    })
}
