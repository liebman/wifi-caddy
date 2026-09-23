#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(clippy::all)]

mod config_api;
mod config_form;
mod config_store;
mod field_attrs;
mod paths;
mod utils;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Derive macro for WiFi caddy config structs.
///
/// Generates storage (load/store, keys, accessors), form HTML/JS for the config UI,
/// and the group API with HTTP server support and config-update notifications (always emitted).
/// Struct-level attributes only override defaults: `#[config_server(storage_magic, storage_version)]`,
/// `#[config_notify(cap = N)]`, `#[config_ui(default_group, title, ...)]`. Omitting `config_server` /
/// `config_notify` does not disable storage or the update channel; defaults apply.
///
/// The crate paths used by the generated code are resolved from your `Cargo.toml`:
/// a direct `wifi-caddy` dependency keeps the bare `wifi_caddy::*` paths, while an
/// `esp-wifi-caddy` dependency routes them through that facade crate. Use the
/// `#[config_crate(...)]` attribute to override the detected crate.
#[proc_macro_derive(
    WifiCaddyConfig,
    attributes(
        config_store,
        config_form,
        config_server,
        config_notify,
        config_ui,
        config_crate
    )
)]
pub fn derive_wifi_caddy_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let paths = paths::CratePaths::resolve(&input.attrs);
    // Three codegen passes, all emitted into the same module:
    // 1. store: ConfigKey enum, load/store, getters/setters
    let store = config_store::derive_config_store_impl(&input, &paths);
    // 2. form: HTML/JS segments for config UI
    let form = config_form::derive_config_form_impl(&input, &paths);
    // 3. group: ConfigApi, ConfigChange, notify channel, esp-wifi-caddy helpers
    let group = config_api::derive_config_api_impl(&input, &paths);
    proc_macro::TokenStream::from(quote::quote! {
        #store
        #form
        #group
    })
}

#[cfg(test)]
mod tests {
    use syn::parse_str;

    /// Asserts that `input_type = "password"` is recognized when other name-value pairs
    /// (page, fieldset, help) appear before it in `#[config_form(...)]`. Without consuming
    /// unrecognized meta values, the parse stream does not advance and input_type stays "text".
    #[test]
    fn config_form_password_recognized_after_fieldset_and_help() {
        let input: syn::DeriveInput = parse_str(
            r#"
            struct S {
                #[config_form(fieldset = "WiFi", input_type = "password", help = "Secret")]
                wifi_pass: String,
            }
            "#,
        )
        .unwrap();
        let syn::Data::Struct(data) = &input.data else {
            panic!("expected struct");
        };
        let field = data.fields.iter().next().unwrap();
        let attr = field
            .attrs
            .iter()
            .find(|a| a.path().is_ident("config_form"))
            .unwrap();
        let mut input_type = String::from("text");
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("input_type") {
                if let Ok(lit) = meta.value().and_then(|v| v.parse::<syn::LitStr>()) {
                    input_type = lit.value();
                }
            } else {
                // Consume (page, fieldset, help, etc.) so stream advances to input_type
                let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
            }
            Ok(())
        });
        assert_eq!(
            input_type, "password",
            "input_type must be recognized so GET /config-group/main redacts the field"
        );
    }

    /// Expands a config with the facade profile and asserts that every routed crate
    /// path goes through the facade, while `enumset` stays with the user and core
    /// mode keeps the historical bare paths.
    #[test]
    fn generated_code_follows_the_resolved_profile() {
        let input: syn::DeriveInput = parse_str(
            r#"
            #[config_crate(esp_wifi_caddy)]
            struct TestConfig {
                #[config_store(notify = "Test")]
                #[config_form(page = "Main", fieldset = "General", help = "A field")]
                test_field: String,
                #[config_store(notify = "Test")]
                #[config_form(page = "Main", fieldset = "General", help = "A number")]
                test_number: u32,
            }
            "#,
        )
        .unwrap();

        let expand = |attrs: &[syn::Attribute]| {
            let paths = crate::paths::CratePaths::resolve(attrs);
            let store = crate::config_store::derive_config_store_impl(&input, &paths);
            let form = crate::config_form::derive_config_form_impl(&input, &paths);
            let api = crate::config_api::derive_config_api_impl(&input, &paths);
            format!("{store}{form}{api}").replace(' ', "")
        };

        let facade = expand(&input.attrs);
        for expected in [
            "::esp_wifi_caddy::wifi_caddy::config_storage::ConfigApi",
            "::esp_wifi_caddy::wifi_caddy::config_storage::ConfigValue",
            "::esp_wifi_caddy::wifi_caddy::config_storage::ConfigFormGen",
            "::esp_wifi_caddy::wifi_caddy::config_storage::ConfigError",
            "::esp_wifi_caddy::embassy_sync::channel::Channel",
            "::esp_wifi_caddy::static_cell::StaticCell",
            "::esp_wifi_caddy::serde_json_core::to_slice",
            "#[serde(crate=\"::esp_wifi_caddy::serde\")]",
            "#[derive(::esp_wifi_caddy::serde::Serialize,::esp_wifi_caddy::serde::Deserialize)]",
            // enumset is never routed: the `EnumSetType` derive names `::enumset` itself.
            "enumset::EnumSet<ConfigChange>",
        ] {
            assert!(
                facade.contains(expected),
                "facade output is missing {expected}"
            );
        }

        let core = expand(&[]);
        for expected in [
            "wifi_caddy::config_storage::ConfigApi",
            "embassy_sync::channel::Channel",
            "static_cell::StaticCell",
            "serde_json_core::to_slice",
            "#[derive(serde::Serialize,serde::Deserialize)]",
        ] {
            assert!(core.contains(expected), "core output is missing {expected}");
        }
        assert!(
            !core.contains("serde(crate=") && !core.contains("esp_wifi_caddy"),
            "core output must stay free of facade paths"
        );
    }
}
