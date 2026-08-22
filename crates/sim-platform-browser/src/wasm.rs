//! Wasm linear-memory membrane. JavaScript owns allocation and byte movement;
//! all dispatch decisions remain in [`Capsule`].

#![allow(unsafe_code)]

use super::{BrowserApis, Capsule};
use std::{cell::RefCell, slice};

thread_local! {
    static CAPSULE: RefCell<Capsule> = RefCell::new(Capsule::new(&BrowserApis::default()));
}

#[unsafe(no_mangle)]
pub extern "C" fn sim_browser_alloc(len: usize) -> *mut u8 {
    let mut bytes = Vec::<u8>::with_capacity(len);
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sim_browser_dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        unsafe {
            drop(Vec::from_raw_parts(ptr, 0, len));
        }
    }
}

/// Calls one static named function. The packed result is `(ptr << 32) | len`.
/// An empty result denotes a rejected frame; detailed refusals stay in frames
/// on the safe modeled/test surface and are never interpreted by JavaScript.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sim_browser_named_call(
    name_ptr: *const u8,
    name_len: usize,
    frame_ptr: *const u8,
    frame_len: usize,
) -> u64 {
    if name_ptr.is_null() || frame_ptr.is_null() {
        return 0;
    }
    let name = unsafe { slice::from_raw_parts(name_ptr, name_len) };
    let frame = unsafe { slice::from_raw_parts(frame_ptr, frame_len) };
    let Ok(name) = std::str::from_utf8(name) else {
        return 0;
    };
    let Ok(mut output) = CAPSULE.with_borrow_mut(|capsule| capsule.call_frame(name, frame)) else {
        return 0;
    };
    let len = output.len();
    if len > u32::MAX as usize {
        return 0;
    }
    let ptr = output.as_mut_ptr();
    std::mem::forget(output);
    ((ptr as u64) << 32) | len as u64
}
