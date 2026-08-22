//! Hardware-free unsupported VITURE SDK stub.

use crate::dynload::VitureLib;

/// Returns an SDK surface that reports unsupported for all live calls.
pub fn unsupported_viture_lib() -> VitureLib {
    VitureLib::stub()
}
