#![allow(unexpected_cfgs)]

extern crate alloc;

use wifi_caddy_proc::WifiCaddyConfig;

#[derive(Clone, Debug, Default, WifiCaddyConfig)]
pub struct TestConfig {
    #[config_store(notify = "Test")]
    #[config_form(page = "Main", fieldset = "General", help = "A test field")]
    test_field: String,

    #[config_store(notify = "Test")]
    #[config_form(page = "Main", fieldset = "General", help = "A numeric field")]
    test_number: u32,
}

fn main() {}
