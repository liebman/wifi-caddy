//! Config form codegen for `WifiCaddyConfig`: generates the entire config HTML page
//! as a single `&'static str` at compile time.

use crate::utils::{
    consume_meta_value, escape_html, escape_js_str, humanize_label, page_name_to_js_id,
    try_parse_lit_str,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

const CONFIG_PAGE_CSS: &str = include_str!("config_page.css");
const CONFIG_PAGE_TAB_SCRIPT: &str = include_str!("config_page_script.js");

// ---------------------------------------------------------------------------
// Type-info lookup (replaces ConfigValue form constants)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveKind {
    String,
    Int,
    Float,
}

struct TypeInfo {
    input_type: &'static str,
    is_float: bool,
    save_kind: SaveKind,
}

fn known_type_info_by_name(name: &str) -> Option<TypeInfo> {
    match name {
        "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64" | "usize" | "isize" => {
            Some(TypeInfo {
                input_type: "number",
                is_float: false,
                save_kind: SaveKind::Int,
            })
        }
        "f32" | "f64" => Some(TypeInfo {
            input_type: "number",
            is_float: true,
            save_kind: SaveKind::Float,
        }),
        "String" => Some(TypeInfo {
            input_type: "text",
            is_float: false,
            save_kind: SaveKind::String,
        }),
        _ => None,
    }
}

fn known_type_info(ty: &syn::Type) -> Option<TypeInfo> {
    if let syn::Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            return known_type_info_by_name(&seg.ident.to_string());
        }
    }
    None
}

fn parse_save_as(s: &str) -> Option<SaveKind> {
    match s {
        "string" => Some(SaveKind::String),
        "int" => Some(SaveKind::Int),
        "float" => Some(SaveKind::Float),
        _ => None,
    }
}

fn infer_save_kind(input_type: &str) -> SaveKind {
    match input_type {
        "number" | "range" => SaveKind::Int,
        _ => SaveKind::String,
    }
}

struct ResolvedInfo {
    input_type: String,
    is_float: bool,
    save_kind: SaveKind,
}

fn resolve_field_info(f: &FormField) -> Result<ResolvedInfo, String> {
    let base = f
        .prim_type
        .as_deref()
        .and_then(known_type_info_by_name)
        .or_else(|| known_type_info(&f.field_type));

    if let Some(info) = base {
        return Ok(ResolvedInfo {
            input_type: f
                .input_type
                .clone()
                .unwrap_or_else(|| info.input_type.to_string()),
            is_float: info.is_float,
            save_kind: f
                .save_as
                .as_deref()
                .and_then(parse_save_as)
                .unwrap_or(info.save_kind),
        });
    }

    let input_type = f.input_type.clone().ok_or_else(|| {
        format!(
            "field `{}` has unrecognized type; add #[config_form(input_type = \"...\")] or #[config_form(prim_type = \"...\")]",
            f.name
        )
    })?;
    let save_kind = f
        .save_as
        .as_deref()
        .and_then(parse_save_as)
        .unwrap_or_else(|| infer_save_kind(&input_type));
    let is_float = save_kind == SaveKind::Float;

    Ok(ResolvedInfo {
        input_type,
        is_float,
        save_kind,
    })
}

// ---------------------------------------------------------------------------
// Intermediate representation
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FormField {
    name: String,
    page: String,
    fieldset: Option<String>,
    hidden: bool,
    label: String,
    help: String,
    class: Option<String>,
    field_type: syn::Type,
    min: Option<String>,
    max: Option<String>,
    input_type: Option<String>,
    prim_type: Option<String>,
    save_as: Option<String>,
}

struct UiAttrs {
    page_heading: String,
    title: String,
    subtitle: String,
    nav_left: String,
    nav_right: String,
    extra_css: String,
    default_group: Option<String>,
}

// ---------------------------------------------------------------------------
// Attribute helpers
// ---------------------------------------------------------------------------

/// Converts a parsed expression (min/max attr value) to a string for HTML attributes.
fn expr_to_min_max_string(expr: &syn::Expr) -> Option<String> {
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

// ---------------------------------------------------------------------------
// Parse #[config_ui(...)] from struct-level attributes
// ---------------------------------------------------------------------------

fn parse_ui_attrs(attrs: &[syn::Attribute]) -> UiAttrs {
    let mut page_heading = String::from("Configuration");
    let mut title = String::from("Configuration");
    let mut subtitle = String::new();
    let mut nav_left = String::from("<span>Configuration</span>");
    let mut nav_right = String::from("<span></span>");
    let mut extra_css = String::new();
    let mut default_group: Option<String> = None;

    for attr in attrs {
        if attr.path().is_ident("config_ui") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("page_heading") {
                    if let Some(v) = try_parse_lit_str(&meta) {
                        page_heading = v;
                    }
                } else if meta.path.is_ident("title") {
                    if let Some(v) = try_parse_lit_str(&meta) {
                        title = v;
                    }
                } else if meta.path.is_ident("subtitle") {
                    if let Some(v) = try_parse_lit_str(&meta) {
                        subtitle = v;
                    }
                } else if meta.path.is_ident("nav_left") {
                    if let Some(v) = try_parse_lit_str(&meta) {
                        nav_left = v;
                    }
                } else if meta.path.is_ident("nav_right") {
                    if let Some(v) = try_parse_lit_str(&meta) {
                        nav_right = v;
                    }
                } else if meta.path.is_ident("extra_css") {
                    if let Some(v) = try_parse_lit_str(&meta) {
                        extra_css = v;
                    }
                } else if meta.path.is_ident("default_group") {
                    if let Some(v) = try_parse_lit_str(&meta) {
                        default_group = Some(v);
                    }
                } else {
                    consume_meta_value(&meta);
                }
                Ok(())
            });
        }
    }

    UiAttrs {
        page_heading,
        title,
        subtitle,
        nav_left,
        nav_right,
        extra_css,
        default_group,
    }
}

// ---------------------------------------------------------------------------
// Phase 1 – parse `#[config_form(...)]` on each struct field
// ---------------------------------------------------------------------------

fn parse_form_fields(data: &syn::DataStruct) -> Vec<FormField> {
    let mut form_fields = Vec::new();

    for field in &data.fields {
        let ident = field.ident.as_ref().expect("unnamed fields not supported");
        let field_name = ident.to_string();

        let mut has_config_form = false;
        let mut skip = false;
        let mut page = String::from("main");
        let mut fieldset: Option<String> = None;
        let mut hidden = false;
        let mut label = humanize_label(&field_name);
        let mut help = String::new();
        let mut class: Option<String> = None;
        let mut input_type: Option<String> = None;
        let mut min: Option<String> = None;
        let mut max: Option<String> = None;
        let mut prim_type: Option<String> = None;
        let mut save_as: Option<String> = None;

        for attr in &field.attrs {
            if attr.path().is_ident("config_form") {
                has_config_form = true;
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("skip") {
                        skip = true;
                    } else if meta.path.is_ident("hidden") {
                        hidden = true;
                    } else if meta.path.is_ident("page") {
                        if let Some(v) = try_parse_lit_str(&meta) {
                            page = v;
                        }
                    } else if meta.path.is_ident("fieldset") {
                        fieldset = try_parse_lit_str(&meta);
                    } else if meta.path.is_ident("label") {
                        if let Some(v) = try_parse_lit_str(&meta) {
                            label = v;
                        }
                    } else if meta.path.is_ident("help") {
                        if let Some(v) = try_parse_lit_str(&meta) {
                            help = v;
                        }
                    } else if meta.path.is_ident("class") {
                        class = try_parse_lit_str(&meta);
                    } else if meta.path.is_ident("input_type") {
                        input_type = try_parse_lit_str(&meta);
                    } else if meta.path.is_ident("prim_type") {
                        prim_type = try_parse_lit_str(&meta);
                    } else if meta.path.is_ident("save_as") {
                        save_as = try_parse_lit_str(&meta);
                    } else if meta.path.is_ident("min") {
                        if let Ok(expr) = meta.value().and_then(|v| v.parse::<syn::Expr>()) {
                            min = expr_to_min_max_string(&expr);
                        }
                    } else if meta.path.is_ident("max") {
                        if let Ok(expr) = meta.value().and_then(|v| v.parse::<syn::Expr>()) {
                            max = expr_to_min_max_string(&expr);
                        }
                    } else {
                        consume_meta_value(&meta);
                    }
                    Ok(())
                });
            }
        }

        if !has_config_form || skip {
            continue;
        }

        form_fields.push(FormField {
            name: field_name,
            page,
            fieldset,
            hidden,
            label,
            help,
            class,
            field_type: field.ty.clone(),
            min,
            max,
            input_type,
            prim_type,
            save_as,
        });
    }

    form_fields
}

// ---------------------------------------------------------------------------
// Phase 2 – HTML generation (fully static string per page)
// ---------------------------------------------------------------------------

fn gen_visible_input_html(f: &FormField, info: &ResolvedInfo) -> String {
    let fname = &f.name;
    let label_esc = escape_html(&f.label);
    let field_class = f
        .class
        .as_ref()
        .map(|c| format!(" {}", escape_html(c)))
        .unwrap_or_default();
    let wrapper_class = format!("config-form-group config-form-field-{}", f.name);
    let min_attr = f
        .min
        .as_ref()
        .map(|s| format!(r#" min="{}""#, escape_html(s)))
        .unwrap_or_default();
    let max_attr = f
        .max
        .as_ref()
        .map(|s| format!(r#" max="{}""#, escape_html(s)))
        .unwrap_or_default();
    let help_esc = escape_html(&f.help);
    let step_attr = if info.is_float { r#" step="any""# } else { "" };
    let req_attr = match f.input_type.as_deref() {
        Some("password") => "",
        _ => " required",
    };

    format!(
        r#"<div class="{wrapper_class}"><label for="{fname}" class="config-form-label" style="color:var(--config-form-label-color,#555)">{label_esc}</label><input type="{input_type}"{step_attr}{req_attr} id="{fname}" name="{fname}" class="config-form-input config-form-input-{fname}{field_class}" style="border:var(--config-form-input-border,2px solid #ddd)"{min_attr}{max_attr}><div class="config-form-help" style="color:var(--config-form-help-color,#888)">{help_esc}</div></div>"#,
        input_type = info.input_type,
    )
}

fn gen_html_string(fields: &[FormField]) -> Result<String, String> {
    let mut html = String::from("<div class=\"config-form\">");

    let mut current_fieldset: Option<&str> = None;
    for f in fields {
        let fieldset_changed = current_fieldset != f.fieldset.as_deref();
        if fieldset_changed {
            if current_fieldset.is_some() {
                html.push_str("</fieldset>");
            }
            current_fieldset = f.fieldset.as_deref();
            if let Some(legend) = current_fieldset {
                html.push_str(&format!(
                    "<fieldset class=\"config-form-fieldset\" style=\"border:var(--config-form-fieldset-border,2px solid #e0e0e0)\"><legend class=\"config-form-legend\" style=\"color:var(--config-form-legend-color,#667eea)\">{}</legend>",
                    escape_html(legend)
                ));
            }
        }

        if f.hidden {
            let name_esc = escape_html(&f.name);
            html.push_str(&format!(
                r#"<input type="hidden" id="{0}" name="{0}">"#,
                name_esc
            ));
        } else {
            let info = resolve_field_info(f)?;
            html.push_str(&gen_visible_input_html(f, &info));
        }
    }

    if current_fieldset.is_some() {
        html.push_str("</fieldset>");
    }
    html.push_str("</div>");

    Ok(html)
}

// ---------------------------------------------------------------------------
// Phase 3 – JS generation (fully static string per page)
// ---------------------------------------------------------------------------

fn gen_js_string(page_name: &str, fields: &[FormField]) -> Result<String, String> {
    let mut js = String::new();
    let page_js_id = page_name_to_js_id(page_name);
    let page_esc = escape_js_str(page_name);
    let form_id = format!("configForm-{}", page_js_id);
    let form_id_esc = escape_js_str(&form_id);
    let load_fn = format!("loadConfig_{}", page_js_id);
    let save_fn = format!("saveConfig_{}", page_js_id);

    js.push_str(&format!(
        "const CONFIG_PAGE_{0}=\"{1}\";const CONFIG_URL_{0}=\"/config-group/\"+CONFIG_PAGE_{0};async function {2}(){{const response=await fetch(CONFIG_URL_{0});if(!response.ok)throw new Error(\"HTTP \"+response.status);const data=await response.json();",
        page_js_id, page_esc, load_fn
    ));

    for f in fields {
        let name_js = escape_js_str(&f.name);
        let fname = &f.name;
        js.push_str(&format!(
            "var el=document.getElementById(\"{0}\");if(el)el.value=data[\"{1}\"]!==undefined?String(data[\"{1}\"]):\"\";",
            fname, name_js
        ));
    }

    js.push_str(&format!(
        "}} async function {0}(){{const form=document.getElementById(\"{1}\");if(!form)return;const formData=new FormData(form);const data={{}};",
        save_fn, form_id_esc
    ));

    for f in fields {
        let name_js = escape_js_str(&f.name);
        let fname = &f.name;
        let info = resolve_field_info(f)?;
        match info.save_kind {
            SaveKind::String => js.push_str(&format!(
                "data[\"{}\"]=formData.get(\"{}\")??\"\";",
                name_js, fname
            )),
            SaveKind::Int => js.push_str(&format!(
                "data[\"{}\"]=parseInt(formData.get(\"{}\"),10);",
                name_js, fname
            )),
            SaveKind::Float => js.push_str(&format!(
                "data[\"{}\"]=parseFloat(formData.get(\"{}\"));",
                name_js, fname
            )),
        }
    }

    js.push_str(&format!(
        "const response=await fetch(CONFIG_URL_{0}+\"?set=\"+encodeURIComponent(JSON.stringify(data)),{{method:\"GET\"}});if(!response.ok)throw new Error(await response.text()||\"HTTP \"+response.status);}};window.{1}=window.{1}||{1};window.{2}=window.{2}||{2};",
        page_js_id, load_fn, save_fn
    ));

    Ok(js)
}

// ---------------------------------------------------------------------------
// Phase 4 – full page assembly
// ---------------------------------------------------------------------------

fn gen_full_page(ui: &UiAttrs, pages: &[(String, Vec<FormField>)]) -> Result<String, String> {
    let resolved_default = ui
        .default_group
        .as_deref()
        .or_else(|| pages.first().map(|(name, _)| name.as_str()))
        .unwrap_or("main");
    let default_id = page_name_to_js_id(resolved_default);
    let show_tabs = pages.len() > 1;

    let mut page = String::with_capacity(8192);

    // Head
    page.push_str("<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1.0\"><title>");
    page.push_str(&escape_html(&ui.title));
    page.push_str("</title><style>");
    page.push_str(CONFIG_PAGE_CSS);
    page.push_str(&ui.extra_css);
    page.push_str("</style></head><body><div class=\"container\"><header><h1>");
    page.push_str(&escape_html(&ui.page_heading));
    page.push_str("</h1><p>");
    page.push_str(&escape_html(&ui.subtitle));
    page.push_str("</p></header><div class=\"nav\">");
    page.push_str(&ui.nav_left);
    page.push_str(&ui.nav_right);
    page.push_str("</div><div class=\"content\"><div id=\"message\" class=\"message\"></div>");

    // Tab bar (only for multi-page)
    if show_tabs {
        page.push_str("<div class=\"config-tabs\">");
        for (page_name, _) in pages {
            let id = page_name_to_js_id(page_name);
            let active_class = if id == default_id {
                "config-tab active"
            } else {
                "config-tab"
            };
            page.push_str(&format!(
                r#"<button type="button" class="{}" data-page="{}">{}</button>"#,
                active_class,
                id,
                escape_html(page_name),
            ));
        }
        page.push_str("</div>");
    }

    // Per-page panels
    for (page_name, fields) in pages {
        let id = page_name_to_js_id(page_name);
        let display = if id == default_id {
            ""
        } else {
            " style=\"display:none\""
        };
        page.push_str(&format!(
            r#"<div class="config-tab-panel" id="panel-{}"{}>"#,
            id, display
        ));
        page.push_str(&format!(
            r#"<div class="config-loading-overlay" id="loading-{}"><span class="loading loading-overlay"></span>Loading...</div>"#,
            id
        ));
        page.push_str(&format!(r#"<form id="configForm-{}">"#, id));

        let form_html = gen_html_string(fields)?;
        page.push_str(&form_html);

        page.push_str(r#"<div class="button-group"><button type="button" class="reloadBtn">Reload</button><button type="submit">Save Configuration</button></div></form>"#);
        page.push_str("</div>");
    }

    // Close content + container divs
    page.push_str("</div></div>");

    // Per-page JS
    page.push_str("<script>");
    for (page_name, fields) in pages {
        let js = gen_js_string(page_name, fields)?;
        page.push_str(&js);
    }
    page.push_str("</script>");

    // Tab script + default page activation
    page.push_str("<script>");
    page.push_str(CONFIG_PAGE_TAB_SCRIPT);
    page.push_str(&format!(
        "var defaultPage=\"{}\";window.addEventListener('load', function() {{ switchTab(defaultPage); }});",
        escape_js_str(&default_id)
    ));
    page.push_str("</script></body></html>");

    Ok(page)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Builds the form half of `WifiCaddyConfig`: generates the entire config HTML page
/// as a single `&'static str` at compile time.
///
/// Reads struct-level `#[config_ui(...)]` attributes for page chrome (title, heading, etc.)
/// and field-level `#[config_form(...)]` for form fields.
///
/// Emits: `const CONFIG_PAGE: &str` (one complete HTML document) and a `ConfigFormGen` impl.
pub fn derive_config_form_impl(input: &DeriveInput) -> TokenStream {
    let name = &input.ident;

    let syn::Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(input, "ConfigForm only supports structs")
            .to_compile_error();
    };

    let ui = parse_ui_attrs(&input.attrs);
    let form_fields = parse_form_fields(data);

    let mut pages: Vec<(String, Vec<FormField>)> = Vec::new();
    for f in form_fields {
        let page_name = f.page.clone();
        if let Some(entry) = pages.iter_mut().find(|(name, _)| *name == page_name) {
            entry.1.push(f);
        } else {
            pages.push((page_name, vec![f]));
        }
    }

    // Validate: no fieldset spans multiple pages
    let mut fieldset_pages: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for (page, fields) in &pages {
        for f in fields {
            if let Some(ref fs) = f.fieldset {
                if let Some(existing_page) = fieldset_pages.get(fs) {
                    if existing_page != page {
                        return syn::Error::new_spanned(
                            input,
                            format!(
                                "fieldset \"{}\" appears on pages \"{}\" and \"{}\"; groups cannot be split across pages",
                                fs, existing_page, page
                            ),
                        )
                        .to_compile_error();
                    }
                } else {
                    fieldset_pages.insert(fs.clone(), page.clone());
                }
            }
        }
    }

    let full_page = match gen_full_page(&ui, &pages) {
        Ok(s) => s,
        Err(msg) => return syn::Error::new_spanned(input, msg).to_compile_error(),
    };

    let page_lit = syn::LitStr::new(&full_page, proc_macro2::Span::call_site());

    quote! {
        impl #name {
            const CONFIG_PAGE: &str = #page_lit;
        }

        impl wifi_caddy::config_storage::ConfigFormGen for #name {
            fn config_page() -> &'static str {
                Self::CONFIG_PAGE
            }
        }
    }
}
