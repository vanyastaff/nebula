//! Nebula theme — technical, non-generic palette.
//!
//! Base #080A0F, amber #F0A030 (energy/execution), cyan #00C8E0 (flow/data).
//! IBM Plex Mono for IDs/code, IBM Plex Sans for UI.

/// Nebula palette — dark, technical, energy + flow.
#[allow(dead_code)]
pub mod nebula {
    use gpui::{Rgba, rgb};

    /// Base background — #080A0F
    pub fn background() -> Rgba {
        rgb(0x08_0a_0f)
    }

    /// Foreground text — #E4E4E7
    pub fn foreground() -> Rgba {
        rgb(0xe4_e4_e7)
    }

    /// Card background — #0d1117
    pub fn card() -> Rgba {
        rgb(0x0d_11_17)
    }

    /// Muted background — #1a1f2e
    pub fn muted() -> Rgba {
        rgb(0x1a_1f_2e)
    }

    /// Muted foreground — #71717a
    pub fn muted_foreground() -> Rgba {
        rgb(0x71_71_7a)
    }

    /// Border — #27272a
    pub fn border() -> Rgba {
        rgb(0x27_27_2a)
    }

    /// Input border — #3f3f46
    pub fn input() -> Rgba {
        rgb(0x3f_3f_46)
    }

    /// Primary (energy/execution) — #F0A030 amber
    pub fn primary() -> Rgba {
        rgb(0xf0_a0_30)
    }

    /// Primary foreground — dark on amber
    pub fn primary_foreground() -> Rgba {
        rgb(0x08_0a_0f)
    }

    /// Secondary — #1a1f2e
    pub fn secondary() -> Rgba {
        rgb(0x1a_1f_2e)
    }

    /// Destructive — #ef4444
    pub fn destructive() -> Rgba {
        rgb(0xef_44_44)
    }

    /// Success — #22c55e
    pub fn success() -> Rgba {
        rgb(0x22_c5_5e)
    }

    /// Warning — #f59e0b
    pub fn warning() -> Rgba {
        rgb(0xf5_9e_0b)
    }

    /// Accent — #3f3f46
    pub fn accent() -> Rgba {
        rgb(0x3f_3f_46)
    }

    /// Flow/data — #00C8E0 cyan
    pub fn flow() -> Rgba {
        rgb(0x00_c8_e0)
    }

    /// Link — cyan for flow/data
    pub fn link() -> Rgba {
        rgb(0x00_c8_e0)
    }

    /// Violet (avatar, badges, manual trigger)
    pub fn violet() -> Rgba {
        rgb(0x7c_3a_ed)
    }

    /// Blue for IDs, webhook, links (#60a5fa) — monitor header/exec ID
    pub fn id_blue() -> Rgba {
        rgb(0x60_a5_fa)
    }

    /// Dark green for cron/success tint (#0a1f10)
    pub fn success_bg() -> Rgba {
        rgb(0x0a_1f_10)
    }

    /// Border green (#14532d)
    pub fn success_border() -> Rgba {
        rgb(0x14_53_2d)
    }

    /// Sidebar accent (active item bg)
    pub fn sidebar_accent() -> Rgba {
        rgb(0x1a_1f_2e)
    }

    /// Sidebar active — cyan
    pub fn sidebar_active() -> Rgba {
        rgb(0x00_c8_e0)
    }

    /// Ring (focus)
    pub fn ring() -> Rgba {
        rgb(0x71_71_7a)
    }
}

/// Alias for backward compatibility.
pub mod shadcn {
    pub use super::nebula::*;
}

/// Font families — IBM Plex for technical, clean typography.
#[allow(dead_code)]
pub mod fonts {
    /// UI text — IBM Plex Sans
    pub const UI: &str = "IBM Plex Sans";
    /// IDs, code — IBM Plex Mono
    pub const MONO: &str = "IBM Plex Mono";
}
