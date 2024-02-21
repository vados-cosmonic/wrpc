use core::iter::zip;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, bail, ensure, Context as _};
use clap::Parser;
use futures::stream::{FuturesUnordered, StreamExt as _, TryStreamExt as _};
use indexmap::IndexMap;
use tokio::{fs, spawn};
use tracing::{debug, error, instrument, trace, warn};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;
use wasmcloud_component_adapters::WASI_PREVIEW1_REACTOR_COMPONENT_ADAPTER;
use wasmtime::component::{
    self, Component, Instance, InstancePre, Linker, LinkerInstance, Resource, ResourceAny,
    ResourceImportIndex, ResourceTable, Val,
};
use wasmtime::{AsContextMut, Engine};
use wasmtime_wasi::preview2::bindings::io::poll::{Host as _, HostPollable};
use wasmtime_wasi::preview2::{
    self, command, subscribe, Subscribe, WasiCtx, WasiCtxBuilder, WasiView,
};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};
use wrpc::runtime::wasmtime::transmission::from_wrpc_value;
use wrpc::runtime::wasmtime::{polyfill_function, WrpcView};
use wrpc::{function_exports, DynamicFunctionType, DynamicResourceType, ResourceType, Type, Value};
use wrpc_transport::Client as _;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// NATS address to use
    #[arg(short, long, default_value = "nats://127.0.0.1:4222")]
    nats: String,

    /// Prefix to listen on
    #[arg(short, long, default_value = "")]
    prefix: String,

    /// Path or URL to Wasm reactor component
    workload: String,
}

pub enum Workload {
    Url(Url),
    Binary(Vec<u8>),
}
//
//struct WasiIoResources {
//    pollable: Option<RuntimeImportIndex>,
//    input_stream: Option<RuntimeImportIndex>,
//    output_stream: Option<RuntimeImportIndex>,
//}
//
//struct WasiResources {
//    //io: WasiIoResources,
//}
//
//impl WasiResources {
//    pub fn lookup<T>(instance_pre: &InstancePre<T>) -> Self {
//        Self {
//            //io: WasiIoResources {
//            //    pollable: instance_pre
//            //        .path_import_index("wasi:io/poll@0.2.0-rc-2023-11-10", &["pollable"]),
//            //    input_stream: instance_pre
//            //        .path_import_index("wasi:io/streams@0.2.0-rc-2023-11-10", &["input-stream"]),
//            //    output_stream: instance_pre
//            //        .path_import_index("wasi:io/streams@0.2.0-rc-2023-11-10", &["output-stream"]),
//            //},
//        }
//    }
//}
//
struct Ctx {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    client: Arc<wrpc::transport::nats::Client>,
    //guest_resources: Arc<HashMap<DynamicResourceType, ResourceImportIndex>>,
    //wasi_resources: Arc<WasiResources>,
    component_ty: Arc<component::types::Component>,
    instance_pre: InstancePre<Self>,
    instance: Option<Instance>,
}

impl WasiView for Ctx {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl WasiHttpView for Ctx {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WrpcView for Ctx {
    type Client = wrpc::transport::nats::Client;

    fn client(&self) -> &Arc<Self::Client> {
        &self.client
    }

    fn component_type(&self) -> &component::types::Component {
        &self.component_ty
    }

    fn instance(&self) -> Option<component::Instance> {
        self.instance
    }
}
////
////struct RemotePollable {
////    nats: Arc<async_nats::Client>,
////    subject: async_nats::Subject,
////    // TODO: Interface/protocol
////}
////
//////#[async_trait]
//////impl Subscribe for RemotePollable {
//////    async fn ready(&mut self) {
//////        let wrpc::nats::Response { payload, .. } =
//////            connect(&self.nats, self.subject.clone(), Bytes::new(), None)
//////                .await
//////                .context("failed to connect to peer")?;
//////        ensure!(payload.is_empty());
//////        Ok(())
//////    }
//////}
////
//
////
////#[instrument(level = "trace", skip(store, root, child, payload))]
////#[async_recursion]
////async fn read_value(
////    mut store: impl AsContextMut<Data = Ctx> + Send + 'async_recursion,
////    root: &mut async_nats::Subscriber,
////    child: &mut async_nats::Subscriber,
////    payload: impl Buf + Send + 'static,
////    ty: &Type,
////    w_ty: &component::Type,
////) -> anyhow::Result<(Val, Box<dyn Buf + Send>)> {
////    let mut payload: Box<dyn Buf + Send> = Box::new(payload);
////    let mut buf = vec![];
////    loop {
////        match ty {
////            Type::Bool => {
////                if payload.remaining() >= 1 {
////                    return Ok((Val::Bool(payload.get_u8() == 1), payload));
////                }
////            }
////            Type::U8 => {
////                if payload.remaining() >= 1 {
////                    return Ok((Val::U8(payload.get_u8()), payload));
////                }
////            }
////            Type::U16 => {
////                if payload.remaining() >= 2 {
////                    return Ok((Val::U16(payload.get_u16_le()), payload));
////                }
////            }
////            Type::U32 => {
////                if payload.remaining() >= 4 {
////                    return Ok((Val::U32(payload.get_u32_le()), payload));
////                }
////            }
////            Type::U64 => {
////                if payload.remaining() >= 8 {
////                    return Ok((Val::U64(payload.get_u64_le()), payload));
////                }
////            }
////            Type::S8 => {
////                if payload.remaining() >= 1 {
////                    return Ok((Val::S8(payload.get_i8()), payload));
////                }
////            }
////            Type::S16 => {
////                if payload.remaining() >= 2 {
////                    return Ok((Val::S16(payload.get_i16_le()), payload));
////                }
////            }
////            Type::S32 => {
////                if payload.remaining() >= 4 {
////                    return Ok((Val::S32(payload.get_i32_le()), payload));
////                }
////            }
////            Type::S64 => {
////                if payload.remaining() >= 8 {
////                    return Ok((Val::S64(payload.get_i64_le()), payload));
////                }
////            }
////            Type::Float32 => {
////                if payload.remaining() >= 4 {
////                    return Ok((Val::Float32(payload.get_f32_le()), payload));
////                }
////            }
////            Type::Float64 => {
////                if payload.remaining() >= 8 {
////                    return Ok((Val::Float64(payload.get_f64_le()), payload));
////                }
////            }
////            Type::Char => {
////                if payload.remaining() >= 4 {
////                    let v = payload
////                        .get_u32_le()
////                        .try_into()
////                        .context("char is not valid")?;
////                    return Ok((Val::Char(v), payload));
////                }
////            }
////            Type::String => {
////                let mut r = payload.reader();
////                r.read_until(0, &mut buf)
////                    .context("failed to read from buffer")?;
////                match buf.pop() {
////                    Some(0) => {
////                        let v = String::from_utf8(buf).context("string is not valid UTF-8")?;
////                        return Ok((Val::String(v.into()), r.into_inner()));
////                    }
////                    Some(c) => {
////                        buf.push(c);
////                    }
////                    _ => {}
////                }
////                payload = r.into_inner();
////            }
////            Type::List(ty) => {
////                let component::Type::List(w_ty) = w_ty else {
////                    bail!("Wasmtime type mismatch, expected list, got {w_ty:?}")
////                };
////                if payload.remaining() >= 4 {
////                    let len = payload
////                        .get_u32_le()
////                        .try_into()
////                        .context("list length does not fit in usize")?;
////                    let mut els = Vec::with_capacity(len);
////                    let el_ty = w_ty.ty();
////                    for i in 0..len {
////                        let el;
////                        (el, payload) = read_value(&mut store, root, child, payload, ty, &el_ty)
////                            .await
////                            .with_context(|| {
////                                format!("failed to decode value of list element {i}")
////                            })?;
////                        els.push(el);
////                    }
////                    let v = w_ty.new_val(els.into()).context("failed to create list")?;
////                    return Ok((v, payload));
////                }
////            }
////            Type::Record(_)
////            | Type::Tuple(_)
////            | Type::Variant(_)
////            | Type::Enum(_) => {
////                bail!("not supported yet")
////            }
////            Type::Option(ty) => {
////                let component::Type::Option(w_ty) = w_ty else {
////                    bail!("Wasmtime type mismatch, expected option, got {w_ty:?}")
////                };
////                if payload.remaining() >= 1 {
////                    if payload.get_u8() == 0 {
////                        let v = w_ty
////                            .new_val(None)
////                            .context("failed to create `option::none` value")?;
////                        return Ok((v, payload));
////                    } else {
////                        let (v, payload) = read_value(store, root, child, payload, ty, &w_ty.ty())
////                            .await
////                            .context("failed to decode `option::some` value")?;
////                        let v = w_ty
////                            .new_val(Some(v))
////                            .context("failed to create `option::some` value")?;
////                        return Ok((v, payload));
////                    }
////                };
////            }
////            Type::Result { .. } => bail!("not supported yet"),
////            Type::Flags(n) => {
////                let component::Type::Flags(w_ty) = w_ty else {
////                    bail!("Wasmtime type mismatch, expected flags, got {w_ty:?}")
////                };
////                match n {
////                    0..=256 => {
////                        todo!()
////                    }
////                    256..=65536 => {
////                        todo!()
////                    }
////                    65536.. => {
////                        todo!()
////                    }
////                }
////            }
////            Type::Resource(ty) => {
////                let mut store = store.as_context_mut();
////                match ty {
////                    types::Resource::Pollable => {
////                        if payload.remaining() >= 1 {
////                            match payload.get_u8() {
////                                0 => {
////                                    //let pollable = table
////                                    //    .push(RemotePollable {
////                                    //        nats: Arc::clone(nats),
////                                    //        subject: v.to_subject(),
////                                    //    })
////                                    //    .context("failed to push pollable to table")?;
////                                    // let _res = subscribe(table, pollable)
////                                    // .context("failed to subscribe to pollable")?;
////                                    bail!("pending pollables not supported yet")
////                                }
////                                1 => {
////                                    struct Ready;
////
////                                    #[async_trait]
////                                    impl Subscribe for Ready {
////                                        async fn ready(&mut self) {}
////                                    }
////
////                                    let pollable = WasiView::table(store.data_mut())
////                                        .push(Ready)
////                                        .context("failed to push resource to table")?;
////                                    let _pollable =
////                                        subscribe(WasiView::table(store.data_mut()), pollable)
////                                            .context("failed to subscribe")?;
////                                    let _instance_pre = store.data().instance_pre.clone();
////                                    //let idx = store.data().wasi_resources.io.pollable;
////                                    //let pollable = pollable
////                                    //    .try_into_resource_any(&mut store, &instance_pre, idx)
////                                    //    .context("failed to convert pollable to `ResourceAny`")?;
////                                    bail!("ready pollables not supported yet")
////                                }
////                                _ => bail!("invalid `pollable` value"),
////                            }
////                            //return Ok(payload);
////                        }
////                    }
////                    types::Resource::InputStream => todo!(),
////                    types::Resource::OutputStream => todo!(),
////                    types::Resource::Guest(ty) => {
////                        trace!(?ty, "decode guest type");
////                        let mut r = payload.reader();
////                        r.read_until(0, &mut buf)
////                            .context("failed to read from buffer")?;
////                        match buf.pop() {
////                            Some(0) => {
////                                let subject =
////                                    String::from_utf8(buf).context("string is not valid UTF-8")?;
////                                let subject = WasiView::table(store.data_mut())
////                                    .push(subject.to_subject())
////                                    .context("failed to push guest resource to table")?;
////                                let instance_pre = store.data().instance_pre.clone();
////                                let idx = *store
////                                    .data()
////                                    .guest_resources
////                                    .get(ty)
////                                    .context("failed to lookup guest resource type index")?;
////                                let idx = instance_pre
////                                    .resource_import_index(idx)
////                                    .context("failed to lookup resource runtime import index")?;
////                                let subject = subject
////                                    .try_into_resource_any(&mut store, &instance_pre, idx)
////                                    .context("failed to convert resource to `ResourceAny`")?;
////                                return Ok((Val::Resource(subject), r.into_inner()));
////                            }
////                            Some(c) => {
////                                buf.push(c);
////                            }
////                            _ => {}
////                        }
////                        payload = r.into_inner();
////                    }
////                }
////            }
////        }
////        // TODO: timeout
////        trace!("await root message");
////        let msg = root.next().await.context("failed to receive message")?;
////        trace!(?msg, "root message received");
////        payload = Box::new(payload.chain(msg.payload))
////    }
////}
////
////
////#[instrument(level = "trace", skip_all)]
////#[async_recursion]
////async fn write_value<'a: 'async_recursion>(
////    mut store: impl AsContextMut<Data = Ctx> + Send + 'async_recursion,
////    payload: &mut BytesMut,
////    v: ComponentValue<'a>,
////) -> anyhow::Result<GuestAsyncValues> {
////    let mut store = store.as_context_mut();
////    match (v.value, v.ty) {
////        (Val::Bool(false), Type::Bool) => {
////            payload.put_u8(0);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::Bool(true), Type::Bool) => {
////            payload.put_u8(1);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::S8(v), Type::S8) => {
////            payload.put_i8(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::U8(v), Type::U8) => {
////            payload.put_u8(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::S16(v), Type::S16) => {
////            payload.put_i16_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::U16(v), Type::U16) => {
////            payload.put_u16_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::S32(v), Type::S32) => {
////            payload.put_i32_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::U32(v), Type::U32) => {
////            payload.put_u32_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::S64(v), Type::S64) => {
////            payload.put_i64_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::U64(v), Type::U64) => {
////            payload.put_u64_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::Float32(v), Type::Float32) => {
////            payload.put_f32_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::Float64(v), Type::Float64) => {
////            payload.put_f64_le(*v);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::Char(v), Type::Char) => {
////            payload.put_u32_le(u32::from(*v));
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::String(v), Type::String) => {
////            payload.put_slice(v.as_bytes());
////            payload.put_u8(0);
////            Ok(GuestAsyncValues::default())
////        }
////        (Val::List(vs), Type::List(ty)) => {
////            let vs = vs.deref();
////            let len: u32 = vs
////                .len()
////                .try_into()
////                .context("list length does not fit in u32")?;
////            payload.put_u32_le(len);
////            write_value_iter(
////                store,
////                payload,
////                vs.iter().map(|value| ComponentValue { value, ty }),
////            )
////            .await
////        }
////        (Val::Record(vs), Type::Record(tys)) => {
////            write_value_iter(
////                store,
////                payload,
////                vs.fields()
////                    .zip(tys)
////                    .map(|((_, value), ty)| ComponentValue { value, ty }),
////            )
////            .await
////        }
////        (Val::Tuple(vs), Type::Tuple(tys)) => {
////            write_value_iter(
////                store,
////                payload,
////                vs.values()
////                    .iter()
////                    .zip(tys)
////                    .map(|(value, ty)| ComponentValue { value, ty }),
////            )
////            .await
////        }
////        (Val::Variant(_v), Type::Variant(..)) => {
////            bail!("variants not supported yet")
////        }
////        (Val::Enum(_v), Type::Enum(..)) => {
////            bail!("enums not supported yet")
////        }
////        (Val::Option(v), Type::Option(ty)) => {
////            if let Some(value) = v.value() {
////                payload.put_u8(1);
////                write_value(&mut store, payload, ComponentValue { value, ty }).await
////            } else {
////                payload.put_u8(0);
////                Ok(GuestAsyncValues::default())
////            }
////        }
////        (Val::Result(v), ty) => match (v.value(), ty) {
////            (Ok(None), Type::Result { ok: None, .. }) => {
////                payload.put_u8(1);
////                Ok(GuestAsyncValues::default())
////            }
////            (Ok(Some(value)), Type::Result { ok: Some(ty), .. }) => {
////                payload.put_u8(1);
////                write_value(&mut store, payload, ComponentValue { value, ty }).await
////            }
////            (Err(None), Type::Result { err: None, .. }) => {
////                payload.put_u8(0);
////                Ok(GuestAsyncValues::default())
////            }
////            (Err(Some(value)), Type::Result { err: Some(ty), .. }) => {
////                payload.put_u8(0);
////                write_value(&mut store, payload, ComponentValue { value, ty }).await
////            }
////            _ => bail!("value type mismatch"),
////        },
////        (Val::Flags(_v), Type::Flags(..)) => {
////            bail!("flags not supported yet")
////        }
////        (Val::Resource(resource), Type::Resource(types::Resource::Pollable)) => {
////            let pollable: Resource<preview2::Pollable> = resource
////                .try_into_resource(&mut store)
////                .context("failed to obtain pollable")?;
////            let ready = store
////                .data_mut()
////                .ready(Resource::new_borrow(pollable.rep()))
////                .await
////                .context("failed to check pollable readiness")?;
////            if ready {
////                //resource
////                //    .resource_drop_async(&mut store)
////                //    .await
////                //    .context("failed to drop resource")?;
////                store
////                    .data_mut()
////                    .drop(pollable)
////                    .context("failed to drop pollable")?;
////                payload.put_u8(1);
////                Ok(GuestAsyncValues::default())
////            } else {
////                payload.put_u8(0);
////                Ok(GuestAsyncValues::from(GuestAsyncValue::Pollable(
////                    GuestLocalPollable {
////                        guest_resource: *resource,
////                        pollable,
////                    },
////                )))
////            }
////        }
////        (Val::Resource(resource), Type::Resource(types::Resource::InputStream)) => {
////            let _stream: Resource<preview2::pipe::AsyncReadStream> = resource
////                .try_into_resource(&mut store)
////                .context("failed to obtain read stream")?;
////            //if stream.owned() {
////            //    let stream = store
////            //        .data_mut()
////            //        .table()
////            //        .delete(res)
////            //        .context("failed to delete resource")?;
////            //} else {
////            //    let stream = store
////            //        .data()
////            //        .table()
////            //        .get(&res)
////            //        .context("failed to get resource")?;
////            //};
////            // if ready and closed, write [1, bytes]
////            // if not closed, [0]
////            bail!("streams not supported yet")
////        }
////        (Val::Resource(resource), Type::Resource(types::Resource::Guest(ty))) => {
////            if resource.ty() == ResourceType::host::<async_nats::Subject>() {
////                let subject: Resource<async_nats::Subject> = resource
////                    .try_into_resource(&mut store)
////                    .context("failed to obtain custom resource")?;
////                if subject.owned() {
////                    let subject = WasiView::table(store.data_mut())
////                        .delete(subject)
////                        .context("failed to delete subject resource")?;
////                    payload.put(subject.as_bytes());
////                } else {
////                    let subject = WasiView::table(store.data_mut())
////                        .get(&subject)
////                        .context("failed to get resource")?;
////                    payload.put(subject.as_bytes());
////                };
////                payload.put_u8(0);
////                Ok(GuestAsyncValues::default())
////            } else {
////                let nats = &store.data().nats;
////                let subject = nats.new_inbox();
////                payload.put_slice(subject.as_bytes());
////                payload.put_u8(0);
////                let listener = listen(nats, format!("{subject}.>"))
////                    .await
////                    .context("failed to listen on inbox")?;
////                Ok(GuestAsyncValues::from(GuestAsyncValue::Resource(
////                    GuestResource {
////                        listener,
////                        subject,
////                        resource: *resource,
////                        ty: ty.clone(),
////                    },
////                )))
////            }
////        }
////        _ => bail!("value type mismatch"),
////    }
////}
////
////#[derive(Debug)]
////enum ComponentResourceValue {
////    Pollable(Resource<preview2::Pollable>),
////    InputStream(Resource<preview2::pipe::AsyncReadStream>),
////    OutputStream(Resource<preview2::pipe::AsyncWriteStream>),
////    Guest(ResourceAny),
////}
////
//#[derive(Debug, Clone)]
//struct ComponentValue<'a> {
//    value: &'a Val,
//    ty: &'a Type,
//}
//
//impl<'a> From<(&'a Val, &'a Type)> for ComponentValue<'a> {
//    fn from((value, ty): (&'a Val, &'a Type)) -> Self {
//        Self { value, ty }
//    }
//}
////
////#[derive(Debug)]
////struct ComponentValueMut<'a> {
////    value: &'a mut Val,
////    ty: &'a Type,
////}
////
////impl<'a> From<(&'a mut Val, &'a Type)> for ComponentValueMut<'a> {
////    fn from((value, ty): (&'a mut Val, &'a Type)) -> Self {
////        Self { value, ty }
////    }
////}
////
////enum HostType {
////    Guest(Type),
////    Future(Box<HostType>),
////    Stream(Box<HostType>),
////}
////
//
#[instrument(level = "trace", skip_all, ret)]
fn polyfill(
    component: &Component,
    resolve: &wit_parser::Resolve,
    exports: &Arc<HashMap<String, HashMap<String, DynamicFunctionType>>>,
    imports: &IndexMap<wit_parser::WorldKey, wit_parser::WorldItem>,
    linker: &mut Linker<Ctx>,
) -> HashMap<Arc<DynamicResourceType>, ResourceImportIndex> {
    let mut paths = HashMap::new();
    for (wk, wi) in imports {
        let instance_name = resolve.name_world_key(wk);
        match instance_name.as_str() {
            "wasi:cli/environment@0.2.0"
            | "wasi:cli/exit@0.2.0"
            | "wasi:cli/stderr@0.2.0"
            | "wasi:cli/stdin@0.2.0"
            | "wasi:cli/stdout@0.2.0"
            | "wasi:cli/terminal-input@0.2.0"
            | "wasi:cli/terminal-output@0.2.0"
            | "wasi:cli/terminal-stderr@0.2.0"
            | "wasi:cli/terminal-stdin@0.2.0"
            | "wasi:cli/terminal-stdout@0.2.0"
            | "wasi:clocks/monotonic-clock@0.2.0"
            | "wasi:clocks/wall-clock@0.2.0"
            | "wasi:filesystem/preopens@0.2.0"
            | "wasi:filesystem/types@0.2.0"
            | "wasi:http/incoming-handler@0.2.0"
            | "wasi:http/outgoing-handler@0.2.0"
            | "wasi:http/types@0.2.0"
            | "wasi:io/error@0.2.0"
            | "wasi:io/poll@0.2.0"
            | "wasi:io/streams@0.2.0"
            | "wasi:sockets/tcp@0.2.0" => continue,
            _ => {
                let wit_parser::WorldItem::Interface(interface) = wi else {
                    continue;
                };
                let mut linker = linker.root();
                let mut linker = match linker.instance(&instance_name) {
                    Ok(linker) => linker,
                    Err(err) => {
                        error!(
                            ?err,
                            instance_name, "failed to instantiate interface from root"
                        );
                        continue;
                    }
                };
                let Some(wit_parser::Interface {
                    types, functions, ..
                }) = resolve.interfaces.get(*interface)
                else {
                    warn!("component imports a non-existent interface");
                    continue;
                };
                let instance_name = Arc::new(instance_name);
                for (func_name, ty) in functions {
                    let func_name = Arc::new(func_name.to_string());
                    if let Err(err) = polyfill_function(
                        component,
                        &mut linker,
                        resolve,
                        exports,
                        &instance_name,
                        &func_name,
                        ty,
                    ) {
                        warn!(
                            ?err,
                            func_name = func_name.as_str(),
                            "failed to polyfill function"
                        )
                    }
                }
                for (name, ty) in types {
                    let Some(ty) = resolve.types.get(*ty) else {
                        warn!("component imports a non-existent type");
                        continue;
                    };
                    match Type::resolve_def(&resolve, ty) {
                        Ok(Type::Resource(ResourceType::Dynamic(ty)))
                            if ty.instance == *instance_name =>
                        {
                            match linker.resource(
                                name,
                                component::ResourceType::host::<async_nats::Subject>(),
                                |_, _| Ok(()),
                            ) {
                                Ok(idx) => {
                                    paths.insert(ty, idx);
                                }
                                Err(err) => {
                                    error!(?err, name, "failed to polyfill resource")
                                }
                            }
                        }
                        Ok(ty) => debug!(?ty, "avoid polyfilling type"),
                        Err(err) => warn!(?err, "failed to resolve type definition"),
                    };
                }
            }
        }
    }
    paths
}
//
//#[instrument(
//    level = "trace",
//    skip(
//        nats,
//        component_ty,
//        engine,
//        instance_pre,
//        guest_resources,
//        exports,
//        payload,
//        conn
//    ),
//    ret
//)]
async fn handle_invocation(
    client: Arc<wrpc::transport::nats::Client>,
    engine: &Engine,
    component_ty: Arc<component::types::Component>,
    instance_pre: &InstancePre<Ctx>,
    guest_resources: Arc<HashMap<Arc<DynamicResourceType>, ResourceImportIndex>>,
    exports: &HashMap<String, HashMap<String, DynamicFunctionType>>,
    instance_name: &str,
    func_name: &str,
    params_ty: &[Type],
    results_ty: &[Type],
    params: Vec<Value>,
    tx_subject: wrpc::transport::nats::Subject,
    tx: wrpc::transport::nats::Transmitter,
) -> anyhow::Result<()> {
    let table = ResourceTable::new();
    let mut wasi = WasiCtxBuilder::new();
    let wasi = if instance_name.is_empty() {
        wasi.args(&[func_name])
    } else {
        wasi.args(&[instance_name, func_name])
    }
    .inherit_stdio()
    .build();
    let ctx = Ctx {
        wasi,
        http: WasiHttpCtx,
        table,
        client,
        //nats: Arc::clone(&nats),
        //guest_resources,
        //wasi_resources: Arc::new(WasiResources::lookup(&instance_pre)),
        component_ty,
        instance_pre: instance_pre.clone(),
        instance: None,
    };
    let mut store = wasmtime::Store::new(engine, ctx);
    let instance = instance_pre
        .instantiate_async(&mut store)
        .await
        .context("failed to instantiate component")?;
    store.data_mut().instance = Some(instance.clone());
    match (instance_name, func_name) {
        ("wasi:http/incoming-handler", "handle") => {
            bail!("handle WASI HTTP")
        }
        _ => {
            let func = {
                let mut exports = instance.exports(&mut store);
                if instance_name.is_empty() {
                    exports.root()
                } else {
                    exports
                        .instance(instance_name)
                        .with_context(|| format!("instance of `{instance_name}` not found"))?
                }
                .func(func_name)
                .with_context(|| format!("function `{func_name}` not found"))?
            };
            let params: Vec<_> = zip(params, params_ty)
                .zip(func.params(&store).iter())
                .map(|((val, ty), w_ty)| from_wrpc_value(&mut store, val, ty, &w_ty))
                .collect::<anyhow::Result<_>>()
                .context("failed to decode result value")?;
            let mut results = vec![Val::Bool(false); results_ty.len()];
            func.call_async(&mut store, &params, &mut results)
                .await
                .context("failed to call function")?;
            func.post_return_async(&mut store)
                .await
                .context("failed to perform post-return cleanup")?;
            //handle_pollables(&mut store, &instance, &nats, exports, pollables)
            //    .await
            //    .context("failed to handle pollables")?;
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .or_else(|_| tracing_subscriber::EnvFilter::try_new("info"))
                .expect("failed to setup env logging filter"),
        )
        .with(tracing_subscriber::fmt::layer().without_time())
        .init();

    let Args {
        nats,
        prefix,
        workload,
    } = Args::parse();
    let nats = async_nats::connect(nats)
        .await
        .context("failed to connect to NATS")?;
    let client = wrpc::transport::nats::Client::new(nats, prefix);
    let client = Arc::new(client);

    let engine = Engine::new(
        wasmtime::Config::new()
            .async_support(true)
            .wasm_component_model(true),
    )
    .context("failed to initialize Wasmtime engine")?;

    let wasm = if workload.starts_with('.') {
        fs::read(&workload)
            .await
            .with_context(|| format!("failed to read relative path to workload `{workload}`"))
            .map(Workload::Binary)
    } else {
        Url::parse(&workload)
            .with_context(|| format!("failed to parse Wasm URL `{workload}`"))
            .map(Workload::Url)
    }?;
    let wasm = match wasm {
        Workload::Url(wasm) => match wasm.scheme() {
            "file" => {
                let wasm = wasm
                    .to_file_path()
                    .map_err(|_| anyhow!("failed to convert Wasm URL to file path"))?;
                fs::read(wasm)
                    .await
                    .context("failed to read Wasm from file URL")?
            }
            "http" | "https" => {
                let wasm = reqwest::get(wasm).await.context("failed to GET Wasm URL")?;
                let wasm = wasm.bytes().await.context("failed fetch Wasm from URL")?;
                wasm.to_vec()
            }
            scheme => bail!("URL scheme `{scheme}` not supported"),
        },
        Workload::Binary(wasm) => wasm,
    };
    let wasm = if wasmparser::Parser::is_core_wasm(&wasm) {
        wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&wasm)
            .context("failed to set core component module")?
            .adapter(
                "wasi_snapshot_preview1",
                WASI_PREVIEW1_REACTOR_COMPONENT_ADAPTER,
            )
            .context("failed to add WASI adapter")?
            .encode()
            .context("failed to encode a component")?
    } else {
        wasm
    };

    let (resolve, world) =
        match wit_component::decode(&wasm).context("failed to decode WIT component")? {
            wit_component::DecodedWasm::Component(resolve, world) => (resolve, world),
            wit_component::DecodedWasm::WitPackage(..) => {
                bail!("binary-encoded WIT packages not supported")
            }
        };
    let resolve = Arc::new(resolve);

    let component = Component::new(&engine, wasm).context("failed to parse component")?;

    let mut linker: Linker<Ctx> = Linker::new(&engine);

    command::add_to_linker(&mut linker).context("failed to link `wasi:cli/command` interfaces")?;
    wasmtime_wasi_http::bindings::wasi::http::types::add_to_linker(&mut linker, |ctx| ctx)
        .context("failed to link `wasi:http/types` interface")?;
    wasmtime_wasi_http::bindings::wasi::http::outgoing_handler::add_to_linker(&mut linker, |ctx| {
        ctx
    })
    .context("failed to link `wasi:http/outgoing-handler` interface")?;

    let wit_parser::World {
        exports, imports, ..
    } = resolve
        .worlds
        .iter()
        .find_map(|(id, w)| (id == world).then_some(w))
        .context("component world missing")?;

    let exports = Arc::new(function_exports(&resolve, exports));
    let guest_resources = Arc::new(polyfill(
        &component,
        &resolve,
        &exports,
        imports,
        &mut linker,
    ));

    let component_ty = linker
        .substituted_component_type(&component)
        .context("failed to derive component type")?;
    let component_ty = Arc::new(component_ty);

    let instance_pre = linker
        .instantiate_pre(&component)
        .context("failed to pre-instantiated component")?;

    let handlers = FuturesUnordered::new();
    for (instance_name, item) in component_ty.exports(&engine) {
        let item = match item {
            component::types::ComponentItem::ComponentInstance(item) => item,
            _ => continue,
        };
        let instance_name = Arc::new(instance_name.clone());
        for (func_name, item) in item.exports(&engine) {
            let ty = match item {
                component::types::ComponentItem::ComponentFunc(func_ty) => func_ty,
                _ => continue,
            };

            let engine = engine.clone();
            let instance_pre = instance_pre.clone();
            let func_name = func_name.to_string();

            let component_ty = Arc::clone(&component_ty);
            let exports = Arc::clone(&exports);
            let guest_resources = Arc::clone(&guest_resources);
            let instance_name = Arc::clone(&instance_name);
            let client = Arc::clone(&client);
            let invocations = client
                .serve(&instance_name, &func_name)
                .await
                .context("failed to serve")?;
            //let params_ty = Arc::clone(&params_ty);
            //let results_ty = Arc::clone(&results_ty);

            handlers.push(spawn(async move {
                //let invocations = client
                //    .serve_static(&instance_name, &func_name, Arc::clone(&params_ty))
                //    .await
                //    .context("failed to serve")?;
                //invocations
                //    .for_each_concurrent(None, |inv| async {
                //        match inv {
                //            Ok((params, tx_subject, tx)) => {
                //                if let Err(err) = handle_invocation(
                //                    Arc::clone(&client),
                //                    &engine,
                //                    component_ty.clone(),
                //                    &instance_pre,
                //                    Arc::clone(&guest_resources),
                //                    &exports,
                //                    &instance_name,
                //                    &func_name,
                //                    &params_ty,
                //                    &results_ty,
                //                    params,
                //                    tx_subject,
                //                    tx,
                //                )
                //                .await
                //                {
                //                    error!(?err, "failed to handle invocation")
                //                }
                //            }
                //            Err(err) => {
                //                error!(?err, "failed to accept invocation");
                //            }
                //        }
                //    })
                //    .await;
                Ok(())
            }))
        }
    }
    handlers
        .try_for_each_concurrent(None, |res: anyhow::Result<()>| async {
            if let Err(res) = res {
                error!("handler failed: {res}")
            }
            Ok(())
        })
        .await?;
    Ok(())
}
