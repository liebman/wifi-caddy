#![allow(unexpected_cfgs)]

//! Regression test for the generated config page.
//!
//! Each `page = "..."` becomes its own `<form>` + config group, and the tab
//! script tracks unsaved edits per tab. This locks in the per-tab Save behavior
//! and the per-tab messaging/indicator that surfaces it in the UI.

extern crate alloc;

use wifi_caddy::config_storage::ConfigFormGen;
use wifi_caddy_proc::WifiCaddyConfig;

#[derive(Clone, Debug, Default, WifiCaddyConfig)]
pub struct PageTestConfig {
    #[config_store(notify = "Wifi")]
    #[config_form(page = "Network", fieldset = "WiFi", help = "SSID")]
    wifi_ssid: String,
    #[config_store(notify = "Wifi")]
    #[config_form(
        page = "Network",
        fieldset = "WiFi",
        input_type = "password",
        help = "Password"
    )]
    wifi_pass: String,
    #[config_store(notify = "Example")]
    #[config_form(page = "Example", fieldset = "Example", help = "String")]
    example_string: String,
    #[config_store(notify = "Example")]
    #[config_form(page = "Example", fieldset = "Example", help = "Integer")]
    example_integer: u32,
}

#[test]
fn generated_page_saves_per_tab() {
    let page = PageTestConfig::config_page();

    // One form per page, each posting to its own config group.
    assert!(page.contains("configForm-Network"), "missing Network form");
    assert!(page.contains("configForm-Example"), "missing Example form");
    assert!(
        page.contains("CONFIG_URL_Network=\"/config-group/\"+CONFIG_PAGE_Network"),
        "Network page is not bound to the Network group"
    );
    assert!(
        page.contains("CONFIG_URL_Example=\"/config-group/\"+CONFIG_PAGE_Example"),
        "Example page is not bound to the Example group"
    );

    // Each page's save function reads only its own form.
    assert!(page.contains("document.getElementById(\"configForm-Network\")"));
    assert!(page.contains("document.getElementById(\"configForm-Example\")"));

    // Per-tab save message and unsaved-changes indicator.
    assert!(page.contains("pageLabel(page) + ' saved'"));
    assert!(page.contains("button.config-tab.dirty::after"));
    assert!(
        !page.contains("showMessage('Configuration saved')"),
        "the hardcoded all-tabs save message should be gone"
    );
}
