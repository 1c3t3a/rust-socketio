use crypto::{digest::Digest, sha2::Sha256};
use rand::{thread_rng, Rng};
use std::{sync::Arc, time::Duration};
use url::Url;

struct EngineIoConfig {
    sid: Arc<Box<dyn Fn() -> String>>,
    ping_timeout: Duration,
    ping_intervall: Duration,
    connected_url: Option<Url>,
}

fn generate_sid() -> String {
    let mut hasher = Sha256::new();
    let mut rng = thread_rng();
    let arr: [u8; 32] = rng.gen();
    hasher.input(&arr);
    hasher.result_str()
}

impl Default for EngineIoConfig {
    fn default() -> Self {
        // TODO: check if the timeout looks good
        EngineIoConfig {
            sid: Arc::new(Box::new(generate_sid)),
            ping_timeout: Duration::from_millis(5000),
            ping_intervall: Duration::from_millis(2500),
            connected_url: None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_default() {
        let config = EngineIoConfig::default();

        assert_eq!(config.connected_url, None);
        assert_eq!(config.ping_intervall, Duration::from_millis(2500));
        assert_eq!(config.ping_timeout, Duration::from_millis(5000));

        // TODO: Remove this debug line
        println!("{}", (config.sid)());
    }
}
