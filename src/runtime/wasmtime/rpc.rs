use core::array;
use core::borrow::Borrow;
use core::cmp::Reverse;
use core::iter::zip;
use core::pin::Pin;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::vec;

use anyhow::{bail, Context as _};
use async_recursion::async_recursion;
use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use futures::prelude::future::try_join_all;
use futures::{stream, try_join, Stream, StreamExt as _, TryStreamExt as _};
use tracing::{instrument, trace};
use wasmtime::component::types::{Case, Field};
use wasmtime::component::{
    Enum, Flags, Instance, List, OptionVal, Record, ResourceAny, ResourceType, ResultVal, Tuple,
    Type, Val, Variant,
};
use wasmtime::{component, AsContextMut};
use wasmtime_wasi::preview2::bindings::io::poll::Host as _;
use wasmtime_wasi::preview2::{self, WasiView};
use wrpc_transport::{
    receive_at_least, receive_discriminant, receive_leb128_unsigned, receive_list_header, Acceptor,
    AsyncSubscription, Receive as _, ReceiveContext, Subject as _, Subscriber,
};

//use crate::runtime::wasmtime::transmission::{encode_values, receive_values, GuestPollable};
use crate::runtime::wasmtime::transmission::GuestPollable;
use crate::runtime::wasmtime::WrpcView;
use crate::DynamicFunctionType;

pub struct TupleReceiver<'a, const N: usize, T> {
    rx: T,
    nested: Option<[Option<AsyncSubscription<T>>; N]>,
    types: [&'a Type; N],
    payload: Bytes,
}

pub struct Value(pub Val);

#[async_trait]
impl ReceiveContext<Type> for Value {
    async fn receive_context<T>(
        ty: &Type,
        payload: impl Buf + Send + 'static,
        rx: &mut (impl Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin),
        sub: Option<AsyncSubscription<T>>,
    ) -> anyhow::Result<(Self, Box<dyn Buf + Send>)>
    where
        T: Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin + 'static,
    {
        let (v, payload) = receive_value(payload, rx, sub, ty).await?;
        Ok((Value(v), payload))
    }
}

impl<const N: usize, S> TupleReceiver<'_, N, S>
where
    S: Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin + 'static,
{
    #[instrument(level = "trace", skip_all)]
    pub async fn receive(mut self) -> anyhow::Result<([Val; N], Box<dyn Buf + Send>, S)> {
        trace!("receive tuple");
        let mut vals = Vec::with_capacity(N);
        let mut payload: Box<dyn Buf + Send> = Box::new(self.payload);
        for (i, ty) in self.types.iter().enumerate() {
            trace!(i, "receive tuple element");
            let v;
            (Value(v), payload) = Value::receive_context(
                ty,
                payload,
                &mut self.rx,
                self.nested
                    .as_mut()
                    .map(|nested| nested.get_mut(i).map(Option::take).flatten())
                    .flatten(),
            )
            .await
            .context("failed to receive tuple value")?;
            vals.push(v);
        }
        let vals = if let Ok(vals) = vals.try_into() {
            vals
        } else {
            bail!("invalid value vector received")
        };
        Ok((vals, payload, self.rx))
    }
}

pub struct TupleDynamicReceiver<'a, S> {
    rx: S,
    nested: Vec<Option<AsyncSubscription<S>>>,
    types: &'a [Type],
    payload: Bytes,
}

impl<S> TupleDynamicReceiver<'_, S>
where
    S: Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin + 'static,
{
    #[instrument(level = "trace", skip_all)]
    pub async fn receive(mut self) -> anyhow::Result<(Vec<Val>, Box<dyn Buf + Send>, S)> {
        trace!("receive tuple");
        let mut vals = Vec::with_capacity(self.types.len());
        let mut payload: Box<dyn Buf + Send> = Box::new(self.payload);
        for (i, ty) in self.types.iter().enumerate() {
            trace!(i, "receive tuple element");
            let v;
            (Value(v), payload) = Value::receive_context(
                ty,
                payload,
                &mut self.rx,
                self.nested.get_mut(i).map(Option::take).flatten(),
            )
            .await
            .context("failed to receive tuple value")?;
            vals.push(v);
        }
        Ok((vals, payload, self.rx))
    }
}

pub trait SubscriberExt: Subscriber {
    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_async(
        &self,
        subject: Self::Subject,
        ty: impl Borrow<Type> + Send,
    ) -> Result<Option<AsyncSubscription<Self::Stream>>, Self::SubscribeError> {
        match ty.borrow() {
            Type::Bool
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::S8
            | Type::S16
            | Type::S32
            | Type::S64
            | Type::Float32
            | Type::Float64
            | Type::Char
            | Type::String
            | Type::Enum(..)
            | Type::Flags(..) => Ok(None),
            Type::List(ty) => {
                let subs = self
                    .subscribe_component_async(subject.child(None), ty.ty())
                    .await?;
                Ok(subs.map(Box::new).map(AsyncSubscription::List))
            }
            Type::Record(ty) => {
                let subs = self
                    .subscribe_component_async_iter(
                        &subject,
                        ty.fields().map(|Field { ty, .. }| ty),
                    )
                    .await?;
                Ok(subs.map(AsyncSubscription::Record))
            }
            Type::Tuple(ty) => {
                let subs = self
                    .subscribe_component_async_iter(&subject, ty.types())
                    .await?;
                Ok(subs.map(AsyncSubscription::Tuple))
            }
            Type::Variant(ty) => {
                let subs = self
                    .subscribe_component_async_iter_optional(
                        &subject,
                        ty.cases().map(|Case { ty, .. }| ty),
                    )
                    .await?;
                Ok(subs.map(AsyncSubscription::Variant))
            }
            Type::Option(ty) => {
                let sub = self
                    .subscribe_component_async(subject.child(Some(1)), ty.ty())
                    .await?;
                Ok(sub.map(Box::new).map(AsyncSubscription::Option))
            }
            Type::Result(ty) => {
                let nested = self
                    .subscribe_component_async_array_optional(&subject, [ty.ok(), ty.err()])
                    .await?;
                Ok(nested.map(|[ok, err]| AsyncSubscription::Result {
                    ok: ok.map(Box::new),
                    err: err.map(Box::new),
                }))
            }
            Type::Own(ty) | Type::Borrow(ty) => {
                self.subscribe_component_resource(subject, ty).await
            }
        }
    }

    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_resource(
        &self,
        subject: impl Borrow<Self::Subject> + Send,
        ty: &ResourceType,
    ) -> Result<Option<AsyncSubscription<Self::Stream>>, Self::SubscribeError> {
        todo!()
        //Type::Future(None) => {
        //    let subscriber = self.subscribe(subject).await?;
        //    Ok(Some(AsyncSubscription::Future {
        //        subscriber,
        //        nested: None,
        //    }))
        //}
        //Type::Future(Some(ty)) => {
        //    let nested = subject.child(Some(0));
        //    let (subscriber, nested) = try_join!(
        //        self.subscribe(subject),
        //        self.subscribe_component_async(nested, ty)
        //    )?;
        //    Ok(Some(AsyncSubscription::Future {
        //        subscriber,
        //        nested: nested.map(Box::new),
        //    }))
        //}
        //Type::Stream { element, end } => {
        //    let nested = subject.child(None);
        //    let (subscriber, nested) = try_join!(
        //        self.subscribe(subject),
        //        self.subscribe_component_async_array_optional(
        //            &nested,
        //            [
        //                element.as_ref().map(AsRef::as_ref),
        //                end.as_ref().map(AsRef::as_ref)
        //            ]
        //        )
        //    )?;
        //    let (nested_element, nested_end) = nested
        //        .map(|[nested_element, nested_end]| {
        //            (nested_element.map(Box::new), nested_end.map(Box::new))
        //        })
        //        .unwrap_or_default();
        //    Ok(Some(AsyncSubscription::Stream {
        //        subscriber,
        //        nested_element,
        //        nested_end,
        //    }))
        //}
        //Type::Resource(Resource::Pollable) => {
        //    self.subscribe_component_async(subject, &Type::Future(None))
        //        .await
        //}
        //Type::Resource(Resource::InputStream) => {
        //    self.subscribe_component_async(
        //        subject,
        //        &Type::Stream {
        //            element: Some(Arc::new(Type::U8)),
        //            end: None,
        //        },
        //    )
        //    .await
        //}
        //Type::Resource(Resource::OutputStream | Resource::Dynamic(..)) => Ok(None),
    }

    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_async_iter(
        &self,
        subject: impl Borrow<Self::Subject> + Send,
        types: impl IntoIterator<Item = impl Borrow<Type> + Send> + Send,
    ) -> Result<Option<Vec<Option<AsyncSubscription<Self::Stream>>>>, Self::SubscribeError> {
        let subs =
            try_join_all(zip(0.., types).map(|(i, ty)| {
                self.subscribe_component_async(subject.borrow().child(Some(i)), ty)
            }))
            .await?;
        Ok(subs.iter().any(Option::is_some).then_some(subs))
    }

    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_async_iter_optional(
        &self,
        subject: impl Borrow<Self::Subject> + Send,
        types: impl IntoIterator<Item = Option<impl Borrow<Type> + Send>> + Send,
    ) -> Result<Option<Vec<Option<AsyncSubscription<Self::Stream>>>>, Self::SubscribeError> {
        let subs = try_join_all(zip(0.., types).map(|(i, ty)| {
            let subject = subject.borrow().child(Some(i));
            async {
                if let Some(ty) = ty {
                    self.subscribe_component_async(subject, ty).await
                } else {
                    Ok(None)
                }
            }
        }))
        .await?;
        Ok(subs.iter().any(Option::is_some).then_some(subs))
    }

    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_async_array<'a, const N: usize>(
        &self,
        subject: impl Borrow<Self::Subject> + Send,
        types: [impl Borrow<Type> + Send + Sync; N],
    ) -> Result<Option<[Option<AsyncSubscription<Self::Stream>>; N]>, Self::SubscribeError> {
        self.subscribe_component_async_array_optional(subject, types.map(Some))
            .await
    }

    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_async_array_optional<'a, const N: usize>(
        &self,
        subject: impl Borrow<Self::Subject> + Send,
        types: [Option<impl Borrow<Type> + Send + Sync>; N],
    ) -> Result<Option<[Option<AsyncSubscription<Self::Stream>>; N]>, Self::SubscribeError> {
        match types.as_slice() {
            [] | [None] | [None, None] | [None, None, None] | [None, None, None, None] => Ok(None),
            [Some(a)] | [Some(a), None] | [Some(a), None, None] | [Some(a), None, None, None] => {
                let mut sub = self
                    .subscribe_component_async(subject.borrow().child(Some(0)), a.borrow())
                    .await?;
                if sub.is_some() {
                    Ok(Some(array::from_fn(
                        |i| if i == 0 { sub.take() } else { None },
                    )))
                } else {
                    Ok(None)
                }
            }
            [None, Some(b)] | [None, Some(b), None] | [None, Some(b), None, None] => {
                let mut sub = self
                    .subscribe_component_async(subject.borrow().child(Some(1)), b.borrow())
                    .await?;
                if sub.is_some() {
                    Ok(Some(array::from_fn(
                        |i| if i == 1 { sub.take() } else { None },
                    )))
                } else {
                    Ok(None)
                }
            }
            [None, None, Some(c)] | [None, None, Some(c), None] => {
                let mut sub = self
                    .subscribe_component_async(subject.borrow().child(Some(2)), c.borrow())
                    .await?;
                if sub.is_some() {
                    Ok(Some(array::from_fn(
                        |i| if i == 2 { sub.take() } else { None },
                    )))
                } else {
                    Ok(None)
                }
            }
            [None, None, None, Some(d)] => {
                let mut sub = self
                    .subscribe_component_async(subject.borrow().child(Some(3)), d.borrow())
                    .await?;
                if sub.is_some() {
                    Ok(Some(array::from_fn(
                        |i| if i == 3 { sub.take() } else { None },
                    )))
                } else {
                    Ok(None)
                }
            }
            [Some(a), Some(b)] | [Some(a), Some(b), None] | [Some(a), Some(b), None, None] => {
                let (mut a, mut b) = try_join!(
                    self.subscribe_component_async(subject.borrow().child(Some(0)), a.borrow()),
                    self.subscribe_component_async(subject.borrow().child(Some(1)), b.borrow()),
                )?;
                if a.is_some() || b.is_some() {
                    Ok(Some(array::from_fn(|i| match i {
                        0 => a.take(),
                        1 => b.take(),
                        _ => None,
                    })))
                } else {
                    Ok(None)
                }
            }
            [Some(a), Some(b), Some(c)] | [Some(a), Some(b), Some(c), None] => {
                let (mut a, mut b, mut c) = try_join!(
                    self.subscribe_component_async(subject.borrow().child(Some(0)), a.borrow()),
                    self.subscribe_component_async(subject.borrow().child(Some(1)), b.borrow()),
                    self.subscribe_component_async(subject.borrow().child(Some(2)), c.borrow()),
                )?;
                if a.is_some() || b.is_some() || c.is_some() {
                    Ok(Some(array::from_fn(|i| match i {
                        0 => a.take(),
                        1 => b.take(),
                        2 => c.take(),
                        _ => None,
                    })))
                } else {
                    Ok(None)
                }
            }
            [Some(a), Some(b), Some(c), Some(d)] => {
                let (mut a, mut b, mut c, mut d) = try_join!(
                    self.subscribe_component_async(subject.borrow().child(Some(0)), a.borrow()),
                    self.subscribe_component_async(subject.borrow().child(Some(1)), b.borrow()),
                    self.subscribe_component_async(subject.borrow().child(Some(2)), c.borrow()),
                    self.subscribe_component_async(subject.borrow().child(Some(3)), d.borrow()),
                )?;
                if a.is_some() || b.is_some() || c.is_some() || d.is_some() {
                    Ok(Some(array::from_fn(|i| match i {
                        0 => a.take(),
                        1 => b.take(),
                        2 => c.take(),
                        3 => d.take(),
                        _ => None,
                    })))
                } else {
                    Ok(None)
                }
            }
            _ => match self
                .subscribe_component_async_iter_optional(subject, types)
                .await?
            {
                Some(subs) => match subs.try_into() {
                    Ok(subs) => Ok(Some(subs)),
                    Err(_) => panic!("invalid subscription vector generated"),
                },
                None => Ok(None),
            },
        }
    }

    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_tuple<'a, const N: usize>(
        &self,
        subject: Self::Subject,
        types: [&'a Type; N],
        payload: Bytes,
    ) -> Result<TupleReceiver<'a, N, Self::Stream>, Self::SubscribeError> {
        let nested = self.subscribe_component_async_array(subject.clone(), types);
        let rx = self.subscribe(subject);
        let (rx, nested) = try_join!(rx, nested)?;
        Ok(TupleReceiver {
            rx,
            nested,
            types,
            payload,
        })
    }

    #[instrument(level = "trace", skip_all)]
    async fn subscribe_component_tuple_dynamic<'a>(
        &self,
        subject: Self::Subject,
        types: &'a [Type],
        payload: Bytes,
    ) -> Result<TupleDynamicReceiver<'a, Self::Stream>, Self::SubscribeError> {
        let nested = self.subscribe_component_async_iter(subject.clone(), types);
        let rx = self.subscribe(subject);
        let (rx, nested) = try_join!(rx, nested)?;
        Ok(TupleDynamicReceiver {
            rx,
            nested: nested.unwrap_or_default(),
            types,
            payload,
        })
    }
}

impl<T: Subscriber> SubscriberExt for T {}

/// Receive a dynamically-typed list
#[instrument(level = "trace", skip_all)]
async fn receive_list<T>(
    payload: impl Buf + Send + 'static,
    rx: &mut (impl Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin),
    sub: Option<AsyncSubscription<T>>,
    ty: &Type,
) -> anyhow::Result<(Vec<Val>, Box<dyn Buf + Send>)>
where
    T: Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin + 'static,
{
    let mut sub = sub.map(AsyncSubscription::try_unwrap_list).transpose()?;
    let (len, mut payload) = receive_list_header(payload, rx).await?;
    trace!(len, "decode list");
    let cap = len
        .try_into()
        .context("list length does not fit in usize")?;
    let mut els = Vec::with_capacity(cap);
    for i in 0..len {
        trace!(i, "decode list element");
        let sub = sub.as_mut().map(|sub| sub.select(i.into()));
        let el;
        (el, payload) = receive_value(payload, rx, sub, ty)
            .await
            .with_context(|| format!("failed to decode value of list element {i}"))?;
        els.push(el);
    }
    Ok((els, payload))
}

#[instrument(level = "trace", skip_all)]
#[async_recursion]
pub async fn receive_resource<T>(
    payload: impl Buf + Send + 'static,
    rx: &mut (impl Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin),
    sub: Option<AsyncSubscription<T>>,
    ty: &ResourceType,
) -> anyhow::Result<(ResourceAny, Box<dyn Buf + Send>)>
where
    T: Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin + 'static,
{
    todo!()
    //if *ty == ResourceType::host::<>() {}
    //match ty {
    //    Type::Own(ty) | Type::Borrow(ty) => {
    //        trace!("decode flags");
    //        let (v, payload) = receive_leb128_unsigned(payload, rx)
    //            .await
    //            .context("failed to decode flags")?;
    //        // NOTE: Currently flags are limited to 64
    //        let mut names = Vec::with_capacity(64);
    //        for (i, name) in zip(0..64, ty.names()) {
    //            if v & (1 << i) != 0 {
    //                names.push(name)
    //            }
    //        }
    //        let v = component::Flags::new(ty, &names).map(Val::Flags)?;
    //        Ok((v, payload))
    //    }
    //    Type::Resource(ty) => {
    //        let Some((mut subscriber, nested)) =
    //            sub.map(AsyncSubscription::try_unwrap_future).transpose()?
    //        else {
    //            bail!("future subscription type mismatch")
    //        };
    //        trace!("decode future");
    //        let mut payload = receive_at_least(payload, rx, 1).await?;
    //        trace!("decode future variant");
    //        match (payload.get_u8(), ty.as_ref()) {
    //            (0, None) => {
    //                trace!("decoded pending unit future");
    //                Ok((
    //                    Self::Future(Box::pin(async move {
    //                        trace!("decode future unit value");
    //                        let buf = subscriber
    //                            .try_next()
    //                            .await
    //                            .context("failed to receive future value")?
    //                            .context("future stream unexpectedly closed")?;
    //                        ensure!(buf.is_empty());
    //                        Ok(None)
    //                    })),
    //                    payload,
    //                ))
    //            }
    //            (0, Some(ty)) => {
    //                let ty = Arc::clone(ty);
    //                trace!("decoded pending future");
    //                Ok((
    //                    Self::Future(Box::pin(async move {
    //                        trace!("decode future nested value");
    //                        let (v, _) =
    //                            receive_value(Bytes::default(), &mut subscriber, nested, &ty)
    //                                .await?;
    //                        Ok(Some(v))
    //                    })),
    //                    payload,
    //                ))
    //            }
    //            (1, None) => {
    //                trace!("decoded ready unit future");
    //                Ok((Self::Future(Box::pin(ready(Ok(None)))), payload))
    //            }
    //            (1, Some(ty)) => {
    //                trace!("decode ready future nested value");
    //                let (v, payload) = receive_value(payload, rx, nested, ty).await?;
    //                Ok((Self::Future(Box::pin(ready(Ok(Some(v))))), payload))
    //            }
    //            _ => bail!("invalid `future` variant"),
    //        }
    //    }
    //    Type::Stream { element, end } => {
    //        let Some((mut subscriber, mut nested_element, mut nested_end)) =
    //            sub.map(AsyncSubscription::try_unwrap_stream).transpose()?
    //        else {
    //            bail!("stream subscription type mismatch")
    //        };
    //        trace!("decode stream");
    //        let mut payload = receive_at_least(payload, rx, 1).await?;
    //        trace!(i = 0, "decode stream item variant");
    //        let byte = payload.copy_to_bytes(1);
    //        match byte.first().unwrap() {
    //            0 => {
    //                let (items_tx, items_rx) = mpsc::channel(1);
    //                let element = element.as_ref().map(Arc::clone);
    //                let end = end.as_ref().map(Arc::clone);
    //                let producer = spawn(async move {
    //                    let mut payload: Box<dyn Buf + Send> = Box::new(Bytes::new());
    //                    for i in 0.. {
    //                        match receive_stream_item(
    //                            payload,
    //                            &mut subscriber,
    //                            nested_element.as_mut().map(|sub| sub.select(i)),
    //                            &mut nested_end,
    //                            element.as_deref(),
    //                            end.as_deref(),
    //                        )
    //                        .await
    //                        {
    //                            Ok((StreamItem::Element(element), buf)) => {
    //                                payload = buf;

    //                                if let Err(err) =
    //                                    items_tx.send(Ok(StreamItem::Element(element))).await
    //                                {
    //                                    trace!(?err, "item receiver closed");
    //                                    return;
    //                                }
    //                            }
    //                            Ok((StreamItem::End(end), _)) => {
    //                                if let Err(err) = items_tx.send(Ok(StreamItem::End(end))).await
    //                                {
    //                                    trace!(?err, "item receiver closed");
    //                                    return;
    //                                }
    //                                return;
    //                            }
    //                            Err(err) => {
    //                                trace!(?err, "stream producer encountered error");
    //                                if let Err(err) = items_tx.send(Err(err)).await {
    //                                    trace!(?err, "item receiver closed");
    //                                }
    //                                return;
    //                            }
    //                        }
    //                    }
    //                });
    //                Ok((
    //                    Self::Stream(Box::pin(StreamValue {
    //                        producer,
    //                        items: ReceiverStream::new(items_rx),
    //                    })),
    //                    payload,
    //                ))
    //            }
    //            1 => {
    //                let (element, payload) = if let Some(element) = element {
    //                    trace!(i = 0, "decode stream element");
    //                    let sub = nested_element.as_mut().map(|sub| sub.select(0));
    //                    let (v, payload) = receive_value(payload, rx, sub, element)
    //                        .await
    //                        .context("failed to decode value of stream element 0")?;
    //                    (Some(v), payload)
    //                } else {
    //                    (None, payload)
    //                };
    //                let mut payload = receive_at_least(payload, rx, 1).await?;
    //                trace!(i = 1, "decode stream item variant");
    //                ensure!(payload.get_u8() == 0);
    //                let (end, payload) = if let Some(end) = end {
    //                    trace!("decode stream end");
    //                    let (v, payload) = receive_value(payload, rx, nested_end, end).await?;
    //                    (Some(v), payload)
    //                } else {
    //                    (None, payload)
    //                };
    //                Ok((
    //                    Value::Stream(Box::pin(stream::iter([
    //                        Ok(StreamItem::Element(element)),
    //                        Ok(StreamItem::End(end)),
    //                    ]))),
    //                    payload,
    //                ))
    //            }
    //            _ => {
    //                trace!("decode stream length");
    //                let (len, mut payload) = receive_leb128_unsigned(byte.chain(payload), rx)
    //                    .await
    //                    .context("failed to decode stream length")?;
    //                trace!(len, "decode stream elements");
    //                let els = if let Some(element) = element {
    //                    let cap = len
    //                        .try_into()
    //                        .context("stream element length does not fit in usize")?;
    //                    let mut els = Vec::with_capacity(cap);
    //                    for i in 0..len {
    //                        trace!(i, "decode stream element");
    //                        let sub = nested_element.as_mut().map(|sub| sub.select(i));
    //                        let el;
    //                        (el, payload) = Value::receive(payload, rx, sub, element)
    //                            .await
    //                            .with_context(|| {
    //                                format!("failed to decode value of list element {i}")
    //                            })?;
    //                        els.push(Ok(StreamItem::Element(Some(el))));
    //                    }
    //                    els
    //                } else {
    //                    Vec::default()
    //                };
    //                ensure!(payload.get_u8() == 0);
    //                let (end, payload) = if let Some(end) = end {
    //                    let (v, payload) = receive_value(payload, rx, nested_end, end).await?;
    //                    (Some(v), payload)
    //                } else {
    //                    (None, payload)
    //                };
    //                Ok((
    //                    Value::Stream(Box::pin(stream::iter(
    //                        els.into_iter().chain([Ok(StreamItem::End(end))]),
    //                    ))),
    //                    payload,
    //                ))
    //            }
    //        }
    //    }
    //    Type::Resource(Resource::Pollable) => {
    //        receive_value(payload, rx, sub, &Type::Future(None)).await
    //    }
    //    Type::Resource(Resource::InputStream) => {
    //        receive_value(
    //            payload,
    //            rx,
    //            sub,
    //            &Type::Stream {
    //                element: Some(Arc::new(Type::U8)),
    //                end: None,
    //            },
    //        )
    //        .await
    //    }
    //    Type::Resource(Resource::OutputStream | Resource::Dynamic(..)) => {
    //        receive_value(payload, rx, sub, &Type::String)
    //            .await
    //            .context("failed to decode resource identifer")
    //    }
    //}
}

#[instrument(level = "trace", skip_all)]
#[async_recursion]
pub async fn receive_value<T>(
    payload: impl Buf + Send + 'static,
    rx: &mut (impl Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin),
    sub: Option<AsyncSubscription<T>>,
    ty: &Type,
) -> anyhow::Result<(Val, Box<dyn Buf + Send>)>
where
    T: Stream<Item = anyhow::Result<Bytes>> + Send + Sync + Unpin + 'static,
{
    match ty {
        Type::Bool => {
            let (v, payload) = bool::receive(payload, rx, sub).await?;
            Ok((Val::Bool(v), payload))
        }
        Type::U8 => {
            let (v, payload) = u8::receive(payload, rx, sub).await?;
            Ok((Val::U8(v), payload))
        }
        Type::U16 => {
            let (v, payload) = u16::receive(payload, rx, sub).await?;
            Ok((Val::U16(v), payload))
        }
        Type::U32 => {
            let (v, payload) = u32::receive(payload, rx, sub).await?;
            Ok((Val::U32(v), payload))
        }
        Type::U64 => {
            let (v, payload) = u64::receive(payload, rx, sub).await?;
            Ok((Val::U64(v), payload))
        }
        Type::S8 => {
            let (v, payload) = i8::receive(payload, rx, sub).await?;
            Ok((Val::S8(v), payload))
        }
        Type::S16 => {
            let (v, payload) = i16::receive(payload, rx, sub).await?;
            Ok((Val::S16(v), payload))
        }
        Type::S32 => {
            let (v, payload) = i32::receive(payload, rx, sub).await?;
            Ok((Val::S32(v), payload))
        }
        Type::S64 => {
            let (v, payload) = i64::receive(payload, rx, sub).await?;
            Ok((Val::S64(v), payload))
        }
        Type::Float32 => {
            let (v, payload) = f32::receive(payload, rx, sub).await?;
            Ok((Val::Float32(v), payload))
        }
        Type::Float64 => {
            let (v, payload) = f64::receive(payload, rx, sub).await?;
            Ok((Val::Float64(v), payload))
        }
        Type::Char => {
            let (v, payload) = char::receive(payload, rx, sub).await?;
            Ok((Val::Char(v), payload))
        }
        Type::String => {
            let (v, payload) = String::receive(payload, rx, sub).await?;
            Ok((Val::String(v.into()), payload))
        }
        Type::List(ty) => {
            let (els, payload) = receive_list(payload, rx, sub, &ty.ty()).await?;
            let v = List::new(ty, els.into()).map(Val::List)?;
            Ok((v, payload))
        }
        Type::Record(ty) => {
            let field_ty = ty.fields();
            let mut fields = Vec::with_capacity(field_ty.len());
            let mut sub = sub.map(AsyncSubscription::try_unwrap_record).transpose()?;
            let mut payload: Box<dyn Buf + Send> = Box::new(payload);
            for (i, Field { name, ty }) in zip(0.., field_ty) {
                trace!(i, "decode record field");
                let sub = sub
                    .as_mut()
                    .and_then(|sub| sub.get_mut(i))
                    .and_then(Option::take);
                let el;
                (el, payload) = receive_value(payload, rx, sub, &ty)
                    .await
                    .with_context(|| format!("failed to decode value of record field {i}"))?;
                fields.push((name, el));
            }
            let v = Record::new(ty, fields).map(Val::Record)?;
            Ok((v, payload))
        }
        Type::Tuple(ty) => {
            let types = ty.types();
            let mut els = Vec::with_capacity(types.len());
            let mut sub = sub.map(AsyncSubscription::try_unwrap_tuple).transpose()?;
            let mut payload: Box<dyn Buf + Send> = Box::new(payload);
            for (i, ty) in zip(0.., types) {
                trace!(i, "decode tuple element");
                let sub = sub
                    .as_mut()
                    .and_then(|sub| sub.get_mut(i))
                    .and_then(Option::take);
                let el;
                (el, payload) = receive_value(payload, rx, sub, &ty)
                    .await
                    .with_context(|| format!("failed to decode value of tuple element {i}"))?;
                els.push(el);
            }
            let v = Tuple::new(ty, els.into()).map(Val::Tuple)?;
            return Ok((v, payload));
        }
        Type::Variant(ty) => {
            trace!("decode variant discriminant");
            let (discriminant, mut payload) = receive_discriminant(payload, rx)
                .await
                .context("failed to decode variant discriminant")?;
            let i: usize = discriminant
                .try_into()
                .context("variant discriminant does not fit in usize")?;
            let Case { name, ty: case_ty } = ty
                .cases()
                .nth(i)
                .context("variant discriminant is unknown")?;
            let sub = sub
                .map(|sub| {
                    let mut sub = sub.try_unwrap_variant()?;
                    anyhow::Ok(sub.get_mut(i).and_then(Option::take))
                })
                .transpose()?
                .flatten();
            let v = if let Some(ty) = case_ty {
                trace!(discriminant, "decode variant value");
                let v;
                (v, payload) = receive_value(payload, rx, sub, &ty)
                    .await
                    .context("failed to decode variant value")?;
                Some(v)
            } else {
                None
            };
            let v = Variant::new(ty, name, v).map(Val::Variant)?;
            Ok((v, payload))
        }
        Type::Enum(ty) => {
            trace!("decode enum discriminant");
            let (discriminant, payload) = receive_discriminant(payload, rx)
                .await
                .context("failed to decode enum discriminant")?;
            let i: usize = discriminant
                .try_into()
                .context("enum discriminant does not fit in usize")?;
            let name = ty.names().nth(i).context("enum discriminant is unknown")?;
            let v = Enum::new(ty, name).map(Val::Enum)?;
            Ok((v, payload))
        }
        Type::Option(ty) => {
            let mut payload = receive_at_least(payload, rx, 1).await?;
            trace!("decode option variant");
            let v = match payload.get_u8() {
                0 => None,
                1 => {
                    let sub = sub.map(AsyncSubscription::try_unwrap_option).transpose()?;
                    trace!("decode option value");
                    let v;
                    (v, payload) = receive_value(payload, rx, sub, &ty.ty())
                        .await
                        .context("failed to decode `option::some` value")?;
                    Some(v)
                }
                _ => bail!("invalid `option` variant"),
            };
            let v = OptionVal::new(ty, v).map(Val::Option)?;
            Ok((v, payload))
        }
        Type::Result(ty) => {
            let sub = sub.map(AsyncSubscription::try_unwrap_result).transpose()?;
            let mut payload = receive_at_least(payload, rx, 1).await?;
            trace!("decode result variant");
            let v = match (payload.get_u8(), ty.ok(), ty.err()) {
                (0, None, _) => Ok(None),
                (0, Some(ty), _) => {
                    trace!("decode `result::ok` value");
                    let v;
                    (v, payload) = receive_value(payload, rx, sub.and_then(|(ok, _)| ok), &ty)
                        .await
                        .context("failed to decode `result::ok` value")?;
                    Ok(Some(v))
                }
                (1, _, None) => Err(None),
                (1, _, Some(ty)) => {
                    trace!("decode `result::err` value");
                    let v;
                    (v, payload) = receive_value(payload, rx, sub.and_then(|(_, err)| err), &ty)
                        .await
                        .context("failed to decode `result::err` value")?;
                    Err(Some(v))
                }
                _ => bail!("invalid `result` variant"),
            };
            let v = ResultVal::new(ty, v).map(Val::Result)?;
            Ok((v, payload))
        }
        Type::Flags(ty) => {
            trace!("decode flags");
            let (v, payload) = receive_leb128_unsigned(payload, rx)
                .await
                .context("failed to decode flags")?;
            // NOTE: Currently flags are limited to 64
            let mut names = Vec::with_capacity(64);
            for (i, name) in zip(0..64, ty.names()) {
                if v & (1 << i) != 0 {
                    names.push(name)
                }
            }
            let v = Flags::new(ty, &names).map(Val::Flags)?;
            Ok((v, payload))
        }
        Type::Own(ty) | Type::Borrow(ty) => {
            let (v, payload) = receive_resource(payload, rx, sub, ty)
                .await
                .context("failed to decode resource value")?;
            Ok((Val::Resource(v), payload))
        }
    }
}

#[async_trait]
pub trait ClientExt: wrpc_transport::Client {
    async fn serve_component_dynamic(
        &self,
        instance: &str,
        name: &str,
        params: Arc<[Type]>,
    ) -> anyhow::Result<
        Pin<
            Box<
                dyn Stream<
                        Item = anyhow::Result<(
                            Vec<Val>,
                            Self::Subject,
                            <Self::Acceptor as Acceptor>::Transmitter,
                        )>,
                    > + Send,
            >,
        >,
    > {
        let invocations = self.serve(instance, name).await?;
        Ok(Box::pin(invocations.and_then({
            move |(payload, rx, sub, accept)| {
                let params = Arc::clone(&params);
                async move {
                    let sub = sub
                        .subscribe_component_tuple_dynamic(rx.clone(), &params, payload)
                        .await
                        .context("failed to subscribe for parameters")?;
                    let (tx_subject, tx) = accept
                        .accept(rx)
                        .await
                        .context("failed to accept invocation")?;
                    let (params, _, _) = sub
                        .receive()
                        .await
                        .context("failed to receive parameters")?;
                    Ok((params, tx_subject, tx))
                }
            }
        })))
    }
}

impl<T: wrpc_transport::Client> ClientExt for T {}

#[instrument(level = "trace", skip_all, ret)]
pub async fn handle_pollables<T: Send + WrpcView>(
    mut store: impl AsContextMut<Data = T> + Send,
    instance: &Instance,
    client: &Arc<impl wrpc_transport::Client>,
    exports: &HashMap<String, HashMap<String, DynamicFunctionType>>,
    mut pollables: Vec<GuestPollable>,
) -> anyhow::Result<()> {
    bail!("not supported yet")
    //let mut store = store.as_context_mut();
    //while !pollables.is_empty() {
    //    let mut ready = store
    //        .data_mut()
    //        .poll(
    //            pollables
    //                .iter()
    //                .map(|pollable| component::Resource::new_borrow(pollable.pollable().rep()))
    //                .collect(),
    //        )
    //        .await
    //        .context("failed to await pollables")?;
    //    // Iterate in reverse order to not break the mapping when removing/pushing entries
    //    ready.sort_by_key(|idx| Reverse(*idx));
    //    for idx in ready {
    //        let idx = idx.try_into().context("invalid ready list index")?;
    //        match pollables.remove(idx) {
    //            GuestPollable::Local {
    //                mut tx,
    //                pollable: GuestLocalPollable { guest_resource, .. },
    //            } => {
    //                guest_resource
    //                    .resource_drop_async(&mut store)
    //                    .await
    //                    .context("failed to drop guest resource")?;
    //                *tx.buffer_mut() = Bytes::new();
    //                tx.sized()
    //                    .flush(&nats)
    //                    .await
    //                    .context("failed to flush pollable")?;
    //            }
    //            GuestPollable::Resource(resource) => {
    //                let GuestResourceCallPollable { request, .. } =
    //                    WasiView::table(store.data_mut())
    //                        .get_mut(&resource.inner)
    //                        .context("failed to get guest resource call stream")?;
    //                let nats::Request {
    //                    payload,
    //                    subject,
    //                    conn,
    //                    ..
    //                } = request
    //                    .take()
    //                    .context("invalid poll state")?
    //                    .context("failed to handle resource method call request")?;
    //                let name = subject
    //                    .strip_prefix(&resource.subject)
    //                    .context("failed to strip prefix")?;
    //                let (_, name) = name
    //                    .split_once('.')
    //                    .context("subject missing a dot separator")?;
    //                if name == "drop" {
    //                    error!("TODO: drop resource pollables");
    //                    resource
    //                        .guest_resource
    //                        .resource_drop_async(&mut store)
    //                        .await
    //                        .context("failed to drop guest resource")?;
    //                    continue;
    //                }

    //                let name = format!("{}.{name}", resource.ty.name);
    //                let Some(DynamicFunctionType::Method {
    //                    receiver: receiver_ty,
    //                    params: params_ty,
    //                    results: results_ty,
    //                }) = exports
    //                    .get(&resource.ty.instance)
    //                    .and_then(|functions| functions.get(&name))
    //                else {
    //                    bail!("export type is not a method")
    //                };
    //                ensure!(*receiver_ty == resource.ty, "method receiver type mismatch");

    //                let func = {
    //                    let mut exports = instance.exports(&mut store);
    //                    let mut interface =
    //                        exports.instance(&resource.ty.instance).with_context(|| {
    //                            format!("instance of `{}` not found", resource.ty.instance)
    //                        })?;
    //                    interface
    //                        .func(&format!("[method]{name}"))
    //                        .with_context(|| format!("function `{name}` not found"))?
    //                };
    //                let conn = conn
    //                    .connect(&nats, Bytes::new(), None)
    //                    .await
    //                    .context("failed to connect to peer")?;
    //                let (tx, mut root, mut child) = conn.split();
    //                let mut params = vec![Val::Bool(false); params_ty.len() + 1];
    //                let (receiver, vals) = params
    //                    .split_first_mut()
    //                    .expect("invalid parameter vector generated");
    //                let wasmtime_tys = func.params(&store).clone();
    //                receive_values(
    //                    &mut store,
    //                    &mut root,
    //                    //&mut child,
    //                    payload,
    //                    vals,
    //                    params_ty,
    //                    wasmtime_tys.into_iter(),
    //                )
    //                .await
    //                .context("failed to decode results")?;
    //                *receiver = Val::Resource(resource.guest_resource);
    //                let mut results = vec![Val::Bool(false); results_ty.len()];
    //                func.call_async(&mut store, &params, &mut results)
    //                    .await
    //                    .context("failed to call async")?;
    //                let mut payload = BytesMut::new();
    //                let pollables = encode_values(
    //                    &mut store,
    //                    &mut payload,
    //                    results.iter().zip(results_ty.as_ref()).map(Into::into),
    //                )
    //                .await
    //                .context("failed to encode results")?;
    //                let mut tx = tx.expect("reply missing for accepted request");
    //                *tx.buffer_mut() = payload.freeze();
    //                tx.sized().flush(&nats).await.context("failed to flush")?;
    //                func.post_return_async(&mut store)
    //                    .await
    //                    .context("failed to perform post-return cleanup")?;

    //                // Push pollable back
    //                pollables.push(GuestPollable::Resource(resource));
    //                pollables.extend(resource_pollables);
    //            }
    //        }
    //    }
    //}
    //Ok(())
}
