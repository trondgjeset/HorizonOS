//! Desktop widgets whose state and painting are shared by the shell compositor.
//!
//! NOTE: Open-source Contribution Area: add future desktop elements as small
//! modules here and keep their hit testing aligned with their rendered bounds.

/// Traditional desktop shortcuts and their icon-level click tracking.
///
/// NOTE: Open-source Contribution Area: extend this module with user-defined
/// shortcuts only after persistent shortcut storage is available.
pub mod icons;
