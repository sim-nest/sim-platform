//! Windows path mechanics confined to the Table/Dir membrane.

use sim_platform_core::stable_digest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathError {
    Empty,
    Absolute,
    Traversal,
    InvalidSeparator,
}

/// An identity-preserving relative Windows path. UTF-16 code units are retained
/// exactly; normalization only establishes the private API spelling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsPath {
    units: Vec<u16>,
    identity: String,
}

impl WindowsPath {
    /// Normalize a Table/Dir relative path for Windows APIs.
    ///
    /// # Errors
    /// Refuses absolute, drive-qualified, empty, or traversing input.
    pub fn from_table_units(units: &[u16]) -> Result<Self, PathError> {
        if units.is_empty() {
            return Err(PathError::Empty);
        }
        if matches!(units.first(), Some(47 | 92)) || units.get(1) == Some(&58) {
            return Err(PathError::Absolute);
        }
        let mut normalized = Vec::with_capacity(units.len());
        for &unit in units {
            if unit == 0 {
                return Err(PathError::InvalidSeparator);
            }
            normalized.push(if unit == 47 { 92 } else { unit });
        }
        if normalized
            .split(|unit| *unit == 92)
            .any(|part| part == [46, 46])
        {
            return Err(PathError::Traversal);
        }
        let bytes = units
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>();
        Ok(Self {
            units: normalized,
            identity: stable_digest(&bytes),
        })
    }

    #[must_use]
    pub fn api_units(&self) -> &[u16] {
        &self.units
    }
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }
}
