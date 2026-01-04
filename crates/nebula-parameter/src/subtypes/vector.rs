//! Vector parameter subtypes for fixed-size numeric arrays.
//!
//! This module defines subtypes for vector-like numeric values used in
//! graphics, 3D applications, game engines, and scientific computing.
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::core::subtype::VectorSubtype;
//!
//! // 3D position
//! let subtype = VectorSubtype::Vector3;
//!
//! // RGB color
//! let subtype = VectorSubtype::ColorRgb;
//!
//! // 4x4 transformation matrix
//! let subtype = VectorSubtype::Matrix4x4;
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;

/// Semantic subtype for vector (fixed-size numeric array) parameters.
///
/// Vectors are used extensively in graphics, 3D applications, game engines,
/// and scientific computing. This enum provides semantic meaning for common
/// vector types, enabling proper validation, UI hints, and transformations.
///
/// # Categories
///
/// - **Geometric Vectors**: 2D, 3D, 4D vectors (Vector2, Vector3, Vector4)
/// - **Colors**: RGB, RGBA, HSV, HSL color spaces
/// - **Matrices**: 2x2, 3x3, 4x4 transformation matrices
/// - **Quaternions**: Rotation representation
/// - **Specialized**: Texture coordinates, normals, tangents
/// - **Custom**: User-defined vector types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorSubtype {
    // =========================================================================
    // Geometric Vectors (4 variants)
    // =========================================================================
    /// 2D vector (x, y).
    ///
    /// # Use Cases
    /// - 2D positions
    /// - 2D directions
    /// - Screen coordinates
    /// - UV texture coordinates
    ///
    /// # Components
    /// - x: horizontal
    /// - y: vertical
    ///
    /// # UI Hint
    /// Two numeric inputs labeled X, Y.
    ///
    /// # Example
    /// ```text
    /// [10.5, 20.3]
    /// ```
    Vector2,

    /// 3D vector (x, y, z).
    ///
    /// # Use Cases
    /// - 3D positions
    /// - 3D directions
    /// - Normals
    /// - Velocities
    ///
    /// # Components
    /// - x: horizontal
    /// - y: vertical
    /// - z: depth
    ///
    /// # UI Hint
    /// Three numeric inputs labeled X, Y, Z.
    ///
    /// # Example
    /// ```text
    /// [1.0, 2.0, 3.0]
    /// ```
    Vector3,

    /// 4D vector (x, y, z, w).
    ///
    /// # Use Cases
    /// - Homogeneous coordinates
    /// - Quaternions (when used for rotation)
    /// - RGBA colors (when used for color)
    /// - Tangent space vectors
    ///
    /// # Components
    /// - x, y, z: spatial
    /// - w: homogeneous/weight component
    ///
    /// # UI Hint
    /// Four numeric inputs labeled X, Y, Z, W.
    ///
    /// # Example
    /// ```text
    /// [1.0, 2.0, 3.0, 1.0]
    /// ```
    Vector4,

    /// Generic N-dimensional vector.
    ///
    /// # Use Cases
    /// - Variable-size vectors
    /// - Scientific computing
    /// - Machine learning features
    ///
    /// # UI Hint
    /// Array of numeric inputs.
    VectorN,

    // =========================================================================
    // Positions and Offsets (3 variants)
    // =========================================================================
    /// 2D position (x, y).
    ///
    /// # Use Cases
    /// - Widget positions
    /// - Mouse coordinates
    /// - 2D game positions
    ///
    /// # UI Hint
    /// Position picker or two numeric inputs.
    Position2D,

    /// 3D position (x, y, z).
    ///
    /// # Use Cases
    /// - Object positions in 3D space
    /// - Camera positions
    /// - Light positions
    ///
    /// # UI Hint
    /// 3D position picker or three numeric inputs.
    Position3D,

    /// 2D offset or translation (dx, dy).
    ///
    /// # Use Cases
    /// - Relative movement
    /// - Translation transforms
    /// - Viewport scrolling
    ///
    /// # UI Hint
    /// Two numeric inputs with +/- indicators.
    Offset2D,

    // =========================================================================
    // Directions and Normals (3 variants)
    // =========================================================================
    /// 2D direction vector (normalized).
    ///
    /// # Use Cases
    /// - Movement directions
    /// - 2D physics
    /// - Input directions
    ///
    /// # Validation
    /// Should be normalized (length = 1).
    ///
    /// # UI Hint
    /// Direction picker (angle + arrow) or two inputs.
    Direction2D,

    /// 3D direction vector (normalized).
    ///
    /// # Use Cases
    /// - Camera forward/up vectors
    /// - Light directions
    /// - Movement directions
    ///
    /// # Validation
    /// Should be normalized (length = 1).
    ///
    /// # UI Hint
    /// 3D direction picker or three inputs.
    Direction3D,

    /// Surface normal vector (normalized).
    ///
    /// # Use Cases
    /// - 3D mesh normals
    /// - Lighting calculations
    /// - Collision detection
    ///
    /// # Validation
    /// Must be normalized (length = 1).
    ///
    /// # UI Hint
    /// Three numeric inputs, auto-normalize option.
    Normal,

    // =========================================================================
    // Scales and Sizes (3 variants)
    // =========================================================================
    /// 2D scale (sx, sy).
    ///
    /// # Use Cases
    /// - Object scaling
    /// - Sprite scaling
    /// - UI scaling
    ///
    /// # UI Hint
    /// Two numeric inputs with link button for uniform scaling.
    Scale2D,

    /// 3D scale (sx, sy, sz).
    ///
    /// # Use Cases
    /// - 3D object scaling
    /// - Transform scaling
    ///
    /// # UI Hint
    /// Three numeric inputs with link button for uniform scaling.
    Scale3D,

    /// 2D size (width, height).
    ///
    /// # Use Cases
    /// - Rectangle dimensions
    /// - Image sizes
    /// - Window sizes
    ///
    /// # Validation
    /// Typically non-negative.
    ///
    /// # UI Hint
    /// Two numeric inputs labeled Width, Height.
    Size2D,

    // =========================================================================
    // Rotations (3 variants)
    // =========================================================================
    /// Euler angles (pitch, yaw, roll) in degrees.
    ///
    /// # Use Cases
    /// - 3D rotations
    /// - Camera orientation
    /// - Object orientation
    ///
    /// # Components
    /// - pitch: rotation around X axis
    /// - yaw: rotation around Y axis
    /// - roll: rotation around Z axis
    ///
    /// # UI Hint
    /// Three numeric inputs with degree symbols.
    ///
    /// # Example
    /// ```text
    /// [45.0, 90.0, 0.0]
    /// ```
    EulerAngles,

    /// Quaternion (x, y, z, w).
    ///
    /// # Use Cases
    /// - 3D rotations (no gimbal lock)
    /// - Animation interpolation
    /// - Physics simulations
    ///
    /// # Components
    /// - x, y, z: imaginary parts
    /// - w: real part
    ///
    /// # Validation
    /// Should be normalized (magnitude = 1).
    ///
    /// # UI Hint
    /// Four numeric inputs or Euler angle converter.
    ///
    /// # Example
    /// ```text
    /// [0.0, 0.0, 0.0, 1.0] (identity)
    /// ```
    Quaternion,

    /// Axis-angle rotation (axis_x, axis_y, axis_z, angle).
    ///
    /// # Use Cases
    /// - Rotation representation
    /// - Animation
    ///
    /// # Components
    /// - axis (x, y, z): rotation axis (normalized)
    /// - angle: rotation angle in radians
    ///
    /// # UI Hint
    /// Three inputs for axis + one for angle.
    AxisAngle,

    // =========================================================================
    // Colors (6 variants)
    // =========================================================================
    /// RGB color (red, green, blue).
    ///
    /// # Use Cases
    /// - Color selection
    /// - Material colors
    /// - Lighting colors
    ///
    /// # Components
    /// - red: 0.0-1.0
    /// - green: 0.0-1.0
    /// - blue: 0.0-1.0
    ///
    /// # UI Hint
    /// Color picker (RGB sliders or color wheel).
    ///
    /// # Example
    /// ```text
    /// [1.0, 0.0, 0.0] (red)
    /// ```
    ColorRgb,

    /// RGBA color (red, green, blue, alpha).
    ///
    /// # Use Cases
    /// - Colors with transparency
    /// - UI elements
    /// - Image compositing
    ///
    /// # Components
    /// - red: 0.0-1.0
    /// - green: 0.0-1.0
    /// - blue: 0.0-1.0
    /// - alpha: 0.0-1.0 (transparency)
    ///
    /// # UI Hint
    /// Color picker with alpha slider.
    ///
    /// # Example
    /// ```text
    /// [1.0, 0.0, 0.0, 0.5] (semi-transparent red)
    /// ```
    ColorRgba,

    /// HSV color (hue, saturation, value).
    ///
    /// # Use Cases
    /// - Intuitive color selection
    /// - Color adjustments
    /// - Color palettes
    ///
    /// # Components
    /// - hue: 0.0-360.0 (degrees)
    /// - saturation: 0.0-1.0
    /// - value: 0.0-1.0
    ///
    /// # UI Hint
    /// HSV color picker (hue wheel + SV square).
    ///
    /// # Example
    /// ```text
    /// [0.0, 1.0, 1.0] (red)
    /// ```
    ColorHsv,

    /// HSL color (hue, saturation, lightness).
    ///
    /// # Use Cases
    /// - Color selection
    /// - CSS colors
    /// - Web design
    ///
    /// # Components
    /// - hue: 0.0-360.0 (degrees)
    /// - saturation: 0.0-1.0
    /// - lightness: 0.0-1.0
    ///
    /// # UI Hint
    /// HSL color picker.
    ///
    /// # Example
    /// ```text
    /// [0.0, 1.0, 0.5] (red)
    /// ```
    ColorHsl,

    /// Linear RGB color (red, green, blue) in linear space.
    ///
    /// # Use Cases
    /// - Physically accurate rendering
    /// - Color math
    /// - HDR colors
    ///
    /// # Components
    /// - red: linear color space
    /// - green: linear color space
    /// - blue: linear color space
    ///
    /// # UI Hint
    /// Color picker with linear space indicator.
    ColorLinearRgb,

    /// sRGB color (red, green, blue) in sRGB space.
    ///
    /// # Use Cases
    /// - Standard web colors
    /// - Monitor display
    /// - Image editing
    ///
    /// # Components
    /// - red: sRGB color space (gamma corrected)
    /// - green: sRGB color space
    /// - blue: sRGB color space
    ///
    /// # UI Hint
    /// Standard color picker (sRGB).
    ColorSrgb,

    // =========================================================================
    // Texture Coordinates (2 variants)
    // =========================================================================
    /// 2D texture coordinates (u, v).
    ///
    /// # Use Cases
    /// - Texture mapping
    /// - UV unwrapping
    /// - Material properties
    ///
    /// # Components
    /// - u: horizontal (0.0-1.0)
    /// - v: vertical (0.0-1.0)
    ///
    /// # UI Hint
    /// Two numeric inputs labeled U, V.
    ///
    /// # Example
    /// ```text
    /// [0.5, 0.5] (center of texture)
    /// ```
    TexCoord2D,

    /// 3D texture coordinates (u, v, w).
    ///
    /// # Use Cases
    /// - 3D textures
    /// - Volume rendering
    ///
    /// # Components
    /// - u, v, w: 3D texture space (0.0-1.0)
    ///
    /// # UI Hint
    /// Three numeric inputs labeled U, V, W.
    TexCoord3D,

    // =========================================================================
    // Bounds and Ranges (3 variants)
    // =========================================================================
    /// 2D bounding box (min_x, min_y, max_x, max_y).
    ///
    /// # Use Cases
    /// - Rectangle bounds
    /// - Collision detection
    /// - Selection areas
    ///
    /// # UI Hint
    /// Four inputs or visual rectangle selector.
    BoundingBox2D,

    /// 3D bounding box (min_x, min_y, min_z, max_x, max_y, max_z).
    ///
    /// # Use Cases
    /// - 3D collision detection
    /// - Culling volumes
    /// - Object bounds
    ///
    /// # UI Hint
    /// Six inputs or visual 3D box selector.
    BoundingBox3D,

    /// Range (min, max).
    ///
    /// # Use Cases
    /// - Value ranges
    /// - Numeric bounds
    /// - Sliders
    ///
    /// # UI Hint
    /// Two inputs labeled Min, Max or range slider.
    Range,

    // =========================================================================
    // Matrices (3 variants)
    // =========================================================================
    /// 2x2 matrix.
    ///
    /// # Use Cases
    /// - 2D transformations
    /// - Linear algebra
    ///
    /// # Components
    /// 4 elements (row-major or column-major).
    ///
    /// # UI Hint
    /// 2x2 grid of numeric inputs.
    Matrix2x2,

    /// 3x3 matrix.
    ///
    /// # Use Cases
    /// - 2D homogeneous transformations
    /// - 3D rotation matrices
    /// - Texture transformations
    ///
    /// # Components
    /// 9 elements (row-major or column-major).
    ///
    /// # UI Hint
    /// 3x3 grid of numeric inputs.
    Matrix3x3,

    /// 4x4 matrix.
    ///
    /// # Use Cases
    /// - 3D transformations
    /// - View/projection matrices
    /// - Bone transforms
    ///
    /// # Components
    /// 16 elements (row-major or column-major).
    ///
    /// # UI Hint
    /// 4x4 grid of numeric inputs or transform decomposition.
    Matrix4x4,

    // =========================================================================
    // Specialized (3 variants)
    // =========================================================================
    /// Tangent vector (tx, ty, tz, handedness).
    ///
    /// # Use Cases
    /// - Normal mapping
    /// - Mesh tangent space
    /// - Rendering
    ///
    /// # Components
    /// - tx, ty, tz: tangent direction
    /// - handedness: +1 or -1
    ///
    /// # UI Hint
    /// Four numeric inputs.
    Tangent,

    /// Bezier control points (2D).
    ///
    /// # Use Cases
    /// - Curve editing
    /// - Animation curves
    /// - Path design
    ///
    /// # Components
    /// Multiple 2D points (start, control1, control2, end).
    ///
    /// # UI Hint
    /// Visual curve editor.
    BezierCurve2D,

    /// Bezier control points (3D).
    ///
    /// # Use Cases
    /// - 3D curves
    /// - Animation paths
    /// - Splines
    ///
    /// # Components
    /// Multiple 3D points.
    ///
    /// # UI Hint
    /// 3D curve editor.
    BezierCurve3D,

    // =========================================================================
    // Custom (1 variant)
    // =========================================================================
    /// Custom vector type defined by the user.
    ///
    /// # Use Cases
    /// - Domain-specific vectors
    /// - Application-specific data
    ///
    /// # Example
    /// ```rust
    /// VectorSubtype::Custom("bone_weights".into())
    /// VectorSubtype::Custom("blend_shape_weights".into())
    /// ```
    Custom(String),
}

impl VectorSubtype {
    /// Create a Custom subtype.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::subtype::VectorSubtype;
    ///
    /// let subtype = VectorSubtype::custom("bone_weights");
    /// ```
    #[must_use]
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }

    /// Get the typical number of components for this vector type.
    ///
    /// Returns `None` for variable-size vectors.
    #[must_use]
    pub fn component_count(&self) -> Option<usize> {
        match self {
            Self::Vector2
            | Self::Position2D
            | Self::Offset2D
            | Self::Direction2D
            | Self::Scale2D
            | Self::Size2D
            | Self::TexCoord2D
            | Self::Range => Some(2),

            Self::Vector3
            | Self::Position3D
            | Self::Direction3D
            | Self::Normal
            | Self::Scale3D
            | Self::EulerAngles
            | Self::ColorRgb
            | Self::ColorHsv
            | Self::ColorHsl
            | Self::ColorLinearRgb
            | Self::ColorSrgb
            | Self::TexCoord3D => Some(3),

            Self::Vector4
            | Self::Quaternion
            | Self::AxisAngle
            | Self::ColorRgba
            | Self::Tangent
            | Self::BoundingBox2D => Some(4),

            Self::BoundingBox3D => Some(6),

            Self::Matrix2x2 => Some(4),
            Self::Matrix3x3 => Some(9),
            Self::Matrix4x4 => Some(16),

            Self::VectorN | Self::BezierCurve2D | Self::BezierCurve3D | Self::Custom(_) => None,
        }
    }

    /// Check if this is a color type.
    #[must_use]
    pub fn is_color(&self) -> bool {
        matches!(
            self,
            Self::ColorRgb
                | Self::ColorRgba
                | Self::ColorHsv
                | Self::ColorHsl
                | Self::ColorLinearRgb
                | Self::ColorSrgb
        )
    }

    /// Check if this is a position or direction type.
    #[must_use]
    pub fn is_spatial(&self) -> bool {
        matches!(
            self,
            Self::Vector2
                | Self::Vector3
                | Self::Vector4
                | Self::Position2D
                | Self::Position3D
                | Self::Direction2D
                | Self::Direction3D
                | Self::Normal
        )
    }

    /// Check if this is a rotation type.
    #[must_use]
    pub fn is_rotation(&self) -> bool {
        matches!(self, Self::EulerAngles | Self::Quaternion | Self::AxisAngle)
    }

    /// Check if this is a matrix type.
    #[must_use]
    pub fn is_matrix(&self) -> bool {
        matches!(self, Self::Matrix2x2 | Self::Matrix3x3 | Self::Matrix4x4)
    }

    /// Check if this is a bounded type (bounding box or range).
    #[must_use]
    pub fn is_bounds(&self) -> bool {
        matches!(
            self,
            Self::BoundingBox2D | Self::BoundingBox3D | Self::Range
        )
    }

    /// Check if this vector should typically be normalized.
    #[must_use]
    pub fn should_be_normalized(&self) -> bool {
        matches!(
            self,
            Self::Direction2D | Self::Direction3D | Self::Normal | Self::Quaternion
        )
    }

    /// Get typical component names for this vector type.
    #[must_use]
    pub fn component_names(&self) -> Option<&[&'static str]> {
        match self {
            Self::Vector2 | Self::Position2D | Self::Offset2D | Self::Direction2D => {
                Some(&["x", "y"])
            }
            Self::Vector3 | Self::Position3D | Self::Direction3D | Self::Normal => {
                Some(&["x", "y", "z"])
            }
            Self::Vector4 => Some(&["x", "y", "z", "w"]),
            Self::Scale2D | Self::Size2D => Some(&["width", "height"]),
            Self::Scale3D => Some(&["x", "y", "z"]),
            Self::EulerAngles => Some(&["pitch", "yaw", "roll"]),
            Self::Quaternion => Some(&["x", "y", "z", "w"]),
            Self::AxisAngle => Some(&["axis_x", "axis_y", "axis_z", "angle"]),
            Self::ColorRgb | Self::ColorLinearRgb | Self::ColorSrgb => {
                Some(&["red", "green", "blue"])
            }
            Self::ColorRgba => Some(&["red", "green", "blue", "alpha"]),
            Self::ColorHsv => Some(&["hue", "saturation", "value"]),
            Self::ColorHsl => Some(&["hue", "saturation", "lightness"]),
            Self::TexCoord2D => Some(&["u", "v"]),
            Self::TexCoord3D => Some(&["u", "v", "w"]),
            Self::Range => Some(&["min", "max"]),
            Self::BoundingBox2D => Some(&["min_x", "min_y", "max_x", "max_y"]),
            Self::BoundingBox3D => Some(&["min_x", "min_y", "min_z", "max_x", "max_y", "max_z"]),
            _ => None,
        }
    }

    /// Get typical value ranges for components.
    ///
    /// Returns (min, max) for each component if constrained.
    #[must_use]
    pub fn component_range(&self) -> Option<(f64, f64)> {
        match self {
            Self::ColorRgb
            | Self::ColorRgba
            | Self::ColorLinearRgb
            | Self::ColorSrgb
            | Self::TexCoord2D
            | Self::TexCoord3D => Some((0.0, 1.0)),
            Self::ColorHsv | Self::ColorHsl => None, // Hue is 0-360, others 0-1
            _ => None,
        }
    }
}

impl Default for VectorSubtype {
    fn default() -> Self {
        Self::Vector3
    }
}

impl fmt::Display for VectorSubtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vector2 => write!(f, "vector2"),
            Self::Vector3 => write!(f, "vector3"),
            Self::Vector4 => write!(f, "vector4"),
            Self::VectorN => write!(f, "vector_n"),
            Self::Position2D => write!(f, "position_2d"),
            Self::Position3D => write!(f, "position_3d"),
            Self::Offset2D => write!(f, "offset_2d"),
            Self::Direction2D => write!(f, "direction_2d"),
            Self::Direction3D => write!(f, "direction_3d"),
            Self::Normal => write!(f, "normal"),
            Self::Scale2D => write!(f, "scale_2d"),
            Self::Scale3D => write!(f, "scale_3d"),
            Self::Size2D => write!(f, "size_2d"),
            Self::EulerAngles => write!(f, "euler_angles"),
            Self::Quaternion => write!(f, "quaternion"),
            Self::AxisAngle => write!(f, "axis_angle"),
            Self::ColorRgb => write!(f, "color_rgb"),
            Self::ColorRgba => write!(f, "color_rgba"),
            Self::ColorHsv => write!(f, "color_hsv"),
            Self::ColorHsl => write!(f, "color_hsl"),
            Self::ColorLinearRgb => write!(f, "color_linear_rgb"),
            Self::ColorSrgb => write!(f, "color_srgb"),
            Self::TexCoord2D => write!(f, "texcoord_2d"),
            Self::TexCoord3D => write!(f, "texcoord_3d"),
            Self::BoundingBox2D => write!(f, "bbox_2d"),
            Self::BoundingBox3D => write!(f, "bbox_3d"),
            Self::Range => write!(f, "range"),
            Self::Matrix2x2 => write!(f, "matrix_2x2"),
            Self::Matrix3x3 => write!(f, "matrix_3x3"),
            Self::Matrix4x4 => write!(f, "matrix_4x4"),
            Self::Tangent => write!(f, "tangent"),
            Self::BezierCurve2D => write!(f, "bezier_2d"),
            Self::BezierCurve3D => write!(f, "bezier_3d"),
            Self::Custom(name) => write!(f, "custom({})", name),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        assert_eq!(VectorSubtype::default(), VectorSubtype::Vector3);
    }

    #[test]
    fn test_component_count() {
        assert_eq!(VectorSubtype::Vector2.component_count(), Some(2));
        assert_eq!(VectorSubtype::Vector3.component_count(), Some(3));
        assert_eq!(VectorSubtype::Vector4.component_count(), Some(4));
        assert_eq!(VectorSubtype::ColorRgb.component_count(), Some(3));
        assert_eq!(VectorSubtype::ColorRgba.component_count(), Some(4));
        assert_eq!(VectorSubtype::Matrix4x4.component_count(), Some(16));
        assert_eq!(VectorSubtype::VectorN.component_count(), None);
    }

    #[test]
    fn test_is_color() {
        assert!(VectorSubtype::ColorRgb.is_color());
        assert!(VectorSubtype::ColorRgba.is_color());
        assert!(VectorSubtype::ColorHsv.is_color());
        assert!(!VectorSubtype::Vector3.is_color());
        assert!(!VectorSubtype::Position3D.is_color());
    }

    #[test]
    fn test_is_spatial() {
        assert!(VectorSubtype::Vector3.is_spatial());
        assert!(VectorSubtype::Position3D.is_spatial());
        assert!(VectorSubtype::Direction3D.is_spatial());
        assert!(!VectorSubtype::ColorRgb.is_spatial());
        assert!(!VectorSubtype::Quaternion.is_spatial());
    }

    #[test]
    fn test_is_rotation() {
        assert!(VectorSubtype::EulerAngles.is_rotation());
        assert!(VectorSubtype::Quaternion.is_rotation());
        assert!(VectorSubtype::AxisAngle.is_rotation());
        assert!(!VectorSubtype::Vector3.is_rotation());
    }

    #[test]
    fn test_is_matrix() {
        assert!(VectorSubtype::Matrix2x2.is_matrix());
        assert!(VectorSubtype::Matrix3x3.is_matrix());
        assert!(VectorSubtype::Matrix4x4.is_matrix());
        assert!(!VectorSubtype::Vector3.is_matrix());
    }

    #[test]
    fn test_should_be_normalized() {
        assert!(VectorSubtype::Direction3D.should_be_normalized());
        assert!(VectorSubtype::Normal.should_be_normalized());
        assert!(VectorSubtype::Quaternion.should_be_normalized());
        assert!(!VectorSubtype::Position3D.should_be_normalized());
    }

    #[test]
    fn test_component_names() {
        assert_eq!(
            VectorSubtype::Vector3.component_names(),
            Some(&["x", "y", "z"] as &[&str])
        );
        assert_eq!(
            VectorSubtype::ColorRgb.component_names(),
            Some(&["red", "green", "blue"] as &[&str])
        );
        assert_eq!(
            VectorSubtype::EulerAngles.component_names(),
            Some(&["pitch", "yaw", "roll"] as &[&str])
        );
    }

    #[test]
    fn test_component_range() {
        assert_eq!(VectorSubtype::ColorRgb.component_range(), Some((0.0, 1.0)));
        assert_eq!(
            VectorSubtype::TexCoord2D.component_range(),
            Some((0.0, 1.0))
        );
        assert_eq!(VectorSubtype::Vector3.component_range(), None);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", VectorSubtype::Vector3), "vector3");
        assert_eq!(format!("{}", VectorSubtype::ColorRgb), "color_rgb");
        assert_eq!(format!("{}", VectorSubtype::Quaternion), "quaternion");
        assert_eq!(
            format!("{}", VectorSubtype::Custom("bone_weights".into())),
            "custom(bone_weights)"
        );
    }

    #[test]
    fn test_serialization() {
        let subtype = VectorSubtype::Vector3;
        let json = serde_json::to_string(&subtype).unwrap();
        assert_eq!(json, "\"vector3\"");

        let deserialized: VectorSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, VectorSubtype::Vector3);
    }

    #[test]
    fn test_custom_serialization() {
        let subtype = VectorSubtype::Custom("blend_weights".into());
        let json = serde_json::to_string(&subtype).unwrap();
        let deserialized: VectorSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, subtype);
    }

    #[test]
    fn test_clone() {
        let subtype = VectorSubtype::ColorRgba;
        let cloned = subtype.clone();
        assert_eq!(subtype, cloned);
    }

    #[test]
    fn test_eq() {
        assert_eq!(VectorSubtype::Vector3, VectorSubtype::Vector3);
        assert_ne!(VectorSubtype::Vector3, VectorSubtype::Vector2);
        assert_eq!(
            VectorSubtype::Custom("test".into()),
            VectorSubtype::Custom("test".into())
        );
        assert_ne!(
            VectorSubtype::Custom("test1".into()),
            VectorSubtype::Custom("test2".into())
        );
    }
}
