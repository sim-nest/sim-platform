//! Unsafe-isolated native ABI and Android JNI membrane.

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
        return failure("Android ABI manifest received a null instance");
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
        return failure("Android ABI call received a null instance");
    }
    if function.is_null() {
        return failure("Android ABI call received a null function symbol");
    }
    let function = unsafe { CStr::from_ptr(function) }.to_string_lossy();
    let arg_bytes = if args.ptr.is_null() && args.len == 0 {
        &[][..]
    } else if args.ptr.is_null() {
        return failure("Android ABI call received null argument bytes");
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

/// Returns the Android capsule's unchanged SIM native ABI v1 function table.
#[unsafe(no_mangle)]
pub extern "C" fn sim_native_abi_v1() -> *const NativeLibAbiV1 {
    &raw const ABI
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
            Err("Android ABI returned a null capsule instance".into())
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
            .map_err(|_| "Android ABI function contained an interior NUL".to_owned())?;
        let response = unsafe {
            (ABI.call)(
                self.instance,
                function.as_ptr(),
                NativeAbiBorrowedBytes::borrow(frame),
            )
        };
        unsafe { take_response(&response, "Android ABI call") }
    }

    /// Fetches the encoded library manifest through the native function table.
    ///
    /// # Errors
    /// Returns a copied ABI error or rejects an invalid response buffer.
    pub fn manifest(&mut self) -> Result<Vec<u8>, String> {
        let response = unsafe { (ABI.manifest)(self.instance) };
        unsafe { take_response(&response, "Android ABI manifest") }
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
            Expr::Symbol(sim_kernel::Symbol::qualified("platform", "android-capsule")),
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
        .expect("Android ABI symbols are compile-time qualified constants");
    sim_kernel::Symbol::qualified(namespace, name)
}

#[cfg(target_os = "android")]
mod android_jni {
    use std::ffi::CString;

    use jni_sys::{JNIEnv, jbyteArray, jint, jlong, jobject};

    use super::StaticAbiCapsule;

    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn Java_org_simnest_shell_SimActivity_nativeInstantiate(
        env: *mut JNIEnv,
        _activity: jobject,
    ) -> jlong {
        match StaticAbiCapsule::new() {
            Ok(capsule) => Box::into_raw(Box::new(capsule)) as jlong,
            Err(error) => {
                unsafe {
                    throw(env, "java/lang/IllegalStateException", &error);
                }
                0
            }
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn Java_org_simnest_shell_SimActivity_nativeDestroy(
        _env: *mut JNIEnv,
        _activity: jobject,
        handle: jlong,
    ) {
        if handle != 0 {
            unsafe {
                drop(Box::from_raw(handle as *mut StaticAbiCapsule));
            }
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn Java_org_simnest_shell_SimActivity_nativeCall(
        env: *mut JNIEnv,
        _activity: jobject,
        handle: jlong,
        function: jint,
        json_frame: jbyteArray,
    ) -> jbyteArray {
        let result = (|| {
            if handle == 0 {
                return Err("Android shell used a closed capsule handle".to_owned());
            }
            let function = match function {
                0 => crate::LIFECYCLE_FUNCTION,
                1 => crate::ACTIVATION_FUNCTION,
                _ => return Err(format!("unknown Android shell function code {function}")),
            };
            let json = unsafe { read_byte_array(env, json_frame)? };
            let input: crate::Input = serde_json::from_slice(&json)
                .map_err(|error| format!("invalid typed Android shell input: {error}"))?;
            let frame = crate::encode_input_frame(&input)?;
            let capsule = unsafe { &mut *(handle as *mut StaticAbiCapsule) };
            let output = crate::decode_output_frame(&capsule.call(function, &frame)?)?;
            serde_json::to_vec(&output).map_err(|error| error.to_string())
        })();
        match result.and_then(|bytes| unsafe { write_byte_array(env, &bytes) }) {
            Ok(bytes) => bytes,
            Err(error) => {
                unsafe {
                    throw(env, "java/lang/IllegalArgumentException", &error);
                }
                std::ptr::null_mut()
            }
        }
    }

    unsafe fn read_byte_array(env: *mut JNIEnv, array: jbyteArray) -> Result<Vec<u8>, String> {
        if env.is_null() || array.is_null() {
            return Err("Android JNI received a null byte array".into());
        }
        let table = unsafe { &**env };
        let get_length = table
            .GetArrayLength
            .ok_or_else(|| "Android JNI GetArrayLength is unavailable".to_owned())?;
        let get_region = table
            .GetByteArrayRegion
            .ok_or_else(|| "Android JNI GetByteArrayRegion is unavailable".to_owned())?;
        let len = unsafe { get_length(env, array) };
        if len < 0 {
            return Err("Android JNI returned a negative byte-array length".into());
        }
        let mut bytes = vec![0u8; len as usize];
        unsafe {
            get_region(env, array, 0, len, bytes.as_mut_ptr().cast());
        }
        Ok(bytes)
    }

    unsafe fn write_byte_array(env: *mut JNIEnv, bytes: &[u8]) -> Result<jbyteArray, String> {
        if env.is_null() {
            return Err("Android JNI received a null environment".into());
        }
        let len = jint::try_from(bytes.len())
            .map_err(|_| "Android JNI output exceeded jint length".to_owned())?;
        let table = unsafe { &**env };
        let new_array = table
            .NewByteArray
            .ok_or_else(|| "Android JNI NewByteArray is unavailable".to_owned())?;
        let set_region = table
            .SetByteArrayRegion
            .ok_or_else(|| "Android JNI SetByteArrayRegion is unavailable".to_owned())?;
        let array = unsafe { new_array(env, len) };
        if array.is_null() {
            return Err("Android JNI could not allocate its output byte array".into());
        }
        unsafe {
            set_region(env, array, 0, len, bytes.as_ptr().cast());
        }
        Ok(array)
    }

    unsafe fn throw(env: *mut JNIEnv, class_name: &str, message: &str) {
        if env.is_null() {
            return;
        }
        let table = unsafe { &**env };
        let (Some(find_class), Some(throw_new)) = (table.FindClass, table.ThrowNew) else {
            return;
        };
        let Ok(class_name) = CString::new(class_name) else {
            return;
        };
        let Ok(message) = CString::new(message.replace('\0', " ")) else {
            return;
        };
        let class = unsafe { find_class(env, class_name.as_ptr()) };
        if !class.is_null() {
            unsafe {
                throw_new(env, class, message.as_ptr());
            }
        }
    }
}
