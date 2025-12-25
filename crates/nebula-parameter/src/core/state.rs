//! Parameter state and flags

use bitflags::bitflags;
use nebula_validator::core::ValidationError;

bitflags! {
    /// Flags representing the current state of a parameter.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct ParameterFlags: u8 {
        /// Value has been modified since last save/load.
        const DIRTY = 0b0000_0001;
        /// User has interacted with this parameter.
        const TOUCHED = 0b0000_0010;
        /// Parameter has passed validation.
        const VALID = 0b0000_0100;
        /// Parameter is currently visible (display conditions met).
        const VISIBLE = 0b0000_1000;
        /// Parameter is enabled (not disabled by conditions).
        const ENABLED = 0b0001_0000;
        /// Parameter is required.
        const REQUIRED = 0b0010_0000;
    }
}

/// Runtime state of a single parameter.
#[derive(Debug, Clone, Default)]
pub struct ParameterState {
    /// Current state flags.
    flags: ParameterFlags,
    /// Validation errors (empty if valid).
    errors: Vec<ValidationError>,
}

impl ParameterState {
    /// Create a new parameter state with default flags.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flags: ParameterFlags::VISIBLE | ParameterFlags::ENABLED,
            errors: Vec::new(),
        }
    }

    /// Get the current flags.
    #[must_use]
    pub fn flags(&self) -> ParameterFlags {
        self.flags
    }

    /// Get mutable access to flags.
    pub fn flags_mut(&mut self) -> &mut ParameterFlags {
        &mut self.flags
    }

    /// Set a flag.
    pub fn set_flag(&mut self, flag: ParameterFlags) {
        self.flags.insert(flag);
    }

    /// Clear a flag.
    pub fn clear_flag(&mut self, flag: ParameterFlags) {
        self.flags.remove(flag);
    }

    /// Check if a flag is set.
    #[must_use]
    pub fn has_flag(&self, flag: ParameterFlags) -> bool {
        self.flags.contains(flag)
    }

    /// Check if parameter is dirty.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.has_flag(ParameterFlags::DIRTY)
    }

    /// Check if parameter was touched.
    #[must_use]
    pub fn is_touched(&self) -> bool {
        self.has_flag(ParameterFlags::TOUCHED)
    }

    /// Check if parameter is valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.has_flag(ParameterFlags::VALID)
    }

    /// Check if parameter is visible.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.has_flag(ParameterFlags::VISIBLE)
    }

    /// Check if parameter is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.has_flag(ParameterFlags::ENABLED)
    }

    /// Get validation errors.
    #[must_use]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Set validation errors and update VALID flag.
    pub fn set_errors(&mut self, errors: Vec<ValidationError>) {
        if errors.is_empty() {
            self.flags.insert(ParameterFlags::VALID);
        } else {
            self.flags.remove(ParameterFlags::VALID);
        }
        self.errors = errors;
    }

    /// Clear validation errors and set VALID flag.
    pub fn clear_errors(&mut self) {
        self.errors.clear();
        self.flags.insert(ParameterFlags::VALID);
    }

    /// Mark as dirty.
    pub fn mark_dirty(&mut self) {
        self.flags.insert(ParameterFlags::DIRTY);
    }

    /// Mark as clean (not dirty).
    pub fn mark_clean(&mut self) {
        self.flags.remove(ParameterFlags::DIRTY);
    }

    /// Mark as touched.
    pub fn mark_touched(&mut self) {
        self.flags.insert(ParameterFlags::TOUCHED);
    }

    /// Set visibility.
    pub fn set_visible(&mut self, visible: bool) {
        if visible {
            self.flags.insert(ParameterFlags::VISIBLE);
        } else {
            self.flags.remove(ParameterFlags::VISIBLE);
        }
    }

    /// Set enabled state.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled {
            self.flags.insert(ParameterFlags::ENABLED);
        } else {
            self.flags.remove(ParameterFlags::ENABLED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = ParameterState::new();
        assert!(state.is_visible());
        assert!(state.is_enabled());
        assert!(!state.is_dirty());
        assert!(!state.is_touched());
        assert!(!state.is_valid());
    }

    #[test]
    fn test_flags() {
        let mut state = ParameterState::new();

        state.mark_dirty();
        assert!(state.is_dirty());

        state.mark_touched();
        assert!(state.is_touched());

        state.mark_clean();
        assert!(!state.is_dirty());
    }

    #[test]
    fn test_validation_errors() {
        let mut state = ParameterState::new();
        assert!(!state.is_valid());

        state.clear_errors();
        assert!(state.is_valid());
        assert!(state.errors().is_empty());

        state.set_errors(vec![ValidationError::new("test", "Test error")]);
        assert!(!state.is_valid());
        assert_eq!(state.errors().len(), 1);
    }

    #[test]
    fn test_visibility() {
        let mut state = ParameterState::new();
        assert!(state.is_visible());

        state.set_visible(false);
        assert!(!state.is_visible());

        state.set_visible(true);
        assert!(state.is_visible());
    }
}
