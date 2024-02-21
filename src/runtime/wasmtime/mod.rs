pub mod polyfill;
pub mod rpc;
pub mod transmission;

mod reexports {
    pub use wasmtime;
    pub use wasmtime_environ;
    pub use wasmtime_wasi;
}
pub use reexports::*;

pub use polyfill::function as polyfill_function;

use std::sync::Arc;

use wasmtime::component;
use wasmtime_wasi::preview2::WasiView;

use crate::transport::Client;

pub trait WrpcView: WasiView {
    type Client: Client + Send + Sync;

    fn client(&self) -> &Arc<Self::Client>;

    fn component_type(&self) -> &component::types::Component;
    fn instance(&self) -> Option<component::Instance>;
}
