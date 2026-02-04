//! Parameter options for controlling behavior, access, and features.
//!
//! Flags provide a unified way to specify parameter attributes across different
//! platforms and use cases: workflow automation, 3D editors, game engines, form builders.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_parameter::core::ParameterFlags;
//!
//! // Required field with expression support (workflow automation)
//! let opts = ParameterFlags::REQUIRED | ParameterFlags::EXPRESSION;
//!
//! // Animatable parameter (3D editor, game engine)
//! let opts = ParameterFlags::ANIMATABLE | ParameterFlags::REALTIME;
//!
//! // Sensitive data (passwords, API keys)
//! let opts = ParameterFlags::sensitive();
//!
//! // Using convenience constructors
//! let opts = ParameterFlags::required().with_expression();
//! ```
//!
//! # Use Cases
//!
//! ## Workflow Automation (n8n-like)
//! ```rust,ignore
//! let input = ParameterFlags::REQUIRED | ParameterFlags::EXPRESSION;
//! let output = ParameterFlags::RUNTIME | ParameterFlags::SKIP_SAVE;
//! ```
//!
//! ## 3D Editor (Blender-like)
//! ```rust,ignore
//! let transform = ParameterFlags::animation_param();
//! let computed = ParameterFlags::computed();
//! ```
//!
//! ## Game Engine (Unity-like)
//! ```rust,ignore
//! let public_field = ParameterFlags::empty();  // Editable in inspector
//! let hide_in_inspector = ParameterFlags::HIDDEN;
//! ```
//!
//! ## Form Builder
//! ```rust,ignore
//! let email = ParameterFlags::REQUIRED;
//! let password = ParameterFlags::password_field();
//! let api_key = ParameterFlags::api_key();
//! ```

use bitflags::bitflags;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

bitflags! {
    /// Flags controlling parameter behavior, access, and features.
    ///
    /// These flags are designed to be platform-agnostic and can be used across
    /// different applications: workflow automation (n8n-like), 3D editors (Blender-like),
    /// game engines (Unity/Unreal-like), and form builders.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct ParameterFlags: u32 {
        // =================================================================
        // Requirement (bit 0)
        // =================================================================

        /// Value must be provided. Empty/null values will fail validation.
        const REQUIRED    = 1 << 0;

        // =================================================================
        // Access Control (bits 1-3)
        // =================================================================

        /// Cannot be modified in UI. Value is visible but input is locked.
        /// The value can still be set programmatically.
        const READONLY    = 1 << 1;

        /// Completely disabled. UI shows greyed out, no interaction possible.
        /// Unlike READONLY, the field appears inactive.
        const DISABLED    = 1 << 2;

        /// Hidden from UI but still part of schema.
        /// Value is still validated and serialized (unless SKIP_SAVE is also set).
        const HIDDEN      = 1 << 3;

        // =================================================================
        // Persistence (bits 4-5)
        // =================================================================

        /// Don't serialize this parameter to storage.
        /// Useful for computed values, caches, or temporary state.
        const SKIP_SAVE   = 1 << 4;

        /// Value is computed at runtime, not provided by user.
        /// Implies the value may change during execution.
        const RUNTIME     = 1 << 5;

        // =================================================================
        // Security (bits 6-7)
        // =================================================================

        /// Sensitive data that should be masked in logs and API responses.
        /// Examples: passwords, API keys, tokens, secrets.
        const SENSITIVE   = 1 << 6;

        /// Write-only parameter. Value is accepted but never returned in API responses.
        /// Examples: password confirmation, one-time tokens.
        const WRITE_ONLY  = 1 << 7;

        // =================================================================
        // Animation / Realtime (bits 8-9)
        // =================================================================

        /// Can be animated with keyframes and curves.
        /// Used in 3D editors, game engines, motion graphics.
        const ANIMATABLE  = 1 << 8;

        /// Triggers update on every change, not just on blur/submit.
        /// Useful for live preview, real-time feedback.
        const REALTIME    = 1 << 9;

        // =================================================================
        // Expressions / Dynamic (bits 10-11)
        // =================================================================

        /// Allows `{{ expression }}` syntax for dynamic values.
        /// Used in workflow automation for referencing other node outputs.
        const EXPRESSION  = 1 << 10;

        /// Can be overridden in linked/inherited data.
        /// Used for library overrides, prefab systems, template inheritance.
        const OVERRIDABLE = 1 << 11;

        // =================================================================
        // Lifecycle (bit 12)
        // =================================================================

        /// Parameter is deprecated. UI should show a warning.
        /// Value is still accepted for backwards compatibility.
        const DEPRECATED  = 1 << 12;

        // =================================================================
        // Network (bit 13)
        // =================================================================

        /// Sync this parameter across network.
        /// Used in multiplayer, distributed systems, collaborative editing.
        const REPLICATED  = 1 << 13;
    }
}

impl ParameterFlags {
    // =========================================================================
    // Convenience Constructors
    // =========================================================================

    /// Create flags for a required parameter.
    #[inline]
    #[must_use]
    pub fn required() -> Self {
        Self::REQUIRED
    }

    /// Create flags for an optional parameter (no flags set).
    #[inline]
    #[must_use]
    pub fn optional() -> Self {
        Self::empty()
    }

    /// Create flags for sensitive data (passwords, tokens).
    ///
    /// Combines: `SENSITIVE | WRITE_ONLY | SKIP_SAVE`
    #[inline]
    #[must_use]
    pub fn sensitive() -> Self {
        Self::SENSITIVE | Self::WRITE_ONLY | Self::SKIP_SAVE
    }

    /// Create flags for an animatable parameter.
    ///
    /// Combines: `ANIMATABLE | REALTIME`
    #[inline]
    #[must_use]
    pub fn animatable() -> Self {
        Self::ANIMATABLE | Self::REALTIME
    }

    /// Create flags for a runtime-computed value.
    ///
    /// Combines: `RUNTIME | READONLY | SKIP_SAVE`
    #[inline]
    #[must_use]
    pub fn computed() -> Self {
        Self::RUNTIME | Self::READONLY | Self::SKIP_SAVE
    }

    /// Create flags for a hidden internal parameter.
    ///
    /// Combines: `HIDDEN | SKIP_SAVE`
    #[inline]
    #[must_use]
    pub fn internal() -> Self {
        Self::HIDDEN | Self::SKIP_SAVE
    }

    // =========================================================================
    // Builder-style Methods
    // =========================================================================

    /// Add REQUIRED flag.
    #[inline]
    #[must_use]
    pub fn with_required(self) -> Self {
        self | Self::REQUIRED
    }

    /// Add READONLY flag.
    #[inline]
    #[must_use]
    pub fn with_readonly(self) -> Self {
        self | Self::READONLY
    }

    /// Add DISABLED flag.
    #[inline]
    #[must_use]
    pub fn with_disabled(self) -> Self {
        self | Self::DISABLED
    }

    /// Add HIDDEN flag.
    #[inline]
    #[must_use]
    pub fn with_hidden(self) -> Self {
        self | Self::HIDDEN
    }

    /// Add SENSITIVE flag.
    #[inline]
    #[must_use]
    pub fn with_sensitive(self) -> Self {
        self | Self::SENSITIVE
    }

    /// Add ANIMATABLE flag.
    #[inline]
    #[must_use]
    pub fn with_animatable(self) -> Self {
        self | Self::ANIMATABLE
    }

    /// Add REALTIME flag.
    #[inline]
    #[must_use]
    pub fn with_realtime(self) -> Self {
        self | Self::REALTIME
    }

    /// Add EXPRESSION flag.
    #[inline]
    #[must_use]
    pub fn with_expression(self) -> Self {
        self | Self::EXPRESSION
    }

    /// Add RUNTIME flag.
    #[inline]
    #[must_use]
    pub fn with_runtime(self) -> Self {
        self | Self::RUNTIME
    }

    /// Add SKIP_SAVE flag.
    #[inline]
    #[must_use]
    pub fn with_skip_save(self) -> Self {
        self | Self::SKIP_SAVE
    }

    /// Add WRITE_ONLY flag.
    #[inline]
    #[must_use]
    pub fn with_write_only(self) -> Self {
        self | Self::WRITE_ONLY
    }

    /// Add OVERRIDABLE flag.
    #[inline]
    #[must_use]
    pub fn with_overridable(self) -> Self {
        self | Self::OVERRIDABLE
    }

    /// Add DEPRECATED flag.
    #[inline]
    #[must_use]
    pub fn with_deprecated(self) -> Self {
        self | Self::DEPRECATED
    }

    /// Add REPLICATED flag.
    #[inline]
    #[must_use]
    pub fn with_replicated(self) -> Self {
        self | Self::REPLICATED
    }

    // =========================================================================
    // Query Methods
    // =========================================================================

    /// Check if this parameter is required.
    #[inline]
    #[must_use]
    pub fn is_required(self) -> bool {
        self.contains(Self::REQUIRED)
    }

    /// Check if this parameter is optional.
    #[inline]
    #[must_use]
    pub fn is_optional(self) -> bool {
        !self.is_required()
    }

    /// Check if this parameter is readonly.
    #[inline]
    #[must_use]
    pub fn is_readonly(self) -> bool {
        self.contains(Self::READONLY)
    }

    /// Check if this parameter is disabled.
    #[inline]
    #[must_use]
    pub fn is_disabled(self) -> bool {
        self.contains(Self::DISABLED)
    }

    /// Check if this parameter is hidden.
    #[inline]
    #[must_use]
    pub fn is_hidden(self) -> bool {
        self.contains(Self::HIDDEN)
    }

    /// Check if this parameter is visible in UI.
    #[inline]
    #[must_use]
    pub fn is_visible(self) -> bool {
        !self.is_hidden()
    }

    /// Check if this parameter should be serialized.
    #[inline]
    #[must_use]
    pub fn should_save(self) -> bool {
        !self.contains(Self::SKIP_SAVE)
    }

    /// Check if this parameter is runtime-computed.
    #[inline]
    #[must_use]
    pub fn is_runtime(self) -> bool {
        self.contains(Self::RUNTIME)
    }

    /// Check if this parameter contains sensitive data.
    #[inline]
    #[must_use]
    pub fn is_sensitive(self) -> bool {
        self.contains(Self::SENSITIVE)
    }

    /// Check if this parameter is write-only.
    #[inline]
    #[must_use]
    pub fn is_write_only(self) -> bool {
        self.contains(Self::WRITE_ONLY)
    }

    /// Check if this parameter can be animated.
    #[inline]
    #[must_use]
    pub fn is_animatable(self) -> bool {
        self.contains(Self::ANIMATABLE)
    }

    /// Check if this parameter updates in realtime.
    #[inline]
    #[must_use]
    pub fn is_realtime(self) -> bool {
        self.contains(Self::REALTIME)
    }

    /// Check if this parameter supports expressions.
    #[inline]
    #[must_use]
    pub fn supports_expression(self) -> bool {
        self.contains(Self::EXPRESSION)
    }

    /// Check if this parameter can be overridden.
    #[inline]
    #[must_use]
    pub fn is_overridable(self) -> bool {
        self.contains(Self::OVERRIDABLE)
    }

    /// Check if this parameter is deprecated.
    #[inline]
    #[must_use]
    pub fn is_deprecated(self) -> bool {
        self.contains(Self::DEPRECATED)
    }

    /// Check if this parameter is replicated over network.
    #[inline]
    #[must_use]
    pub fn is_replicated(self) -> bool {
        self.contains(Self::REPLICATED)
    }

    /// Check if this parameter is editable (not readonly and not disabled).
    #[inline]
    #[must_use]
    pub fn is_editable(self) -> bool {
        !self.is_readonly() && !self.is_disabled()
    }

    /// Check if this parameter is user-provided (not runtime-computed).
    #[inline]
    #[must_use]
    pub fn is_user_provided(self) -> bool {
        !self.is_runtime()
    }

    // =========================================================================
    // Group Query Methods
    // =========================================================================

    /// Check if this parameter can be displayed in UI.
    ///
    /// Returns `false` if `HIDDEN` or `WRITE_ONLY`.
    #[inline]
    #[must_use]
    pub fn can_display(self) -> bool {
        !self.is_hidden() && !self.is_write_only()
    }

    /// Check if this parameter can be edited by user.
    ///
    /// Returns `false` if `READONLY`, `DISABLED`, `RUNTIME`, or `WRITE_ONLY`.
    #[inline]
    #[must_use]
    pub fn can_edit(self) -> bool {
        !self.is_readonly() && !self.is_disabled() && !self.is_runtime() && !self.is_write_only()
    }

    /// Check if this parameter needs validation.
    ///
    /// Returns `true` if `REQUIRED` or not `RUNTIME`.
    #[inline]
    #[must_use]
    pub fn needs_validation(self) -> bool {
        self.is_required() || !self.is_runtime()
    }

    /// Check if this parameter affects other parameters.
    ///
    /// Returns `true` if `REALTIME` or `EXPRESSION`.
    #[inline]
    #[must_use]
    pub fn affects_others(self) -> bool {
        self.is_realtime() || self.supports_expression()
    }

    /// Check if this parameter should be included in API responses.
    ///
    /// Returns `false` if `WRITE_ONLY` or (`HIDDEN` and not for internal API).
    #[inline]
    #[must_use]
    pub fn include_in_response(self, internal_api: bool) -> bool {
        if self.is_write_only() {
            return false;
        }

        if self.is_hidden() && !internal_api {
            return false;
        }

        true
    }

    // =========================================================================
    // More Preset Constructors
    // =========================================================================

    /// Create flags for a form field (visible, editable, persistent).
    #[inline]
    #[must_use]
    pub fn form_field() -> Self {
        Self::empty()
    }

    /// Create flags for a required form field.
    #[inline]
    #[must_use]
    pub fn required_field() -> Self {
        Self::REQUIRED
    }

    /// Create flags for a readonly display field.
    #[inline]
    #[must_use]
    pub fn display_only() -> Self {
        Self::READONLY
    }

    /// Create flags for a cached/derived value.
    ///
    /// Combines: `RUNTIME | SKIP_SAVE`
    #[inline]
    #[must_use]
    pub fn cached() -> Self {
        Self::RUNTIME | Self::SKIP_SAVE
    }

    /// Create flags for metadata (hidden, saved).
    #[inline]
    #[must_use]
    pub fn metadata() -> Self {
        Self::HIDDEN
    }

    /// Create flags for temporary state (hidden, not saved).
    #[inline]
    #[must_use]
    pub fn temporary() -> Self {
        Self::HIDDEN | Self::SKIP_SAVE
    }

    /// Create flags for a networked parameter (replicated, realtime).
    #[inline]
    #[must_use]
    pub fn networked() -> Self {
        Self::REPLICATED | Self::REALTIME
    }

    /// Create flags for a password field: required, sensitive, write-only.
    ///
    /// Combines: `REQUIRED | SENSITIVE | WRITE_ONLY`
    #[inline]
    #[must_use]
    pub fn password_field() -> Self {
        Self::REQUIRED | Self::SENSITIVE | Self::WRITE_ONLY
    }

    /// Create flags for an API key field: sensitive, not saved.
    ///
    /// Combines: `SENSITIVE | WRITE_ONLY | SKIP_SAVE`
    #[inline]
    #[must_use]
    pub fn api_key() -> Self {
        Self::sensitive()
    }

    /// Create flags for a system field: readonly, runtime, hidden.
    ///
    /// Combines: `READONLY | RUNTIME | HIDDEN`
    #[inline]
    #[must_use]
    pub fn system_field() -> Self {
        Self::READONLY | Self::RUNTIME | Self::HIDDEN
    }

    /// Create flags for an animation parameter: animatable, realtime, replicated.
    ///
    /// Combines: `ANIMATABLE | REALTIME | REPLICATED`
    #[inline]
    #[must_use]
    pub fn animation_param() -> Self {
        Self::ANIMATABLE | Self::REALTIME | Self::REPLICATED
    }

    /// Create flags for an expression field: expression support, realtime.
    ///
    /// Combines: `EXPRESSION | REALTIME`
    #[inline]
    #[must_use]
    pub fn expression_field() -> Self {
        Self::EXPRESSION | Self::REALTIME
    }

    /// Create flags for a deprecated field: deprecated, readonly.
    ///
    /// Combines: `DEPRECATED | READONLY`
    #[inline]
    #[must_use]
    pub fn deprecated_field() -> Self {
        Self::DEPRECATED | Self::READONLY
    }

    // =========================================================================
    // Builder Remove Methods
    // =========================================================================

    /// Remove `REQUIRED` flag.
    #[inline]
    #[must_use]
    pub fn without_required(self) -> Self {
        self & !Self::REQUIRED
    }

    /// Remove `READONLY` flag.
    #[inline]
    #[must_use]
    pub fn without_readonly(self) -> Self {
        self & !Self::READONLY
    }

    /// Remove `DISABLED` flag.
    #[inline]
    #[must_use]
    pub fn without_disabled(self) -> Self {
        self & !Self::DISABLED
    }

    /// Remove `HIDDEN` flag.
    #[inline]
    #[must_use]
    pub fn without_hidden(self) -> Self {
        self & !Self::HIDDEN
    }

    /// Remove `SKIP_SAVE` flag.
    #[inline]
    #[must_use]
    pub fn without_skip_save(self) -> Self {
        self & !Self::SKIP_SAVE
    }

    /// Remove `RUNTIME` flag.
    #[inline]
    #[must_use]
    pub fn without_runtime(self) -> Self {
        self & !Self::RUNTIME
    }

    /// Remove `SENSITIVE` flag.
    #[inline]
    #[must_use]
    pub fn without_sensitive(self) -> Self {
        self & !Self::SENSITIVE
    }

    /// Remove `WRITE_ONLY` flag.
    #[inline]
    #[must_use]
    pub fn without_write_only(self) -> Self {
        self & !Self::WRITE_ONLY
    }

    /// Remove `ANIMATABLE` flag.
    #[inline]
    #[must_use]
    pub fn without_animatable(self) -> Self {
        self & !Self::ANIMATABLE
    }

    /// Remove `REALTIME` flag.
    #[inline]
    #[must_use]
    pub fn without_realtime(self) -> Self {
        self & !Self::REALTIME
    }

    /// Remove `EXPRESSION` flag.
    #[inline]
    #[must_use]
    pub fn without_expression(self) -> Self {
        self & !Self::EXPRESSION
    }

    /// Remove `OVERRIDABLE` flag.
    #[inline]
    #[must_use]
    pub fn without_overridable(self) -> Self {
        self & !Self::OVERRIDABLE
    }

    /// Remove `DEPRECATED` flag.
    #[inline]
    #[must_use]
    pub fn without_deprecated(self) -> Self {
        self & !Self::DEPRECATED
    }

    /// Remove `REPLICATED` flag.
    #[inline]
    #[must_use]
    pub fn without_replicated(self) -> Self {
        self & !Self::REPLICATED
    }

    // =========================================================================
    // Conflict Detection
    // =========================================================================

    /// Check for conflicting flags and return warnings.
    ///
    /// This method helps identify potentially problematic flag combinations
    /// that may indicate configuration errors.
    #[must_use]
    pub fn validate_consistency(self) -> Vec<&'static str> {
        let mut warnings = Vec::new();

        // READONLY + REQUIRED might be confusing
        if self.is_readonly() && self.is_required() {
            warnings.push("READONLY + REQUIRED: readonly fields can't fail required validation");
        }

        // DISABLED + REQUIRED is invalid
        if self.is_disabled() && self.is_required() {
            warnings.push("DISABLED + REQUIRED: disabled fields can't be required");
        }

        // HIDDEN + REQUIRED might be problematic
        if self.is_hidden() && self.is_required() {
            warnings.push("HIDDEN + REQUIRED: users can't provide hidden required values");
        }

        // RUNTIME + REQUIRED is invalid
        if self.is_runtime() && self.is_required() {
            warnings.push("RUNTIME + REQUIRED: runtime values are computed, not user-provided");
        }

        // WRITE_ONLY + READONLY is contradictory
        if self.is_write_only() && self.is_readonly() {
            warnings.push("WRITE_ONLY + READONLY: contradictory access control");
        }

        // ANIMATABLE without REALTIME is unusual
        if self.is_animatable() && !self.is_realtime() {
            warnings
                .push("ANIMATABLE without REALTIME: animations typically need realtime updates");
        }

        warnings
    }

    // =========================================================================
    // Utility Methods
    // =========================================================================

    /// Get all set flags as a `Vec` of names.
    #[must_use]
    pub fn flag_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.is_required() {
            names.push("REQUIRED");
        }
        if self.is_readonly() {
            names.push("READONLY");
        }
        if self.is_disabled() {
            names.push("DISABLED");
        }
        if self.is_hidden() {
            names.push("HIDDEN");
        }
        if !self.should_save() {
            names.push("SKIP_SAVE");
        }
        if self.is_runtime() {
            names.push("RUNTIME");
        }
        if self.is_sensitive() {
            names.push("SENSITIVE");
        }
        if self.is_write_only() {
            names.push("WRITE_ONLY");
        }
        if self.is_animatable() {
            names.push("ANIMATABLE");
        }
        if self.is_realtime() {
            names.push("REALTIME");
        }
        if self.supports_expression() {
            names.push("EXPRESSION");
        }
        if self.is_overridable() {
            names.push("OVERRIDABLE");
        }
        if self.is_deprecated() {
            names.push("DEPRECATED");
        }
        if self.is_replicated() {
            names.push("REPLICATED");
        }
        names
    }

    /// Count how many flags are set.
    #[inline]
    #[must_use]
    pub fn count_set(&self) -> u32 {
        self.bits().count_ones()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_empty() {
        let flags = ParameterFlags::default();
        assert!(flags.is_empty());
        assert!(flags.is_optional());
        assert!(flags.is_visible());
        assert!(flags.is_editable());
    }

    #[test]
    fn test_required() {
        let flags = ParameterFlags::required();
        assert!(flags.is_required());
        assert!(!flags.is_optional());
    }

    #[test]
    fn test_sensitive() {
        let flags = ParameterFlags::sensitive();
        assert!(flags.is_sensitive());
        assert!(flags.is_write_only());
        assert!(!flags.should_save());
    }

    #[test]
    fn test_animatable() {
        let flags = ParameterFlags::animatable();
        assert!(flags.is_animatable());
        assert!(flags.is_realtime());
    }

    #[test]
    fn test_computed() {
        let flags = ParameterFlags::computed();
        assert!(flags.is_runtime());
        assert!(flags.is_readonly());
        assert!(!flags.should_save());
    }

    #[test]
    fn test_builder_chain() {
        let flags = ParameterFlags::required().with_expression().with_realtime();

        assert!(flags.is_required());
        assert!(flags.supports_expression());
        assert!(flags.is_realtime());
        assert!(!flags.is_animatable());
    }

    #[test]
    fn test_combination() {
        let flags = ParameterFlags::REQUIRED | ParameterFlags::HIDDEN;
        assert!(flags.is_required());
        assert!(flags.is_hidden());
        assert!(!flags.is_visible());
    }

    #[test]
    fn test_editable() {
        assert!(ParameterFlags::empty().is_editable());
        assert!(ParameterFlags::REQUIRED.is_editable());
        assert!(!ParameterFlags::READONLY.is_editable());
        assert!(!ParameterFlags::DISABLED.is_editable());
        assert!(!(ParameterFlags::READONLY | ParameterFlags::DISABLED).is_editable());
    }

    #[test]
    fn test_serialization() {
        let flags = ParameterFlags::REQUIRED | ParameterFlags::EXPRESSION;
        let json = serde_json::to_string(&flags).unwrap();
        let deserialized: ParameterFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(flags, deserialized);
    }

    #[test]
    fn test_display_empty() {
        let flags = ParameterFlags::empty();
        assert_eq!(format!("{}", flags), "none");
    }

    #[test]
    fn test_display_single() {
        let flags = ParameterFlags::REQUIRED;
        assert_eq!(format!("{}", flags), "required");
    }

    #[test]
    fn test_display_multiple() {
        let flags =
            ParameterFlags::REQUIRED | ParameterFlags::EXPRESSION | ParameterFlags::REALTIME;
        let display = format!("{}", flags);
        assert!(display.contains("required"));
        assert!(display.contains("expression"));
        assert!(display.contains("realtime"));
    }

    #[test]
    fn test_from_str() {
        let flags: ParameterFlags = "required | expression | realtime".parse().unwrap();
        assert!(flags.is_required());
        assert!(flags.supports_expression());
        assert!(flags.is_realtime());
    }

    #[test]
    fn test_from_str_case_insensitive() {
        let flags: ParameterFlags = "REQUIRED | Expression | ReAlTiMe".parse().unwrap();
        assert!(flags.is_required());
        assert!(flags.supports_expression());
        assert!(flags.is_realtime());
    }

    #[test]
    fn test_from_str_empty() {
        let flags: ParameterFlags = "".parse().unwrap();
        assert!(flags.is_empty());

        let flags: ParameterFlags = "none".parse().unwrap();
        assert!(flags.is_empty());
    }

    #[test]
    fn test_from_str_unknown_flag() {
        let result: Result<ParameterFlags, _> = "required | unknown_flag".parse();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().unknown_flag, "unknown_flag");
    }

    #[test]
    fn test_can_display() {
        assert!(ParameterFlags::empty().can_display());
        assert!(ParameterFlags::REQUIRED.can_display());
        assert!(!ParameterFlags::HIDDEN.can_display());
        assert!(!ParameterFlags::WRITE_ONLY.can_display());
    }

    #[test]
    fn test_can_edit() {
        assert!(ParameterFlags::empty().can_edit());
        assert!(ParameterFlags::REQUIRED.can_edit());
        assert!(!ParameterFlags::READONLY.can_edit());
        assert!(!ParameterFlags::DISABLED.can_edit());
        assert!(!ParameterFlags::RUNTIME.can_edit());
        assert!(!ParameterFlags::WRITE_ONLY.can_edit());
    }

    #[test]
    fn test_needs_validation() {
        assert!(ParameterFlags::REQUIRED.needs_validation());
        assert!(ParameterFlags::empty().needs_validation()); // not runtime
        assert!(!ParameterFlags::RUNTIME.needs_validation());
        assert!(ParameterFlags::required().with_runtime().needs_validation()); // required overrides
    }

    #[test]
    fn test_affects_others() {
        assert!(!ParameterFlags::empty().affects_others());
        assert!(ParameterFlags::REALTIME.affects_others());
        assert!(ParameterFlags::EXPRESSION.affects_others());
    }

    #[test]
    fn test_include_in_response() {
        let flags = ParameterFlags::empty();
        assert!(flags.include_in_response(false));
        assert!(flags.include_in_response(true));

        let write_only = ParameterFlags::WRITE_ONLY;
        assert!(!write_only.include_in_response(false));
        assert!(!write_only.include_in_response(true));

        let hidden = ParameterFlags::HIDDEN;
        assert!(!hidden.include_in_response(false));
        assert!(hidden.include_in_response(true));
    }

    #[test]
    fn test_preset_constructors() {
        assert_eq!(ParameterFlags::form_field(), ParameterFlags::empty());
        assert_eq!(ParameterFlags::required_field(), ParameterFlags::REQUIRED);
        assert_eq!(ParameterFlags::display_only(), ParameterFlags::READONLY);
        assert_eq!(
            ParameterFlags::cached(),
            ParameterFlags::RUNTIME | ParameterFlags::SKIP_SAVE
        );
        assert_eq!(ParameterFlags::metadata(), ParameterFlags::HIDDEN);
        assert_eq!(
            ParameterFlags::temporary(),
            ParameterFlags::HIDDEN | ParameterFlags::SKIP_SAVE
        );
        assert_eq!(
            ParameterFlags::networked(),
            ParameterFlags::REPLICATED | ParameterFlags::REALTIME
        );
        assert_eq!(
            ParameterFlags::password_field(),
            ParameterFlags::REQUIRED | ParameterFlags::SENSITIVE | ParameterFlags::WRITE_ONLY
        );
        assert_eq!(ParameterFlags::api_key(), ParameterFlags::sensitive());
        assert_eq!(
            ParameterFlags::system_field(),
            ParameterFlags::READONLY | ParameterFlags::RUNTIME | ParameterFlags::HIDDEN
        );
        assert_eq!(
            ParameterFlags::animation_param(),
            ParameterFlags::ANIMATABLE | ParameterFlags::REALTIME | ParameterFlags::REPLICATED
        );
        assert_eq!(
            ParameterFlags::expression_field(),
            ParameterFlags::EXPRESSION | ParameterFlags::REALTIME
        );
        assert_eq!(
            ParameterFlags::deprecated_field(),
            ParameterFlags::DEPRECATED | ParameterFlags::READONLY
        );
    }

    #[test]
    fn test_without_methods() {
        let flags = ParameterFlags::sensitive(); // SENSITIVE | WRITE_ONLY | SKIP_SAVE

        let without_sensitive = flags.without_sensitive();
        assert!(!without_sensitive.is_sensitive());
        assert!(without_sensitive.is_write_only());

        let without_write_only = flags.without_write_only();
        assert!(without_write_only.is_sensitive());
        assert!(!without_write_only.is_write_only());

        // Chain removes
        let minimal = flags
            .without_sensitive()
            .without_write_only()
            .without_skip_save();
        assert!(minimal.is_empty());
    }

    #[test]
    fn test_validate_consistency() {
        // No warnings for valid combinations
        assert!(ParameterFlags::required().validate_consistency().is_empty());
        assert!(
            ParameterFlags::animatable()
                .validate_consistency()
                .is_empty()
        );

        // READONLY + REQUIRED
        let flags = ParameterFlags::READONLY | ParameterFlags::REQUIRED;
        let warnings = flags.validate_consistency();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("READONLY + REQUIRED"));

        // DISABLED + REQUIRED
        let flags = ParameterFlags::DISABLED | ParameterFlags::REQUIRED;
        let warnings = flags.validate_consistency();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("DISABLED + REQUIRED"));

        // HIDDEN + REQUIRED
        let flags = ParameterFlags::HIDDEN | ParameterFlags::REQUIRED;
        let warnings = flags.validate_consistency();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("HIDDEN + REQUIRED"));

        // RUNTIME + REQUIRED
        let flags = ParameterFlags::RUNTIME | ParameterFlags::REQUIRED;
        let warnings = flags.validate_consistency();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("RUNTIME + REQUIRED"));

        // WRITE_ONLY + READONLY
        let flags = ParameterFlags::WRITE_ONLY | ParameterFlags::READONLY;
        let warnings = flags.validate_consistency();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("WRITE_ONLY + READONLY"));

        // ANIMATABLE without REALTIME
        let flags = ParameterFlags::ANIMATABLE;
        let warnings = flags.validate_consistency();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ANIMATABLE without REALTIME"));
    }

    #[test]
    fn test_flag_names() {
        let flags = ParameterFlags::REQUIRED | ParameterFlags::EXPRESSION;
        let names = flags.flag_names();
        assert!(names.contains(&"REQUIRED"));
        assert!(names.contains(&"EXPRESSION"));
        assert!(!names.contains(&"HIDDEN"));

        let empty_names = ParameterFlags::empty().flag_names();
        assert!(empty_names.is_empty());
    }

    #[test]
    fn test_count_set() {
        assert_eq!(ParameterFlags::empty().count_set(), 0);
        assert_eq!(ParameterFlags::REQUIRED.count_set(), 1);
        assert_eq!(
            (ParameterFlags::REQUIRED | ParameterFlags::EXPRESSION).count_set(),
            2
        );
        assert_eq!(ParameterFlags::sensitive().count_set(), 3);
    }

    #[test]
    fn test_roundtrip_display_fromstr() {
        let flags =
            ParameterFlags::REQUIRED | ParameterFlags::EXPRESSION | ParameterFlags::REALTIME;
        let display = format!("{}", flags);
        let parsed: ParameterFlags = display.parse().unwrap();
        assert_eq!(flags, parsed);
    }
}

// =============================================================================
// Runtime State Flags
// =============================================================================

bitflags! {
    /// Flags representing the current runtime state of a parameter.
    ///
    /// These flags track transient state during editing and validation,
    /// separate from the configuration flags ([`ParameterFlags`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::core::StateFlags;
    ///
    /// // Default state: visible and enabled
    /// let flags = StateFlags::default();
    /// assert!(flags.contains(StateFlags::VISIBLE));
    /// assert!(flags.contains(StateFlags::ENABLED));
    ///
    /// // Combine flags
    /// let flags = StateFlags::DIRTY | StateFlags::TOUCHED;
    /// assert!(flags.contains(StateFlags::DIRTY));
    /// ```
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct StateFlags: u8 {
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
    }
}

impl Default for StateFlags {
    /// Returns default state flags: `VISIBLE | ENABLED`.
    ///
    /// New parameters are visible and enabled by default.
    fn default() -> Self {
        Self::VISIBLE | Self::ENABLED
    }
}

// =============================================================================
// ParameterState
// =============================================================================

use nebula_validator::core::ValidationError;

/// Runtime state of a single parameter.
///
/// Tracks transient state during editing: dirty/touched flags, validation
/// errors, visibility, and enabled state. This is separate from the static
/// configuration in [`ParameterFlags`].
///
/// # Examples
///
/// ```
/// use nebula_parameter::core::ParameterState;
///
/// // Create new state (visible and enabled by default)
/// let mut state = ParameterState::new();
/// assert!(state.is_visible());
/// assert!(state.is_enabled());
/// assert!(!state.is_dirty());
///
/// // Update on user input
/// state.on_input();
/// assert!(state.is_dirty());
/// assert!(state.is_touched());
///
/// // Reset to initial state
/// state.reset();
/// assert!(!state.is_dirty());
/// assert!(!state.is_touched());
/// ```
#[derive(Debug, Clone, Default)]
pub struct ParameterState {
    /// Current state flags.
    flags: StateFlags,
    /// Validation errors (empty if valid).
    errors: Vec<ValidationError>,
}

impl ParameterState {
    /// Create a new parameter state with default flags.
    ///
    /// The parameter starts as visible, enabled, and untouched.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flags: StateFlags::default(),
            errors: Vec::new(),
        }
    }

    /// Get the current flags.
    #[inline]
    #[must_use]
    pub fn flags(&self) -> StateFlags {
        self.flags
    }

    /// Get mutable access to flags.
    pub fn flags_mut(&mut self) -> &mut StateFlags {
        &mut self.flags
    }

    /// Set a flag.
    pub fn set_flag(&mut self, flag: StateFlags) {
        self.flags.insert(flag);
    }

    /// Clear a flag.
    pub fn clear_flag(&mut self, flag: StateFlags) {
        self.flags.remove(flag);
    }

    /// Check if a flag is set.
    #[inline]
    #[must_use]
    pub fn has_flag(&self, flag: StateFlags) -> bool {
        self.flags.contains(flag)
    }

    /// Check if parameter is dirty.
    #[inline]
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.has_flag(StateFlags::DIRTY)
    }

    /// Check if parameter was touched.
    #[inline]
    #[must_use]
    pub fn is_touched(&self) -> bool {
        self.has_flag(StateFlags::TOUCHED)
    }

    /// Check if parameter is valid.
    #[inline]
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.has_flag(StateFlags::VALID)
    }

    /// Check if parameter is visible.
    #[inline]
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.has_flag(StateFlags::VISIBLE)
    }

    /// Check if parameter is enabled.
    #[inline]
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.has_flag(StateFlags::ENABLED)
    }

    /// Get validation errors.
    #[inline]
    #[must_use]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Set validation errors and update VALID flag.
    pub fn set_errors(&mut self, errors: Vec<ValidationError>) {
        if errors.is_empty() {
            self.flags.insert(StateFlags::VALID);
        } else {
            self.flags.remove(StateFlags::VALID);
        }
        self.errors = errors;
    }

    /// Clear validation errors and set VALID flag.
    pub fn clear_errors(&mut self) {
        self.errors.clear();
        self.flags.insert(StateFlags::VALID);
    }

    /// Mark as dirty.
    pub fn mark_dirty(&mut self) {
        self.flags.insert(StateFlags::DIRTY);
    }

    /// Mark as clean (not dirty).
    pub fn mark_clean(&mut self) {
        self.flags.remove(StateFlags::DIRTY);
    }

    /// Mark as touched.
    pub fn mark_touched(&mut self) {
        self.flags.insert(StateFlags::TOUCHED);
    }

    /// Set visibility.
    pub fn set_visible(&mut self, visible: bool) {
        if visible {
            self.flags.insert(StateFlags::VISIBLE);
        } else {
            self.flags.remove(StateFlags::VISIBLE);
        }
    }

    /// Set enabled state.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled {
            self.flags.insert(StateFlags::ENABLED);
        } else {
            self.flags.remove(StateFlags::ENABLED);
        }
    }

    // =========================================================================
    // Convenience Methods
    // =========================================================================

    /// Update state after user input.
    ///
    /// Marks the parameter as both dirty and touched in a single call.
    /// This is the typical state change when a user modifies a field.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::core::ParameterState;
    ///
    /// let mut state = ParameterState::new();
    /// state.on_input();
    ///
    /// assert!(state.is_dirty());
    /// assert!(state.is_touched());
    /// ```
    pub fn on_input(&mut self) {
        self.mark_touched();
        self.mark_dirty();
    }

    /// Update state after validation completes.
    ///
    /// Sets the validation state based on the result. If validation succeeded,
    /// clears any previous errors. If validation failed, stores the error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::core::ParameterState;
    /// use nebula_validator::core::ValidationError;
    ///
    /// let mut state = ParameterState::new();
    ///
    /// // Validation passed
    /// state.on_validate(Ok(()));
    /// assert!(state.is_valid());
    ///
    /// // Validation failed
    /// state.on_validate(Err(ValidationError::new("field", "invalid value")));
    /// assert!(!state.is_valid());
    /// ```
    pub fn on_validate(&mut self, result: Result<(), ValidationError>) {
        match result {
            Ok(()) => self.clear_errors(),
            Err(e) => self.set_errors(vec![e]),
        }
    }

    /// Reset to initial state.
    ///
    /// Restores the parameter to its default state: visible, enabled,
    /// not dirty, not touched, and no validation errors.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::core::ParameterState;
    ///
    /// let mut state = ParameterState::new();
    /// state.on_input();
    /// state.set_visible(false);
    ///
    /// state.reset();
    ///
    /// assert!(!state.is_dirty());
    /// assert!(!state.is_touched());
    /// assert!(state.is_visible());
    /// assert!(state.is_enabled());
    /// ```
    pub fn reset(&mut self) {
        self.flags = StateFlags::default();
        self.errors.clear();
    }
}

// =============================================================================
// Serde Implementation (serialize as u32)
// =============================================================================

impl Serialize for ParameterFlags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.bits().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ParameterFlags {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bits = u32::deserialize(deserializer)?;
        Ok(Self::from_bits_retain(bits))
    }
}

// =============================================================================
// Display Implementation
// =============================================================================

impl fmt::Display for ParameterFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return write!(f, "none");
        }

        let mut flags = Vec::new();

        if self.is_required() {
            flags.push("required");
        }
        if self.is_readonly() {
            flags.push("readonly");
        }
        if self.is_disabled() {
            flags.push("disabled");
        }
        if self.is_hidden() {
            flags.push("hidden");
        }
        if !self.should_save() {
            flags.push("skip_save");
        }
        if self.is_runtime() {
            flags.push("runtime");
        }
        if self.is_sensitive() {
            flags.push("sensitive");
        }
        if self.is_write_only() {
            flags.push("write_only");
        }
        if self.is_animatable() {
            flags.push("animatable");
        }
        if self.is_realtime() {
            flags.push("realtime");
        }
        if self.supports_expression() {
            flags.push("expression");
        }
        if self.is_overridable() {
            flags.push("overridable");
        }
        if self.is_deprecated() {
            flags.push("deprecated");
        }
        if self.is_replicated() {
            flags.push("replicated");
        }

        write!(f, "{}", flags.join(" | "))
    }
}

// =============================================================================
// FromStr Implementation
// =============================================================================

/// Error type for parsing `ParameterFlags` from a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFlagsError {
    /// The unknown flag name that caused the error.
    pub unknown_flag: String,
}

impl fmt::Display for ParseFlagsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown flag: {}", self.unknown_flag)
    }
}

impl std::error::Error for ParseFlagsError {}

impl FromStr for ParameterFlags {
    type Err = ParseFlagsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut flags = Self::empty();

        for part in s.split('|').map(str::trim) {
            match part.to_lowercase().as_str() {
                "required" => flags |= Self::REQUIRED,
                "readonly" => flags |= Self::READONLY,
                "disabled" => flags |= Self::DISABLED,
                "hidden" => flags |= Self::HIDDEN,
                "skip_save" => flags |= Self::SKIP_SAVE,
                "runtime" => flags |= Self::RUNTIME,
                "sensitive" => flags |= Self::SENSITIVE,
                "write_only" => flags |= Self::WRITE_ONLY,
                "animatable" => flags |= Self::ANIMATABLE,
                "realtime" => flags |= Self::REALTIME,
                "expression" => flags |= Self::EXPRESSION,
                "overridable" => flags |= Self::OVERRIDABLE,
                "deprecated" => flags |= Self::DEPRECATED,
                "replicated" => flags |= Self::REPLICATED,
                "none" | "" => {}
                unknown => {
                    return Err(ParseFlagsError {
                        unknown_flag: unknown.to_owned(),
                    });
                }
            }
        }

        Ok(flags)
    }
}
