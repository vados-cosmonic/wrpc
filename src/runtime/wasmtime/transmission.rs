use core::borrow::Borrow;
use core::cmp::Reverse;
use core::iter::{zip, Sum};
use core::ops::{Add, Deref};
use std::collections::{HashMap, VecDeque};
use std::vec;

use anyhow::{bail, Context as _};
use async_recursion::async_recursion;
use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use futures::StreamExt as _;
use tracing::instrument;
use wasmtime::{component, AsContextMut};
use wasmtime_wasi::preview2::{self, WasiView};
use wrpc_types::Resource;

use crate::runtime::wasmtime::WrpcView;
//use crate::{nats, DynamicResourceType, Receiver, Type, Value};
use crate::{DynamicResourceType, Type, Value};

#[derive(Debug)]
pub struct GuestLocalPollable {
    guest_resource: component::ResourceAny,
    pollable: component::Resource<preview2::Pollable>,
}

#[derive(Debug)]
pub struct GuestResource {
    resource: component::ResourceAny,
    listener: (),
    subject: String,
    ty: DynamicResourceType,
}

#[derive(Debug)]
pub enum GuestAsyncValue {
    Pollable(GuestLocalPollable),
    Resource(GuestResource),
}

#[derive(Debug)]
struct GuestResourceCallPollable {
    listener: (),
    request: Option<anyhow::Result<()>>,
}

#[async_trait]
impl preview2::Subscribe for GuestResourceCallPollable {
    async fn ready(&mut self) {
        if self.request.is_none() {
            //self.request = self.listener.next().await;
            self.request = todo!()
        }
    }
}

#[derive(Debug)]
struct GuestResourcePollable {
    pollable: component::Resource<preview2::Pollable>,
    inner: component::Resource<GuestResourceCallPollable>,
    guest_resource: component::ResourceAny,
    subject: String,
    ty: DynamicResourceType,
}

#[derive(Debug)]
pub enum GuestPollable {
    Local {
        tx: (),
        pollable: GuestLocalPollable,
    },
    Resource(GuestResourcePollable),
}

impl GuestPollable {
    fn pollable(&self) -> &component::Resource<preview2::Pollable> {
        match self {
            Self::Local {
                pollable: GuestLocalPollable { pollable, .. },
                ..
            } => pollable,
            Self::Resource(GuestResourcePollable { pollable, .. }) => pollable,
        }
    }
}
//
//#[derive(Debug, Default)]
//pub struct GuestAsyncValues(Vec<(VecDeque<u32>, GuestAsyncValue)>);
//
//impl Deref for GuestAsyncValues {
//    type Target = Vec<(VecDeque<u32>, GuestAsyncValue)>;
//
//    fn deref(&self) -> &Self::Target {
//        &self.0
//    }
//}
//
//impl IntoIterator for GuestAsyncValues {
//    type Item = (VecDeque<u32>, GuestAsyncValue);
//    type IntoIter = vec::IntoIter<Self::Item>;
//
//    fn into_iter(self) -> Self::IntoIter {
//        self.0.into_iter()
//    }
//}
//
//impl From<GuestAsyncValue> for GuestAsyncValues {
//    fn from(value: GuestAsyncValue) -> Self {
//        Self(vec![(VecDeque::default(), value)])
//    }
//}
//
//impl Extend<(VecDeque<u32>, GuestAsyncValue)> for GuestAsyncValues {
//    fn extend<T: IntoIterator<Item = (VecDeque<u32>, GuestAsyncValue)>>(&mut self, iter: T) {
//        self.0.extend(iter)
//    }
//}
//
//impl Extend<Self> for GuestAsyncValues {
//    fn extend<T: IntoIterator<Item = Self>>(&mut self, iter: T) {
//        for Self(vs) in iter {
//            self.extend(vs)
//        }
//    }
//}
//
//impl Add for GuestAsyncValues {
//    type Output = Self;
//
//    fn add(mut self, Self(vs): Self) -> Self::Output {
//        self.extend(vs);
//        self
//    }
//}
//
//impl Sum for GuestAsyncValues {
//    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
//        iter.reduce(Add::add).unwrap_or_default()
//    }
//}
//
//impl FromIterator<Self> for GuestAsyncValues {
//    fn from_iter<T: IntoIterator<Item = Self>>(iter: T) -> Self {
//        iter.into_iter().sum()
//    }
//}
//
//impl GuestAsyncValues {
//    fn nest_at(&mut self, idx: u32) {
//        for (path, _) in self.0.iter_mut() {
//            path.push_front(idx)
//        }
//    }
//
//    pub fn into_pollables<T: WrpcView>(
//        self,
//        mut store: impl AsContextMut<Data = T> + Send,
//        tx: &nats::Publisher,
//    ) -> anyhow::Result<Vec<GuestPollable>> {
//        let mut store = store.as_context_mut();
//        let mut pollables = Vec::with_capacity(self.0.len());
//        for (path, fut) in self {
//            match fut {
//                GuestAsyncValue::Pollable(pollable) => {
//                    let mut iter = path.iter().map(ToString::to_string);
//                    let seps = path.len().checked_sub(1).context("path cannot be empty")?;
//                    let path_len = iter
//                        .by_ref()
//                        .map(|idx| idx.len())
//                        .sum::<usize>()
//                        .checked_add(seps)
//                        .context("invalid path length")?;
//                    let path = iter.fold(String::with_capacity(path_len), |mut path, idx| {
//                        path.push_str(&idx);
//                        path
//                    });
//                    pollables.push(GuestPollable::Local {
//                        tx: tx.child(&path),
//                        pollable,
//                    });
//                }
//                GuestAsyncValue::Resource(GuestResource {
//                    resource,
//                    listener,
//                    subject,
//                    ty,
//                }) => {
//                    let inner = GuestResourceCallPollable {
//                        request: None,
//                        listener,
//                    };
//                    let inner = WasiView::table(store.data_mut())
//                        .push(inner)
//                        .context("failed to push guest resource call stream resource to table")?;
//                    let pollable = preview2::subscribe(
//                        WasiView::table(store.data_mut()),
//                        component::Resource::<GuestResourceCallPollable>::new_borrow(inner.rep()),
//                    )
//                    .context("failed to subscribe to guest resource call stream")?;
//                    pollables.push(GuestPollable::Resource(GuestResourcePollable {
//                        pollable,
//                        inner,
//                        guest_resource: resource,
//                        subject,
//                        ty,
//                    }))
//                }
//            }
//        }
//        Ok(pollables)
//    }
//}

//#[instrument(level = "trace", skip_all)]
//#[async_recursion]
//pub async fn encode_values<'a, T>(
//    mut store: impl AsContextMut<Data = T> + Send + 'async_recursion,
//    mut payload: &mut BytesMut,
//    iter: impl ExactSizeIterator<Item = (&'a component::Val, &'a Type)> + Send + 'async_recursion,
//) -> anyhow::Result<Vec<GuestPollable>> {
//    let mut pollables = Vec::with_capacity(iter.len());
//    let mut path = vec![];
//    for (i, (val, ty)) in zip(0.., iter) {
//        path.push(i);
//        let v = to_wrpc_value(&mut store, val, ty, &mut pollables)
//            .await
//            .context("failed to construct wRPC value")?;
//        v.encode(&mut payload)
//            .await
//            .context("failed to encode wRPC value")?;
//        path.pop();
//    }
//    Ok(pollables)
//}
