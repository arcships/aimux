//! # aimux-macros
//!
//! Procedural macros for aimux.
//!
//! Currently provides the `#[tool]` attribute for declaratively defining tools.

use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, GenericArgument, ItemFn, PathArguments, ReturnType, Type, parse_macro_input};

/// Attribute macro that turns an async JSON tool function into a `ToolFn` implementation.
///
/// # Usage
///
/// ```ignore
/// use aimux_macros::tool;
/// use serde_json::Value;
///
/// #[tool("get_weather", "Get current weather for a location")]
/// async fn get_weather(args: Value) -> Result<Value, String> {
///     Ok(format!("It's 22°C in {}", args).into())
/// }
/// ```
///
/// The function name is used as the tool name (or overridden by the first argument).
/// The function must be `async fn(serde_json::Value) -> Result<serde_json::Value, E>`.
/// The optional second attribute argument is used as the generated `FunctionTool` description.
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    if let Err(error) = validate_tool_signature(&input) {
        return TokenStream::from(error.into_compile_error());
    }

    let fn_name = &input.sig.ident;
    let fn_name_str = fn_name.to_string();

    // Parse optional name and description from attribute args.
    let (tool_name, description) = parse_attr_args(&attr, &fn_name_str);

    let tool_name_ident =
        syn::Ident::new(&format!("{}Tool", capitalize(&tool_name)), fn_name.span());

    let expanded = quote! {
        #input

        pub struct #tool_name_ident;

        impl #tool_name_ident {
            pub fn definition(&self) -> aimux_core::tool::FunctionTool {
                aimux_core::tool::FunctionTool::new(#tool_name, serde_json::json!({}))
                    .with_description(#description)
            }
        }

        #[async_trait::async_trait]
        impl aimux_tools::ToolFn for #tool_name_ident {
            fn name(&self) -> &str {
                #tool_name
            }

            async fn execute(&self, args: &serde_json::Value) -> Result<serde_json::Value, aimux_core::error::AiMuxError> {
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

fn validate_tool_signature(function: &ItemFn) -> syn::Result<()> {
    let signature = &function.sig;

    if signature.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &signature.fn_token,
            "#[tool] requires an `async fn(serde_json::Value) -> Result<serde_json::Value, E>` function",
        ));
    }

    if signature.inputs.len() != 1 {
        return Err(syn::Error::new_spanned(
            &signature.inputs,
            "#[tool] requires exactly one `serde_json::Value` argument",
        ));
    }

    let argument = signature.inputs.first().expect("length checked above");
    let FnArg::Typed(argument) = argument else {
        return Err(syn::Error::new_spanned(
            argument,
            "#[tool] requires exactly one `serde_json::Value` argument",
        ));
    };

    if !is_value_type(&argument.ty) {
        return Err(syn::Error::new_spanned(
            &argument.ty,
            "#[tool] argument must be `serde_json::Value` (or imported `Value`)",
        ));
    }

    let ReturnType::Type(_, return_type) = &signature.output else {
        return Err(syn::Error::new_spanned(
            &signature.ident,
            "#[tool] return type must be `Result<serde_json::Value, E>`",
        ));
    };

    let Some(arguments) = result_type_arguments(return_type) else {
        return Err(syn::Error::new_spanned(
            return_type,
            "#[tool] return type must be `Result<serde_json::Value, E>`",
        ));
    };

    if !is_value_type(arguments.0) {
        return Err(syn::Error::new_spanned(
            arguments.0,
            "#[tool] success type must be `serde_json::Value` (or imported `Value`)",
        ));
    }

    Ok(())
}

fn is_value_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    type_path.qself.is_none()
        && type_path.path.segments.last().is_some_and(|segment| {
            segment.ident == "Value" && matches!(segment.arguments, PathArguments::None)
        })
}

fn result_type_arguments(ty: &Type) -> Option<(&Type, &Type)> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let mut types = arguments.args.iter().filter_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    });
    let ok = types.next()?;
    let error = types.next()?;
    if types.next().is_some() {
        return None;
    }
    Some((ok, error))
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
