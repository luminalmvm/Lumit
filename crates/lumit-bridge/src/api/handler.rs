//! Where the bridge runs a call from Dart.
//!
//! On the desktop this is the bridge's own handler, unchanged: each call that
//! isn't sync runs on a pool thread. The browser build has one thread, the
//! page's, and the engine's state lives in that thread's memory, so every call
//! runs there, in turn.

#[cfg(not(target_family = "wasm"))]
pub static FLUTTER_RUST_BRIDGE_HANDLER: std::sync::LazyLock<
    flutter_rust_bridge::DefaultHandler<flutter_rust_bridge::for_generated::SimpleThreadPool>,
> = std::sync::LazyLock::new(|| {
    assert_eq!(
        crate::frb_generated::FLUTTER_RUST_BRIDGE_CODEGEN_VERSION,
        flutter_rust_bridge::for_generated::FLUTTER_RUST_BRIDGE_RUNTIME_VERSION,
        "the bridge's generated code and its runtime are different versions",
    );
    flutter_rust_bridge::DefaultHandler::new_simple(Default::default())
});

#[cfg(target_family = "wasm")]
pub static FLUTTER_RUST_BRIDGE_HANDLER: std::sync::LazyLock<
    flutter_rust_bridge::DefaultHandler<OnThePage>,
> = std::sync::LazyLock::new(|| flutter_rust_bridge::DefaultHandler::new_simple(OnThePage));

/// A pool of no threads: the call runs now, on the page.
///
/// The bridge only hands a call over by posting it to a worker, so this gives
/// it something shaped like one whose `postMessage` runs the call straight
/// away, as a worker's own script would.
#[cfg(target_family = "wasm")]
pub struct OnThePage;

#[cfg(target_family = "wasm")]
impl flutter_rust_bridge::for_generated::BaseThreadPool for OnThePage {
    fn execute(
        &self,
        closure: flutter_rust_bridge::for_generated::TransferClosure<wasm_bindgen::JsValue>,
    ) {
        thread_local! {
            static WORKER: web_sys::Worker = {
                let post = js_sys::Function::new_with_args(
                    "data",
                    "const [payload, ...transfer] = data; \
                     wasm_bindgen.receive_transfer_closure(payload, transfer);",
                );
                let worker = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&worker, &"postMessage".into(), &post);
                wasm_bindgen::JsCast::unchecked_into(worker)
            };
        }
        WORKER.with(|worker| {
            if let Err(err) = closure.apply(worker) {
                web_sys::console::error_1(&err);
            }
        });
    }
}
