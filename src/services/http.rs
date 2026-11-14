//! HTTPS through Windows' own TLS stack (SChannel) and certificate store, shared by all services.

use std::sync::OnceLock;
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent(concat!("LifeHUD/", env!("CARGO_PKG_VERSION"), " (+https://github.com/lifehud)"))
            .build()
            .into()
    })
}

pub fn get(url: &str) -> Result<String, ureq::Error> {
    agent().get(url).call()?.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string()
}
