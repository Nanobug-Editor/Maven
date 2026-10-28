//! Maven policy lives in this independent guest; project evaluation belongs to the real Maven tool.
#[cfg(target_arch = "wasm32")]
mod application;
#[cfg(target_arch = "wasm32")]
mod configurations;
#[cfg(target_arch = "wasm32")]
mod execution;
#[cfg(target_arch = "wasm32")]
mod imports;
#[cfg(target_arch = "wasm32")]
mod language;
#[cfg(target_arch = "wasm32")]
mod model;
#[cfg(target_arch = "wasm32")]
mod panel;

#[cfg(target_arch = "wasm32")]
mod guest {
    use plugin_protocol::{
        api,
        bindings::{Guest, export},
    };
    use std::cell::RefCell;
    thread_local! {
        /// Bookkeeping holds metadata and owned process/reply handles, never editor text.
        static STATE: RefCell<crate::imports::State> = RefCell::new(Default::default());
    }
    /// One workspace instance owns its imports; host lifetime revokes every asynchronous receipt.
    struct MavenPlugin;
    impl Guest for MavenPlugin {
        /// Dispatch public host snapshots and command events; no ambient WASI filesystem is used.
        fn dispatch(payload: String) -> Result<String, String> {
            api::guest::dispatch(&payload, |input| {
                STATE.with_borrow_mut(|state| state.dispatch(input))
            })
        }
    }
    export!(MavenPlugin);
}
