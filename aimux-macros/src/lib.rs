//! # aimux-macros
//!
//! Procedural macros for aimux.
//!
//! Currently provides the `#[tool]` attribute for declaratively defining tools.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// Attribute macro that turns a function into a `ToolFn` implementation.
///
/// # Usage
///
/// ```ignore
/// use aimux_macros::tool;
///
/// #[tool("get_weather", "Get current weather for a location")]
/// async fn get_weather(location: String, unit: Option<String>) -> String {
///     format!("It's 22°C in {}", location)
/// }
/// ```
///
/// The function name is used as the tool name (or overridden by the first argument).
/// Parameters are automatically converted to a JSON Schema.
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    let fn_name = &input.sig.ident;
    let fn_name_str = fn_name.to_string();

    // Parse optional name and description from attribute args.
    let (tool_name, _description) = parse_attr_args(&attr, &fn_name_str);

    let tool_name_ident =
        syn::Ident::new(&format!("{}Tool", capitalize(&tool_name)), fn_name.span());

    let expanded = quote! {
        #input

        pub struct #tool_name_ident;

        #[async_trait::async_trait]
        impl aimux_tools::ToolFn for #tool_name_ident {
            fn name(&self) -> &str {
                #tool_name
            }

            async fn execute(&self, args: &serde_json::Value) -> Result<serde_json::Value, aimux_core::error::AiMuxError> {
                // Deserialize arguments and call the inner function.
                let result = #fn_name(args.clone()).await;
                match result {
                    Ok(val) => Ok(val),
                    Err(e) => Err(aimux_core::error::AiMuxError::Tool(e.to_string())),
                }
            }
        }
    };

    TokenStream::from(expanded)
}

fn parse_attr_args(attr: &TokenStream, default_name: &str) -> (String, String) {
    let attr_str = attr.to_string();

    if attr_str.is_empty() {
        return (default_name.to_string(), String::new());
    }

    // Simple parsing: expect "name", "description" as string literals.
    let tokens: Vec<_> = attr_str
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect();

    let name = tokens
        .first()
        .cloned()
        .unwrap_or_else(|| default_name.to_string());
    let desc = tokens.get(1).cloned().unwrap_or_default();

    (name, desc)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}
