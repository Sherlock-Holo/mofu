use std::sync::Once;

use crate::log;

#[flutter_rust_bridge::frb(sync)]
pub fn setup() {
    static LOG_INIT_ONCE: Once = Once::new();

    LOG_INIT_ONCE.call_once(|| {
        log::init_tracing();
    })
}
