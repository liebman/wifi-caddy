//! Crate-path resolution for generated code.
//!
//! Generated code names its runtime dependencies by crate path, and those paths
//! are resolved in the *deriving* crate. Two profiles are supported:
//!
//! * **core** — bare names (`wifi_caddy::…`, `serde::…`), resolved from the
//!   user's own dependencies. This is the historical output and is used when the
//!   deriving crate depends on `wifi-caddy` directly.
//! * **facade** — absolute paths through a facade crate
//!   (`::esp_wifi_caddy::wifi_caddy::…`), used when the deriving crate depends on
//!   `esp-wifi-caddy` instead. The facade re-exports every crate the generated
//!   code needs, so no other dependency entries are required.
//!
//! The profile is selected by reading the deriving crate's manifest
//! (`proc-macro-crate`), the same mechanism `esp-hal-procmacros` uses to resolve
//! `esp-hal` in `#[esp_hal::main]`. `#[config_crate(…)]` overrides the choice.
//!
//! `enumset`, `alloc`, `core` and `defmt` are deliberately *not* routed: the
//! `enumset` derive resolves `::enumset` from the user's own dependency, and
//! `defmt` is gated on the user's `defmt` feature.

use proc_macro_crate::FoundCrate;
use proc_macro_crate::crate_name;
use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;

/// Package name of the platform-agnostic core crate.
const CORE_PACKAGE: &str = "wifi-caddy";
/// Package name of the ESP32 facade crate that re-exports the generated code's dependencies.
const FACADE_PACKAGE: &str = "esp-wifi-caddy";

/// Crate paths used by generated code.
#[derive(Clone)]
pub struct CratePaths {
    /// Path to the core crate root: `wifi_caddy` or `::esp_wifi_caddy::wifi_caddy`.
    pub wifi_caddy: TokenStream,
    /// Path to `serde`, used for the generated per-page DTO derives.
    pub serde: TokenStream,
    /// Path to `serde_json_core`, used by the `ConfigApi` impl.
    pub serde_json_core: TokenStream,
    /// Path to `embassy_sync`, used by the notify channel types.
    pub embassy_sync: TokenStream,
    /// Path to `static_cell`, used by the notify channel static.
    pub static_cell: TokenStream,
    /// `#[serde(crate = "…")]` container attribute for generated DTOs.
    ///
    /// Empty in core mode: `serde_derive` then resolves the user's own `serde`
    /// dependency. In facade mode it points at the facade's `serde` re-export.
    pub serde_crate_attr: TokenStream,
}

impl CratePaths {
    /// Core profile: bare crate names, resolved from the user's own dependencies.
    ///
    /// Output is identical to the paths emitted before crate-path resolution
    /// existed, so existing `wifi-caddy` users are unaffected.
    pub fn core() -> Self {
        Self {
            wifi_caddy: quote!(wifi_caddy),
            serde: quote!(serde),
            serde_json_core: quote!(serde_json_core),
            embassy_sync: quote!(embassy_sync),
            static_cell: quote!(static_cell),
            serde_crate_attr: quote! {},
        }
    }

    /// Core profile with the core crate reached through an explicit root.
    ///
    /// Used when the deriving crate renamed its `wifi-caddy` dependency, or when
    /// the derive is used inside `wifi-caddy` itself (`root` = `crate`). The
    /// other crates stay bare: `wifi-caddy` does not re-export them.
    fn core_rooted(root: &str) -> Self {
        let root: syn::Path = syn::parse_str(root).expect("core root must be a valid path");
        Self {
            wifi_caddy: quote!(#root),
            ..Self::core()
        }
    }

    /// Facade profile: every path goes through `root`.
    ///
    /// `root` is a path to the facade crate, e.g. `::esp_wifi_caddy` or, when the
    /// derive is used inside the facade itself, `crate`.
    fn facade(root: &str) -> Self {
        let root_path: syn::Path = syn::parse_str(root).expect("facade root must be a valid path");
        let serde_crate = syn::LitStr::new(&format!("{root}::serde"), Span::call_site());
        Self {
            wifi_caddy: quote!(#root_path::wifi_caddy),
            serde: quote!(#root_path::serde),
            serde_json_core: quote!(#root_path::serde_json_core),
            embassy_sync: quote!(#root_path::embassy_sync),
            static_cell: quote!(#root_path::static_cell),
            serde_crate_attr: quote!(#[serde(crate = #serde_crate)]),
        }
    }

    /// Resolve the profile for a config struct.
    ///
    /// `#[config_crate(…)]` wins; otherwise the deriving crate's manifest decides:
    /// a direct `wifi-caddy` dependency selects the core profile, a direct
    /// `esp-wifi-caddy` dependency the facade profile, and if neither is found the
    /// core profile is used (so error messages keep naming `wifi_caddy`).
    pub fn resolve(attrs: &[syn::Attribute]) -> Self {
        if let Some(paths) = parse_config_crate_attr(attrs) {
            return paths;
        }

        match crate_name(CORE_PACKAGE) {
            Ok(FoundCrate::Name(name)) => {
                return if name == "wifi_caddy" {
                    Self::core()
                } else {
                    Self::core_rooted(&format!("::{name}"))
                };
            }
            Ok(FoundCrate::Itself) => return Self::core_rooted("crate"),
            Err(_) => {}
        }

        match crate_name(FACADE_PACKAGE) {
            Ok(FoundCrate::Name(name)) => Self::facade(&format!("::{name}")),
            // Deriving inside the facade crate itself.
            Ok(FoundCrate::Itself) => Self::facade("crate"),
            // Neither crate is a dependency: keep the historical paths so the
            // compile error names the crate the user is expected to add.
            Err(_) => Self::core(),
        }
    }
}

/// Parse the optional `#[config_crate(…)]` override.
///
/// Accepted forms:
///
/// * `#[config_crate(core)]` — core profile (bare paths).
/// * `#[config_crate(wifi_caddy)]` — core profile, same as omitting the attribute.
/// * `#[config_crate(esp_wifi_caddy)]` — facade profile rooted at `::esp_wifi_caddy`.
/// * `#[config_crate("crate")]` — facade profile rooted at an arbitrary path, for
///   use inside the facade crate itself.
fn parse_config_crate_attr(attrs: &[syn::Attribute]) -> Option<CratePaths> {
    for attr in attrs {
        if !attr.path().is_ident("config_crate") {
            continue;
        }

        if let Ok(path) = attr.parse_args::<syn::Path>() {
            let name = quote!(#path).to_string().replace(' ', "");
            return Some(match name.as_str() {
                "core" | "wifi_caddy" => CratePaths::core(),
                _ if path.segments.len() == 1 => CratePaths::facade(&format!("::{name}")),
                _ => CratePaths::facade(&name),
            });
        }

        if let Ok(lit) = attr.parse_args::<syn::LitStr>() {
            let path = lit.value();
            return Some(match path.as_str() {
                "core" => CratePaths::core(),
                _ => CratePaths::facade(&path),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core profile must keep emitting exactly the historical crate names.
    #[test]
    fn core_profile_uses_bare_paths() {
        let paths = CratePaths::core();
        assert_eq!(paths.wifi_caddy.to_string(), "wifi_caddy");
        assert_eq!(paths.serde.to_string(), "serde");
        assert_eq!(paths.serde_json_core.to_string(), "serde_json_core");
        assert_eq!(paths.embassy_sync.to_string(), "embassy_sync");
        assert_eq!(paths.static_cell.to_string(), "static_cell");
        assert!(
            paths.serde_crate_attr.is_empty(),
            "core mode must not emit #[serde(crate = ...)]"
        );
    }

    /// The facade profile must route every path and point serde at the facade.
    #[test]
    fn facade_profile_roots_all_paths() {
        let paths = CratePaths::facade("::esp_wifi_caddy");
        let compact = |ts: &TokenStream| ts.to_string().replace(' ', "");
        assert_eq!(compact(&paths.wifi_caddy), "::esp_wifi_caddy::wifi_caddy");
        assert_eq!(compact(&paths.serde), "::esp_wifi_caddy::serde");
        assert_eq!(
            compact(&paths.serde_json_core),
            "::esp_wifi_caddy::serde_json_core"
        );
        assert_eq!(
            compact(&paths.embassy_sync),
            "::esp_wifi_caddy::embassy_sync"
        );
        assert_eq!(compact(&paths.static_cell), "::esp_wifi_caddy::static_cell");
        assert_eq!(
            compact(&paths.serde_crate_attr),
            "#[serde(crate=\"::esp_wifi_caddy::serde\")]"
        );
    }

    /// `#[config_crate(...)]` must override detection, including the core escape
    /// and the arbitrary-path (string) form.
    #[test]
    fn config_crate_attr_overrides_detection() {
        let input: syn::DeriveInput =
            syn::parse_str("#[config_crate(esp_wifi_caddy)] struct S { f: u32 }").unwrap();
        let paths = CratePaths::resolve(&input.attrs);
        assert_eq!(
            paths.wifi_caddy.to_string().replace(' ', ""),
            "::esp_wifi_caddy::wifi_caddy"
        );

        let input: syn::DeriveInput =
            syn::parse_str("#[config_crate(core)] struct S { f: u32 }").unwrap();
        assert_eq!(CratePaths::resolve(&input.attrs).wifi_caddy.to_string(), "wifi_caddy");

        let input: syn::DeriveInput =
            syn::parse_str("#[config_crate(\"crate\")] struct S { f: u32 }").unwrap();
        assert_eq!(
            CratePaths::resolve(&input.attrs)
                .wifi_caddy
                .to_string()
                .replace(' ', ""),
            "crate::wifi_caddy"
        );
    }

    /// Without an override, this crate's manifest (`wifi-caddy` in
    /// `[dev-dependencies]`) selects the core profile.
    #[test]
    fn detection_falls_back_to_core_for_wifi_caddy_users() {
        let paths = CratePaths::resolve(&[]);
        assert_eq!(paths.wifi_caddy.to_string(), "wifi_caddy");
        assert!(paths.serde_crate_attr.is_empty());
    }
}
