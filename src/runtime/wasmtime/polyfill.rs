use core::iter::zip;
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Context as _};
use tokio::try_join;
use tracing::{instrument, trace};
use wasmtime::component::types::ComponentFunc;
use wasmtime::component::{self, Component, Linker, LinkerInstance};
use wrpc_runtime_wasmtime::{from_wrpc_value, to_wrpc_value};
use wrpc_transport::Client as _;
use wrpc_types::{DynamicFunction, Type};

use crate::runtime::wasmtime::rpc::handle_pollables;
use crate::runtime::wasmtime::WrpcView;

#[instrument(level = "trace", skip(component, linker), ret)]
pub fn function<T: WrpcView>(
    component: &Component,
    linker: &mut Linker<T>,
    instance_name: &Arc<String>,
    func_name: &Arc<String>,
    ty: ComponentFunc,
) -> anyhow::Result<()> {
    let mut linker = linker.root();
    let mut linker = match linker.instance(instance_name) {
        Ok(linker) => linker,
        Err(err) => {
            bail!("failed to instantiate interface from root")
        }
    };
    let instance_name = Arc::clone(&instance_name);
    let func_name = Arc::clone(&func_name);
    linker
        .func_new_async(
            component,
            &Arc::clone(&func_name),
            move |mut store, params, results| {
                let instance_name = Arc::clone(&instance_name);
                let func_name = Arc::clone(&func_name);
                Box::new(async move {
                    let client = Arc::clone(store.data().client());

                    //let mut pollables = vec![];
                    let params: Vec<_> = params
                        .iter()
                        .map(|val| to_wrpc_value(&mut store, val))
                        .collect::<anyhow::Result<_>>()
                        .context("failed to convert wasmtime parameters to wRPC values")?;

                    trace!(?instance_name, ?func_name, "call freestanding function");
                    let ((result_values, tx), ()) = try_join!(
                        async {
                            if false {
                                Ok((vec![], ()))
                            } else {
                                bail!("not supported yet")
                            }
                            //client
                            //    .invoke_component_dynamic(&instance_name, &func_name, params, &results_ty)
                            //    .await
                            //    .context("failed to invoke freestanding function")
                        },
                        async {
                            // TODO: Implement this correctly
                            if true {
                                bail!("not supported yet")
                                //let instance = store
                                //    .data()
                                //    .instance()
                                //    .clone()
                                //    .context("instance missing from store")?;
                                //handle_pollables(
                                //    &mut store, &instance, &client, &exports, pollables,
                                //)
                                //.await
                                //.context("failed to handle pollables")
                            } else {
                                Ok(())
                            }
                        }
                    )?;
                    trace!(
                        ?instance_name,
                        ?func_name,
                        "received freestanding function response"
                    );
                    let ty = store
                        .data()
                        .component_type()
                        .get_import(&instance_name)
                        .with_context(|| format!("import of `{instance_name}`not found"))?;
                    let component::types::ComponentItem::ComponentInstance(ty) = ty else {
                        bail!("component import type is not an instance: {ty:?}")
                    };
                    let ty = ty
                        .get_export(&func_name)
                        .context("instance does not export polyfilled function")?;
                    let component::types::ComponentItem::ComponentFunc(ty) = ty else {
                        bail!("instance export is not a function: {ty:?}")
                    };
                    for (i, (val, ty)) in zip(result_values, ty.results()).enumerate() {
                        let val = from_wrpc_value(val, &ty)?;
                        let result = results.get_mut(i).context("invalid result vector")?;
                        *result = val;
                    }
                    trace!(
                        ?instance_name,
                        ?func_name,
                        "decoded freestanding function response"
                    );
                    Ok(())
                })
            },
        )
        .context("failed to polyfill freestanding function")
    //let (_, name) = func_name
    //    .split_once('.')
    //    .context("method name is not valid")?;
    //let instance_name = Arc::clone(instance_name);
    //let name = Arc::new(name.to_string());
    //let exports = Arc::clone(exports);
    //linker
    //    .func_new_async(
    //        component,
    //        &Arc::clone(&func_name),
    //        move |mut store, params, results| {
    //            let name = Arc::clone(&name);
    //            let func_name = Arc::clone(&func_name);
    //            let instance_name = Arc::clone(&instance_name);
    //            let params_ty = Arc::clone(&params_ty);
    //            let results_ty = Arc::clone(&results_ty);
    //            let exports = Arc::clone(&exports);
    //            Box::new(async move {
    //                let (this, params) = params
    //                    .split_first()
    //                    .context("method receiver parameter missing")?;
    //                let component::Val::Resource(this) = this else {
    //                    bail!("method receiver is not a resource");
    //                };
    //                let res: component::Resource<async_nats::Subject> = this
    //                    .try_into_resource(&mut store)
    //                    .context("failed to obtain custom resource")?;
    //                let subject = if this.owned() {
    //                    let subject = WasiView::table(store.data_mut())
    //                        .delete(res)
    //                        .context("failed to delete resource")?;
    //                    format!("{subject}.{name}")
    //                } else {
    //                    let subject = WasiView::table(store.data_mut())
    //                        .get(&res)
    //                        .context("failed to get resource")?;
    //                    format!("{subject}.{name}")
    //                };
    //                this.resource_drop_async(&mut store)
    //                    .await
    //                    .context("failed to drop subject resource")?;
    //                let nats = Arc::clone(&store.data().client());

    //                let mut payload = BytesMut::new();
    //                let futs = encode_values(
    //                    &mut store,
    //                    &mut payload,
    //                    params.iter().zip(params_ty.iter().skip(1)),
    //                )
    //                .await
    //                .context("failed to encode parameters")?;

    //                // TODO: Protocol/interface negotiation in headers

    //                let nats::Response { payload, conn, .. } =
    //                    nats::connect(&nats, subject, payload.freeze(), None)
    //                        .await
    //                        .context("failed to connect to peer")?;
    //                let (tx, mut root, mut child) = conn.split();
    //                if let Some(tx) = tx {
    //                    tx.sized().flush(&nats).await.context("failed to flush")?;
    //                    let instance = store
    //                        .data()
    //                        .instance()
    //                        .clone()
    //                        .context("instance missing from store")?;
    //                    handle_pollables(&mut store, &instance, &nats, &exports, pollables)
    //                        .await
    //                        .context("failed to handle pollables")?;
    //                }
    //                let ty = store
    //                    .data()
    //                    .component_type()
    //                    .get_import(&instance_name)
    //                    .context("instance import not found")?;
    //                let component::types::ComponentItem::ComponentInstance(ty) = ty else {
    //                    bail!("component import type is not an instance: {ty:?}")
    //                };
    //                let ty = ty
    //                    .get_export(&func_name)
    //                    .context("instance does not export polyfilled function")?;
    //                let component::types::ComponentItem::ComponentFunc(ty) = ty else {
    //                    bail!("instance export is not a function: {ty:?}")
    //                };
    //                receive_values(
    //                    &mut store,
    //                    todo!(),
    //                    payload,
    //                    results,
    //                    &results_ty,
    //                    ty.results(),
    //                )
    //                .await
    //                .context("failed to decode results")?;
    //                Ok(())
    //            })
    //        },
    //    )
    //    .context("failed to polyfill method")
}
