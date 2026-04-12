//! Shared parsing for `#[config_form(...)]` on struct fields (used by form and API codegen).

use crate::utils::{consume_meta_value, try_parse_lit_str};

/// All values parsed from field-level `#[config_form(...)]` attributes.
#[derive(Clone, Debug)]
pub struct ParsedFormAttrs {
    pub page: String,
    pub skip: bool,
    pub input_type: Option<String>,
    pub fieldset: Option<String>,
    pub help: String,
    pub label: Option<String>,
    pub hidden: bool,
    pub min: Option<String>,
    pub max: Option<String>,
    pub class: Option<String>,
    pub prim_type: Option<String>,
    pub save_as: Option<String>,
}

impl Default for ParsedFormAttrs {
    fn default() -> Self {
        Self {
            page: String::from("main"),
            skip: false,
            input_type: None,
            fieldset: None,
            help: String::new(),
            label: None,
            hidden: false,
            min: None,
            max: None,
            class: None,
            prim_type: None,
            save_as: None,
        }
    }
}

/// Converts a parsed expression (`min` / `max` attr value) to a string for HTML attributes.
pub fn expr_to_min_max_string(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            expr_to_min_max_string(&unary.expr).map(|s| format!("-{s}"))
        }
        syn::Expr::Lit(expr_lit) => match &expr_lit.lit {
            syn::Lit::Int(i) => i.base10_parse::<i64>().ok().map(|n| n.to_string()),
            syn::Lit::Float(f) => f.base10_parse::<f64>().ok().map(|n| n.to_string()),
            syn::Lit::Str(s) => Some(s.value()),
            _ => None,
        },
        _ => None,
    }
}

/// Parses nested meta from one `#[config_form(...)]` into `out` (merging with any prior state).
pub fn parse_config_form_attr_into(
    attr: &syn::Attribute,
    out: &mut ParsedFormAttrs,
) -> syn::Result<()> {
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("skip") {
            out.skip = true;
        } else if meta.path.is_ident("hidden") {
            out.hidden = true;
        } else if meta.path.is_ident("page") {
            if let Some(v) = try_parse_lit_str(&meta) {
                out.page = v;
            }
        } else if meta.path.is_ident("fieldset") {
            out.fieldset = try_parse_lit_str(&meta);
        } else if meta.path.is_ident("label") {
            out.label = try_parse_lit_str(&meta);
        } else if meta.path.is_ident("help") {
            if let Some(v) = try_parse_lit_str(&meta) {
                out.help = v;
            }
        } else if meta.path.is_ident("class") {
            out.class = try_parse_lit_str(&meta);
        } else if meta.path.is_ident("input_type") {
            out.input_type = try_parse_lit_str(&meta);
        } else if meta.path.is_ident("prim_type") {
            out.prim_type = try_parse_lit_str(&meta);
        } else if meta.path.is_ident("save_as") {
            out.save_as = try_parse_lit_str(&meta);
        } else if meta.path.is_ident("min") {
            if let Ok(expr) = meta.value().and_then(|v| v.parse::<syn::Expr>()) {
                out.min = expr_to_min_max_string(&expr);
            }
        } else if meta.path.is_ident("max") {
            if let Ok(expr) = meta.value().and_then(|v| v.parse::<syn::Expr>()) {
                out.max = expr_to_min_max_string(&expr);
            }
        } else {
            consume_meta_value(&meta);
        }
        Ok(())
    })
}
