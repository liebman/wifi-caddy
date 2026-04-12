//! Group API and notify codegen for `WifiCaddyConfig`: JSON get/set, `ConfigServer` + update channel, statics.

use crate::field_attrs::{parse_config_form_attr_into, ParsedFormAttrs};
use crate::utils::{consume_meta_value, to_pascal_case, try_parse_lit_int, try_parse_lit_str};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::DeriveInput;

// ---------------------------------------------------------------------------
// Intermediate representation
// ---------------------------------------------------------------------------

/// Logical field for ConfigApi: name, type, page, notify variant, and whether to redact/skip-if-empty.
#[derive(Clone)]
struct ApiField {
    ident: syn::Ident,
    ty: syn::Type,
    page: String,
    is_password: bool,
    /// From config_store(notify = "Wifi") or notify_group = "wifi" (PascalCase); variant for ConfigChange.
    notify: Option<String>,
}

/// All values parsed from struct-level attributes on the annotated config struct.
struct StructAttrs {
    /// `storage_magic = 0x...` from `#[config_server]`.
    storage_magic: Option<u32>,
    /// `storage_version = N` from `#[config_server]`.
    storage_version: Option<u32>,
    /// `cap = N` from `#[config_notify]`.
    notify_cap: Option<usize>,
}

// ---------------------------------------------------------------------------
// Phase 1 – parse struct-level attributes
// ---------------------------------------------------------------------------

fn parse_struct_attrs(attrs: &[syn::Attribute]) -> StructAttrs {
    let mut storage_magic: Option<u32> = None;
    let mut storage_version: Option<u32> = None;
    let mut notify_cap: Option<usize> = None;

    for attr in attrs {
        if attr.path().is_ident("config_server") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("storage_magic") {
                    storage_magic = try_parse_lit_int(&meta);
                } else if meta.path.is_ident("storage_version") {
                    storage_version = try_parse_lit_int(&meta);
                } else {
                    consume_meta_value(&meta);
                }
                Ok(())
            });
        } else if attr.path().is_ident("config_notify") {
            if let syn::Meta::List(_) = &attr.meta {
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("cap") {
                        notify_cap = try_parse_lit_int(&meta);
                    } else {
                        consume_meta_value(&meta);
                    }
                    Ok(())
                });
            }
        }
    }

    StructAttrs {
        storage_magic,
        storage_version,
        notify_cap,
    }
}

// ---------------------------------------------------------------------------
// Phase 2 – collect ApiFields from struct fields
// ---------------------------------------------------------------------------

fn collect_api_fields(fields: &syn::Fields) -> Vec<ApiField> {
    let mut api_fields = Vec::new();

    for field in fields {
        let field_ident = field.ident.as_ref().expect("unnamed fields not supported");

        let mut has_config_form = false;
        let mut form = ParsedFormAttrs::default();
        let mut notify: Option<String> = None;

        // from config_form: input_type (password → redact in GET), skip, page
        for attr in &field.attrs {
            if attr.path().is_ident("config_form") {
                has_config_form = true;
                let _ = parse_config_form_attr_into(attr, &mut form);
            }
        }

        // from config_store: notify / notify_group → ConfigChange variant name
        for attr in &field.attrs {
            if attr.path().is_ident("config_store") {
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("notify") {
                        notify = try_parse_lit_str(&meta);
                    } else if meta.path.is_ident("notify_group") {
                        // Backward compat: convert to PascalCase variant name
                        if let Ok(syn::Expr::Lit(expr_lit)) =
                            meta.value().and_then(|v| v.parse::<syn::Expr>())
                        {
                            if let syn::Lit::Str(s) = expr_lit.lit {
                                notify = Some(to_pascal_case(&s.value()));
                            }
                        }
                    } else {
                        consume_meta_value(&meta);
                    }
                    Ok(())
                });
            }
        }

        if !has_config_form || form.skip {
            continue;
        }

        let input_type = form.input_type.unwrap_or_else(|| String::from("text"));

        api_fields.push(ApiField {
            ident: field_ident.clone(),
            ty: field.ty.clone(),
            page: form.page,
            is_password: input_type == "password",
            notify,
        });
    }

    api_fields
}

// ---------------------------------------------------------------------------
// Phase 3 – ConfigChange enum
// ---------------------------------------------------------------------------

/// Collect distinct notify variant names and emit the ConfigChange enum.
/// Fields without an explicit `notify = "..."` are assigned the catchall `Changed` variant.
fn gen_config_change_enum(pages: &[(String, Vec<ApiField>)]) -> TokenStream {
    let mut variant_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut has_untagged = false;
    for (_, fields) in pages {
        for f in fields {
            if let Some(ref v) = f.notify {
                variant_names.insert(v.clone());
            } else {
                has_untagged = true;
            }
        }
    }
    if has_untagged {
        variant_names.insert("Changed".to_string());
    }
    let variant_idents: Vec<syn::Ident> = variant_names
        .iter()
        .map(|s| syn::Ident::new(s, proc_macro2::Span::call_site()))
        .collect();

    let variants = variant_idents.iter().map(|v| quote! { #v });
    quote! {
        #[derive(enumset::EnumSetType)]
        #[enumset(repr = "u64")]
        pub enum ConfigChange {
            #(#variants),*
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 4 – per-page DTOs and ConfigApi match arms
// ---------------------------------------------------------------------------

/// Returns (dto_structs, get_group_json arms, set_group_json arms).
fn gen_dto_and_group_arms(
    name: &syn::Ident,
    pages: &[(String, Vec<ApiField>)],
) -> (Vec<TokenStream>, Vec<TokenStream>, Vec<TokenStream>) {
    let mut dto_structs = Vec::new();
    let mut get_arms = Vec::new();
    let mut set_arms = Vec::new();

    for (page_name, fields) in pages {
        let page_pascal = to_pascal_case(page_name);
        let page_ident = syn::Ident::new(&page_pascal, proc_macro2::Span::call_site());
        let dto_name = format_ident!("{}{}PageDto", name, page_ident);
        let page_lit = syn::LitStr::new(page_name, proc_macro2::Span::call_site());

        // DTO struct: one public field per config field, serialisable to/from JSON
        let dto_fields = fields.iter().map(|f| {
            let i = &f.ident;
            let t = &f.ty;
            quote! { pub #i: #t }
        });
        let dto_doc = format!(
            "Generated DTO for config page \"{}\" of `{}`.",
            page_name, name
        );
        dto_structs.push(quote! {
            #[doc = #dto_doc]
            #[derive(serde::Serialize, serde::Deserialize)]
            pub struct #dto_name {
                #(#dto_fields),*
            }
        });

        // GET: password fields return Default::default() (redacted); others clone the value
        let get_dto_fields = fields.iter().map(|f| {
            let i = &f.ident;
            if f.is_password {
                quote! { #i: Default::default() }
            } else {
                quote! { #i: self.#i.clone() }
            }
        });
        get_arms.push(quote! {
            #page_lit => {
                let dto = #dto_name { #(#get_dto_fields),* };
                let len = serde_json_core::to_slice(&dto, buf)
                    .map_err(|_| wifi_caddy::config_storage::ConfigError::InvalidData)?;
                Ok(len)
            }
        });

        // SET: compare before applying each field; accumulate changed ConfigChange variants
        let set_compare_apply: Vec<TokenStream> = fields
            .iter()
            .map(|f| {
                let i = &f.ident;
                let setter = format_ident!("set_{}", i);
                let field_ty = &f.ty;
                let variant_name = f.notify.as_deref().unwrap_or("Changed");
                let variant_ident = syn::Ident::new(variant_name, proc_macro2::Span::call_site());
                let insert_line = quote! { changed.insert(ConfigChange::#variant_ident); };

                if f.is_password {
                    // Skip applying if the form sent an empty password (keep existing value)
                    quote! {
                        if dto.#i != <#field_ty as Default>::default() {
                            if self.#i != dto.#i {
                                #insert_line
                                self.#setter(dto.#i.clone());
                            }
                        }
                    }
                } else {
                    quote! {
                        if self.#i != dto.#i {
                            #insert_line
                            self.#setter(dto.#i.clone());
                        }
                    }
                }
            })
            .collect();

        set_arms.push(quote! {
            #page_lit => {
                let (dto, _) = serde_json_core::from_str::<#dto_name>(json)
                    .map_err(|_| wifi_caddy::config_storage::ConfigError::InvalidData)?;
                let mut changed = enumset::EnumSet::<ConfigChange>::new();
                #(#set_compare_apply)*
                Ok(changed)
            }
        });
    }

    (dto_structs, get_arms, set_arms)
}

// ---------------------------------------------------------------------------
// Phase 5 – set_field arms (single key=value from HTTP query string)
// ---------------------------------------------------------------------------

fn gen_set_field_arms(pages: &[(String, Vec<ApiField>)]) -> Vec<TokenStream> {
    pages
        .iter()
        .flat_map(|(_, fields)| fields.iter())
        .map(|f| {
            let key_str = f.ident.to_string();
            let key_lit = syn::LitStr::new(&key_str, proc_macro2::Span::call_site());
            let i = &f.ident;
            let setter = format_ident!("set_{}", i);
            let field_ty = &f.ty;

            let variant_name = f.notify.as_deref().unwrap_or("Changed");
            let vid = syn::Ident::new(variant_name, proc_macro2::Span::call_site());
            let parse_and_apply = quote! {
                if let Ok(parsed) = value.parse::<#field_ty>() {
                    if self.#i != parsed {
                        self.#setter(parsed);
                        let mut changed = enumset::EnumSet::<ConfigChange>::new();
                        changed.insert(ConfigChange::#vid);
                        Ok(Some(changed))
                    } else {
                        Ok(Some(enumset::EnumSet::new()))
                    }
                } else {
                    Err(wifi_caddy::config_storage::ConfigError::InvalidData)
                }
            };

            if f.is_password {
                // Empty value → keep existing; non-empty → parse and apply
                quote! {
                    #key_lit => {
                        if value.is_empty() {
                            Ok(Some(enumset::EnumSet::new()))
                        } else {
                            #parse_and_apply
                        }
                    }
                }
            } else {
                quote! {
                    #key_lit => { #parse_and_apply }
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Phase 6 – notify channel (Channel + helper fns)
// ---------------------------------------------------------------------------

fn gen_notify_channel(attrs: &StructAttrs, num_pages: usize) -> TokenStream {
    let cap_val = attrs.notify_cap.unwrap_or(num_pages);
    let cap_lit = proc_macro2::Literal::usize_unsuffixed(cap_val);

    quote! {
        type ConfigUpdateChannel = embassy_sync::channel::Channel<
            embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
            enumset::EnumSet<ConfigChange>,
            #cap_lit,
        >;
        pub type ConfigUpdateReceiver = &'static ConfigUpdateChannel;
        static CONFIG_UPDATE_CHANNEL: static_cell::StaticCell<ConfigUpdateChannel> =
            static_cell::StaticCell::new();
    }
}

// ---------------------------------------------------------------------------
// Phase 7 – ConfigServer trait impl (always emitted; #[config_server] only sets storage defaults)
// ---------------------------------------------------------------------------

fn gen_config_server_impl(
    name: &syn::Ident,
    attrs: &StructAttrs,
    init_notify_body: &TokenStream,
) -> TokenStream {
    let storage_magic_val = attrs.storage_magic.unwrap_or(0x4255_aa42);
    let storage_version_val = attrs.storage_version.unwrap_or(1);
    let storage_magic_lit = proc_macro2::Literal::u32_unsuffixed(storage_magic_val);
    let storage_version_lit = proc_macro2::Literal::u32_unsuffixed(storage_version_val);

    quote! {
        impl wifi_caddy::config_storage::ConfigServer for #name {
            type UpdateReceiver = ConfigUpdateReceiver;

            fn storage_params() -> wifi_caddy::ConfigStorageParams {
                wifi_caddy::ConfigStorageParams {
                    magic: #storage_magic_lit,
                    format_version: #storage_version_lit,
                }
            }

            fn init_notify() -> (
                Self::UpdateReceiver,
                embassy_sync::channel::DynamicSender<
                    'static,
                    <Self as wifi_caddy::config_storage::ConfigApi>::ChangedSet,
                >,
            ) {
                #init_notify_body
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Builds the group API, notify channel machinery, and `ConfigServer` impl for `WifiCaddyConfig`.
///
/// Struct-level overrides (optional on the struct): `#[config_server(storage_magic, storage_version)]`
/// for flash params (defaults apply if omitted), and `#[config_notify(cap = N)]` for channel capacity
/// (default: number of config pages). Omitting these attributes does not disable codegen.
/// Field-level: from `#[config_form]` we use `skip` and `input_type` (password → redacted in GET);
/// from `#[config_store]`, `notify = "Wifi"` or `notify_group = "wifi"` add a `ConfigChange` variant.
///
/// Emits: per-page DTOs (e.g. `AppConfigMainPageDto` for struct `AppConfig` and page `main`) for JSON, `ConfigChange` enum, `ConfigApi` impl;
/// channel types; `impl ConfigServer` with storage params and `init_notify`. Opt-out is not supported.
///
/// All generated code references only `wifi_caddy::*` — no platform-specific types.
/// Platform crates (e.g. `esp-wifi-caddy`) use the `ConfigServer` trait to access
/// storage params and the notify channel via `init_notify`.
pub fn derive_config_api_impl(input: &DeriveInput) -> TokenStream {
    let name = &input.ident;

    let syn::Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(input, "ConfigApi only supports structs")
            .to_compile_error();
    };

    let attrs = parse_struct_attrs(&input.attrs);
    let api_fields = collect_api_fields(&data.fields);

    // Group fields by page name, preserving declaration order
    let mut pages: Vec<(String, Vec<ApiField>)> = Vec::new();
    for f in api_fields {
        let page_name = f.page.clone();
        if let Some(entry) = pages.iter_mut().find(|(name, _)| *name == page_name) {
            entry.1.push(f);
        } else {
            pages.push((page_name, vec![f]));
        }
    }

    let config_change_enum = gen_config_change_enum(&pages);
    let (dto_structs, get_arms, set_arms) = gen_dto_and_group_arms(name, &pages);
    let set_field_arms = gen_set_field_arms(&pages);
    let notify_channel_block = gen_notify_channel(&attrs, pages.len());

    let init_notify_body = quote! {
        let ch = CONFIG_UPDATE_CHANNEL.init(ConfigUpdateChannel::new());
        let sender = embassy_sync::channel::DynamicSender::from(ch.sender());
        (ch, sender)
    };

    let config_server_impl = gen_config_server_impl(name, &attrs, &init_notify_body);

    let default_err = quote! { _ => Err(wifi_caddy::config_storage::ConfigError::InvalidData) };

    quote! {
        #(#dto_structs)*

        #config_change_enum

        impl wifi_caddy::config_storage::ConfigApi for #name {
            type Error = wifi_caddy::config_storage::ConfigError;
            type ChangedSet = enumset::EnumSet<ConfigChange>;

            fn get_group_json(&self, group: &str, buf: &mut [u8]) -> Result<usize, Self::Error> {
                match group {
                    #(#get_arms)*
                    #default_err
                }
            }

            fn set_group_json(&mut self, group: &str, json: &str) -> Result<Self::ChangedSet, Self::Error> {
                match group {
                    #(#set_arms)*
                    #default_err
                }
            }

            fn set_field(&mut self, key: &str, value: &str) -> Result<Option<Self::ChangedSet>, Self::Error> {
                match key {
                    #(#set_field_arms),*,
                    _ => Ok(None),
                }
            }
        }

        #notify_channel_block

        #config_server_impl
    }
}
