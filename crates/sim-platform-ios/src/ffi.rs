//! Unsafe-isolated native ABI and iOS JNI membrane.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_void};

use sim_kernel::{
    Expr, NativeAbiBorrowedBytes, NativeAbiCallResponse, NativeAbiError, NativeAbiOwnedBytes,
    NativeLibAbiV1, native_abi_owned_bytes,
};

use crate::Capsule;

unsafe extern "C" fn instantiate() -> *mut c_void {
    Box::into_raw(Box::new(Capsule::default())).cast::<c_void>()
}

unsafe extern "C" fn destroy_instance(instance: *mut c_void) {
    if !instance.is_null() {
        unsafe {
            drop(Box::from_raw(instance.cast::<Capsule>()));
        }
    }
}

unsafe extern "C" fn manifest(instance: *mut c_void) -> NativeAbiCallResponse {
    if instance.is_null() {
        return failure("iOS ABI manifest received a null instance");
    }
    match sim_codec_binary::encode_frame(&manifest_expr()) {
        Ok(frame) => success(frame.0),
        Err(error) => failure(error.to_string()),
    }
}

unsafe extern "C" fn call(
    instance: *mut c_void,
    function: *const c_char,
    args: NativeAbiBorrowedBytes,
) -> NativeAbiCallResponse {
    if instance.is_null() {
        return failure("iOS ABI call received a null instance");
    }
    if function.is_null() {
        return failure("iOS ABI call received a null function symbol");
    }
    let function = unsafe { CStr::from_ptr(function) }.to_string_lossy();
    let arg_bytes = if args.ptr.is_null() && args.len == 0 {
        &[][..]
    } else if args.ptr.is_null() {
        return failure("iOS ABI call received null argument bytes");
    } else {
        unsafe { std::slice::from_raw_parts(args.ptr, args.len) }
    };
    let capsule = unsafe { &mut *instance.cast::<Capsule>() };
    match capsule.call_frame(&function, arg_bytes) {
        Ok(bytes) => success(bytes),
        Err(error) => failure(error),
    }
}

unsafe extern "C" fn destroy_bytes(bytes: NativeAbiOwnedBytes) {
    if !bytes.ptr.is_null() {
        unsafe {
            drop(Vec::from_raw_parts(bytes.ptr, bytes.len, bytes.cap));
        }
    }
}

unsafe extern "C" fn destroy_error(error: *mut NativeAbiError) {
    if error.is_null() {
        return;
    }
    let error = unsafe { Box::from_raw(error) };
    if !error.message.is_null() {
        unsafe {
            drop(CString::from_raw(error.message));
        }
    }
}

static ABI: NativeLibAbiV1 = NativeLibAbiV1::new(
    instantiate,
    destroy_instance,
    manifest,
    call,
    destroy_bytes,
    destroy_error,
);

/// Returns the iOS capsule's unchanged SIM native ABI v1 function table.
#[unsafe(no_mangle)]
pub extern "C" fn sim_native_abi_v1() -> *const NativeLibAbiV1 {
    &raw const ABI
}

/// Converts shell-owned typed JSON into the canonical SIM binary input frame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sim_ios_encode_input_json(
    json: NativeAbiBorrowedBytes,
) -> NativeAbiCallResponse {
    let bytes = match unsafe { borrowed_slice(json, "iOS shell JSON") } {
        Ok(bytes) => bytes,
        Err(error) => return failure(error),
    };
    match serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid typed iOS shell input: {error}"))
        .and_then(|input| crate::encode_input_frame(&input))
    {
        Ok(bytes) => success(bytes),
        Err(error) => failure(error),
    }
}

/// Converts one canonical SIM binary output frame into shell-owned JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sim_ios_decode_output_json(
    frame: NativeAbiBorrowedBytes,
) -> NativeAbiCallResponse {
    let bytes = match unsafe { borrowed_slice(frame, "iOS output frame") } {
        Ok(bytes) => bytes,
        Err(error) => return failure(error),
    };
    match crate::decode_output_frame(bytes)
        .and_then(|output| serde_json::to_vec(&output).map_err(|error| error.to_string()))
    {
        Ok(bytes) => success(bytes),
        Err(error) => failure(error),
    }
}

unsafe fn borrowed_slice<'a>(
    bytes: NativeAbiBorrowedBytes,
    label: &str,
) -> Result<&'a [u8], String> {
    if bytes.ptr.is_null() {
        return if bytes.len == 0 {
            Ok(&[])
        } else {
            Err(format!("{label} had a null pointer"))
        };
    }
    Ok(unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) })
}

/// Safe owner for one instance reached exclusively through `NativeLibAbiV1`.
pub struct StaticAbiCapsule {
    instance: *mut c_void,
}

impl StaticAbiCapsule {
    /// Instantiates one capsule through the static native function table.
    ///
    /// # Errors
    /// Returns an error if the ABI entry unexpectedly returns a null instance.
    pub fn new() -> Result<Self, String> {
        let instance = unsafe { (ABI.instantiate)() };
        if instance.is_null() {
            Err("iOS ABI returned a null capsule instance".into())
        } else {
            Ok(Self { instance })
        }
    }

    /// Invokes a named function through `NativeLibAbiV1::call`.
    ///
    /// # Errors
    /// Returns a copied ABI error or rejects an invalid response buffer.
    pub fn call(&mut self, function: &str, frame: &[u8]) -> Result<Vec<u8>, String> {
        let function = CString::new(function)
            .map_err(|_| "iOS ABI function contained an interior NUL".to_owned())?;
        let response = unsafe {
            (ABI.call)(
                self.instance,
                function.as_ptr(),
                NativeAbiBorrowedBytes::borrow(frame),
            )
        };
        unsafe { take_response(&response, "iOS ABI call") }
    }

    /// Fetches the encoded library manifest through the native function table.
    ///
    /// # Errors
    /// Returns a copied ABI error or rejects an invalid response buffer.
    pub fn manifest(&mut self) -> Result<Vec<u8>, String> {
        let response = unsafe { (ABI.manifest)(self.instance) };
        unsafe { take_response(&response, "iOS ABI manifest") }
    }
}

impl Drop for StaticAbiCapsule {
    fn drop(&mut self) {
        unsafe {
            (ABI.destroy_instance)(self.instance);
        }
        self.instance = std::ptr::null_mut();
    }
}

fn success(bytes: Vec<u8>) -> NativeAbiCallResponse {
    NativeAbiCallResponse::success(native_abi_owned_bytes(bytes))
}

fn failure(message: impl Into<String>) -> NativeAbiCallResponse {
    NativeAbiCallResponse::failure(NativeAbiError::boxed(message))
}

unsafe fn take_response(
    response: &NativeAbiCallResponse,
    operation: &str,
) -> Result<Vec<u8>, String> {
    if !response.error.is_null() {
        let message = unsafe {
            let error = &*response.error;
            if error.message.is_null() {
                format!("{operation} failed without an error message")
            } else {
                CStr::from_ptr(error.message).to_string_lossy().into_owned()
            }
        };
        unsafe {
            (ABI.destroy_error)(response.error);
        }
        return Err(message);
    }
    if response.bytes.len == 0 {
        unsafe {
            (ABI.destroy_bytes)(response.bytes);
        }
        return Ok(Vec::new());
    }
    if response.bytes.ptr.is_null() || response.bytes.len > response.bytes.cap {
        return Err(format!("{operation} returned an invalid owned byte buffer"));
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(response.bytes.ptr.cast_const(), response.bytes.len).to_vec()
    };
    unsafe {
        (ABI.destroy_bytes)(response.bytes);
    }
    Ok(bytes)
}

fn manifest_expr() -> Expr {
    Expr::Map(vec![
        entry(
            "id",
            Expr::Symbol(sim_kernel::Symbol::qualified("platform", "ios-capsule")),
        ),
        entry("version", Expr::String(env!("CARGO_PKG_VERSION").into())),
        entry("abi-major", number(0)),
        entry("abi-minor", number(1)),
        entry(
            "target",
            Expr::String(
                sim_kernel::LibTarget::HostRegistered
                    .to_symbol()
                    .as_qualified_str(),
            ),
        ),
        entry(
            "requires",
            Expr::List(vec![Expr::Map(vec![
                entry(
                    "id",
                    Expr::Symbol(sim_kernel::Symbol::qualified("sim", "platform")),
                ),
                entry("minimum-version", Expr::String("0.1.0".into())),
            ])]),
        ),
        entry("capabilities", Expr::List(Vec::new())),
        entry(
            "exports",
            Expr::List(
                [crate::LIFECYCLE_FUNCTION, crate::ACTIVATION_FUNCTION]
                    .into_iter()
                    .map(|symbol| {
                        Expr::Map(vec![
                            entry("kind", Expr::String("function".into())),
                            entry("symbol", Expr::Symbol(symbol_from_qualified(symbol))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn entry(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(sim_kernel::Symbol::new(key)), value)
}

fn number(value: u16) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: sim_kernel::Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn symbol_from_qualified(value: &str) -> sim_kernel::Symbol {
    let (namespace, name) = value
        .split_once('/')
        .expect("iOS ABI symbols are compile-time qualified constants");
    sim_kernel::Symbol::qualified(namespace, name)
}
