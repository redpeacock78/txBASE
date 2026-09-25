#![cfg(target_arch = "wasm32")]

use crate::edge::{
    AsyncObjectStore, AsyncObjectStoreFuture, AsyncObjectTable, CommitResult, ObjectStoreError,
};
use crate::wasm::WasmQueryStream as CoreQueryStream;
use crate::xbf::{XbfLimits, decode_with_limits, encode};
use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

const HOST_METHODS: [&str; 5] = ["get", "putIfAbsent", "compareAndSwap", "delete", "list"];

#[derive(Clone)]
struct JsObjectStore {
    host: JsValue,
    signal: Option<JsValue>,
}

impl JsObjectStore {
    fn new(host: JsValue) -> Result<Self, ObjectStoreError> {
        if host.is_null() || host.is_undefined() {
            return Err(ObjectStoreError::Invalid(
                "WASM object-store host must be an object".into(),
            ));
        }
        let store = Self { host, signal: None };
        for method in HOST_METHODS {
            store.function(method)?;
        }
        Ok(store)
    }

    fn with_signal(&self, signal: JsValue) -> Self {
        Self {
            host: self.host.clone(),
            signal: Some(signal),
        }
    }

    fn function(&self, method: &str) -> Result<Function, ObjectStoreError> {
        Reflect::get(&self.host, &JsValue::from_str(method))
            .map_err(|_| {
                ObjectStoreError::Invalid(format!(
                    "WASM object-store host method {method} could not be read"
                ))
            })?
            .dyn_into::<Function>()
            .map_err(|_| {
                ObjectStoreError::Invalid(format!(
                    "WASM object-store host method {method} must be a function"
                ))
            })
    }

    fn promise(&self, method: &str, arguments: &[JsValue]) -> Result<Promise, ObjectStoreError> {
        let arguments = Array::from_iter(arguments.iter());
        if let Some(signal) = &self.signal {
            arguments.push(signal);
        }
        let value = self
            .function(method)?
            .apply(&self.host, &arguments)
            .map_err(|error| map_host_error(&error))?;
        value.dyn_into::<Promise>().map_err(|_| {
            ObjectStoreError::Invalid(format!(
                "WASM object-store host method {method} must return a Promise"
            ))
        })
    }
}

impl AsyncObjectStore for JsObjectStore {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        Box::pin(async move {
            let value = JsFuture::from(self.promise("get", &[JsValue::from_str(key)])?)
                .await
                .map_err(|error| map_host_error(&error))?;
            if value.is_null() || value.is_undefined() {
                return Ok(None);
            }
            let bytes = value.dyn_into::<Uint8Array>().map_err(|_| {
                ObjectStoreError::Invalid(
                    "WASM object-store get must resolve to Uint8Array or null".into(),
                )
            })?;
            Ok(Some(bytes.to_vec()))
        })
    }

    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move {
            let bytes = Uint8Array::from(bytes);
            let promise = self.promise("putIfAbsent", &[JsValue::from_str(key), bytes.into()])?;
            JsFuture::from(promise)
                .await
                .map_err(|error| map_host_error(&error))?;
            Ok(())
        })
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move {
            let expected = expected
                .map(Uint8Array::from)
                .map(JsValue::from)
                .unwrap_or(JsValue::NULL);
            let replacement = Uint8Array::from(replacement);
            let promise = self.promise(
                "compareAndSwap",
                &[JsValue::from_str(key), expected, replacement.into()],
            )?;
            JsFuture::from(promise)
                .await
                .map_err(|error| map_host_error(&error))?;
            Ok(())
        })
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move {
            let promise = self.promise("delete", &[JsValue::from_str(key)])?;
            JsFuture::from(promise)
                .await
                .map_err(|error| map_host_error(&error))?;
            Ok(())
        })
    }

    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        Box::pin(async move {
            let value = JsFuture::from(self.promise("list", &[JsValue::from_str(prefix)])?)
                .await
                .map_err(|error| map_host_error(&error))?;
            if !Array::is_array(&value) {
                return Err(ObjectStoreError::Invalid(
                    "WASM object-store list must resolve to an Array".into(),
                ));
            }
            let values = Array::from(&value);
            let mut keys = Vec::with_capacity(values.length() as usize);
            for index in 0..values.length() {
                let value = values.get(index);
                let Some(key) = value.as_string() else {
                    return Err(ObjectStoreError::Invalid(
                        "WASM object-store list keys must be strings".into(),
                    ));
                };
                keys.push(key);
            }
            Ok(keys)
        })
    }
}

#[wasm_bindgen]
pub struct WasmObjectTable {
    table: AsyncObjectTable<JsObjectStore>,
    store: JsObjectStore,
    namespace: String,
}

#[wasm_bindgen]
pub struct WasmObjectQueryStream {
    core: CoreQueryStream,
}

#[wasm_bindgen]
impl WasmObjectTable {
    #[wasm_bindgen(constructor)]
    pub fn new(host: JsValue, namespace: String) -> Result<WasmObjectTable, JsValue> {
        let store = JsObjectStore::new(host).map_err(to_js_error)?;
        let table = AsyncObjectTable::new(store.clone(), namespace.clone()).map_err(to_js_error)?;
        Ok(Self {
            table,
            store,
            namespace,
        })
    }

    pub fn manifest_key(&self) -> String {
        self.table.manifest_key().to_owned()
    }

    pub async fn manifest_json(&self) -> Result<JsValue, JsValue> {
        let manifest = self.table.manifest().await.map_err(to_js_error)?;
        match manifest {
            Some(manifest) => json_string(&manifest),
            None => Ok(JsValue::NULL),
        }
    }

    pub async fn read_xbf(&self) -> Result<JsValue, JsValue> {
        let table = self.table.read().await.map_err(to_js_error)?;
        table
            .map(|table| encode(&table).map(bytes_value).map_err(to_js_error))
            .unwrap_or_else(|| Ok(JsValue::NULL))
    }

    pub async fn read_xbf_at(&self, generation: u64) -> Result<JsValue, JsValue> {
        let table = self.table.read_at(generation).await.map_err(to_js_error)?;
        table
            .map(|table| encode(&table).map(bytes_value).map_err(to_js_error))
            .unwrap_or_else(|| Ok(JsValue::NULL))
    }

    pub async fn query_stream_json(&self, body: &[u8]) -> Result<WasmObjectQueryStream, JsValue> {
        object_query_stream(&self.table, body, None).await
    }

    pub async fn query_stream_json_at(
        &self,
        generation: u64,
        body: &[u8],
    ) -> Result<WasmObjectQueryStream, JsValue> {
        object_query_stream(&self.table, body, Some(generation)).await
    }

    pub async fn query_stream_json_with_signal(
        &self,
        body: &[u8],
        signal: JsValue,
    ) -> Result<WasmObjectQueryStream, JsValue> {
        object_query_stream_with_signal(&self.store, &self.namespace, body, None, signal).await
    }

    pub async fn query_stream_json_at_with_signal(
        &self,
        generation: u64,
        body: &[u8],
        signal: JsValue,
    ) -> Result<WasmObjectQueryStream, JsValue> {
        object_query_stream_with_signal(
            &self.store,
            &self.namespace,
            body,
            Some(generation),
            signal,
        )
        .await
    }

    pub async fn commit_xbf(&self, bytes: &[u8]) -> Result<JsValue, JsValue> {
        let table = decode_with_limits(bytes, &XbfLimits::default()).map_err(to_js_error)?;
        let result = self.table.commit(&table).await.map_err(to_js_error)?;
        let value = match result {
            CommitResult::Committed { generation } => {
                serde_json::json!({"status": "committed", "generation": generation})
            }
            CommitResult::AlreadyCommitted { generation } => {
                serde_json::json!({"status": "already_committed", "generation": generation})
            }
        };
        json_string(&value)
    }

    pub async fn recover(&self) -> Result<u32, JsValue> {
        let recovered = self.table.recover().await.map_err(to_js_error)?;
        u32::try_from(recovered)
            .map_err(|_| to_js_error("WASM object-store recovery count exceeds u32"))
    }

    pub async fn retain_generations(&self, keep_last: usize) -> Result<JsValue, JsValue> {
        let removed = self
            .table
            .retain_generations(keep_last)
            .await
            .map_err(to_js_error)?;
        json_string(&removed)
    }

    pub async fn cleanup_orphans(&self) -> Result<JsValue, JsValue> {
        let removed = self.table.cleanup_orphans().await.map_err(to_js_error)?;
        json_string(&removed)
    }
}

#[wasm_bindgen]
impl WasmObjectQueryStream {
    pub fn next_json(&mut self) -> Result<JsValue, JsValue> {
        self.core
            .next_json()
            .map(|record| {
                record
                    .map(|record| JsValue::from_str(&record))
                    .unwrap_or(JsValue::NULL)
            })
            .map_err(to_js_error)
    }

    pub fn cancel(&mut self) {
        self.core.cancel();
    }
}

// ponytail: keep cancellation Worker-scoped; add generic tokens when another runtime needs them.
async fn object_query_stream(
    table: &AsyncObjectTable<JsObjectStore>,
    body: &[u8],
    generation: Option<u64>,
) -> Result<WasmObjectQueryStream, JsValue> {
    let request = crate::query::parse(body).map_err(to_js_error)?;
    let stream = table
        .query_stream_owned(generation, request)
        .await
        .map_err(to_js_error)?;
    Ok(WasmObjectQueryStream {
        core: CoreQueryStream::new(stream),
    })
}

async fn object_query_stream_with_signal(
    store: &JsObjectStore,
    namespace: &str,
    body: &[u8],
    generation: Option<u64>,
    signal: JsValue,
) -> Result<WasmObjectQueryStream, JsValue> {
    validate_abort_signal(&signal)?;
    let table = AsyncObjectTable::new(store.with_signal(signal), namespace.to_owned())
        .map_err(to_js_error)?;
    object_query_stream(&table, body, generation).await
}

fn validate_abort_signal(signal: &JsValue) -> Result<(), JsValue> {
    let is_function = |name| {
        Reflect::get(signal, &JsValue::from_str(name))
            .ok()
            .is_some_and(|value| value.dyn_ref::<Function>().is_some())
    };
    let valid = !signal.is_null()
        && !signal.is_undefined()
        && Reflect::get(signal, &JsValue::from_str("aborted"))
            .ok()
            .is_some_and(|value| value.as_bool().is_some())
        && is_function("addEventListener")
        && is_function("removeEventListener");
    if valid {
        Ok(())
    } else {
        Err(to_js_error("query stream signal must be an AbortSignal"))
    }
}

fn bytes_value(bytes: Vec<u8>) -> JsValue {
    Uint8Array::from(bytes.as_slice()).into()
}

fn json_string<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    serde_json::to_string(value)
        .map(|value| JsValue::from_str(&value))
        .map_err(to_js_error)
}

fn to_js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn map_host_error(value: &JsValue) -> ObjectStoreError {
    let code = Reflect::get(value, &JsValue::from_str("code"))
        .ok()
        .and_then(|value| value.as_string());
    let message = Reflect::get(value, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "JavaScript host operation rejected".into());
    match code.as_deref() {
        Some("invalid") => ObjectStoreError::Invalid(message),
        Some("conflict") => ObjectStoreError::Conflict(message),
        Some("missing") => ObjectStoreError::Missing(message),
        Some("cancelled") => ObjectStoreError::Cancelled(message),
        _ => ObjectStoreError::Unavailable(message),
    }
}
