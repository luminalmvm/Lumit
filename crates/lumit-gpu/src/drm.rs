//! Which graphics card a context is on, in the kernel's own words.
//!
//! # In plain terms
//!
//! A laptop with two graphics cards has two of everything, and the two halves
//! of Lumit don't choose between them the same way. The renderer asks Vulkan
//! for the fast card. Flutter draws the window with whichever card the desktop
//! handed it, which on these laptops is the slow one. The Viewer's picture is
//! passed from the first to the second without a copy (see
//! [`crate::shared_linux`]), and that only works when both are the *same*
//! card, or two cards whose drivers know how to share one. NVIDIA's driver
//! doesn't. On an Intel + NVIDIA laptop the Intel driver didn't refuse the
//! hand-off, it accepted it and then killed the process on the first draw.
//!
//! So each side has to be able to say which card it is on, in a form the two
//! can compare. Names are no use for that ("NVIDIA GeForce RTX 3050" against
//! "Mesa Intel(R) UHD Graphics" says they differ, but two identical cards
//! would say they are the same). Linux already has an unambiguous name for a
//! graphics card: its **device node**, the file under `/dev/dri/`, which the
//! kernel identifies by a pair of numbers, *major* and *minor*. Vulkan reports
//! the pair for the card it is using (`VK_EXT_physical_device_drm`), EGL
//! reports the file for the card Flutter is using, and a `stat()` of that file
//! gives the same pair back.
//!
//! A card has up to two nodes: a **primary** node (`/dev/dri/card0`), which
//! can also drive a display, and a **render** node (`/dev/dri/renderD128`),
//! which can only draw. They are different files with different numbers, so a
//! primary node must only ever be compared with a primary node and a render
//! node with a render node. Both are carried for that reason.
//!
//! This module is only the description. It is plain data with no Vulkan in it,
//! so it builds and is tested on every platform. The Vulkan query that fills
//! it in is [`crate::shared_linux`]'s, and the comparison is made by the Linux
//! runner (`flutter_ui/linux/runner/gpu_device_match.h`), the only place that
//! knows which card Flutter landed on.
//!
//! Nothing here runs on a particular thread: it is values, made once as a
//! context opens and copied from then on.

/// One DRM device node, as the kernel numbers it: what `stat()` reports as the
/// major and minor of `/dev/dri/card0` or `/dev/dri/renderD128`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DrmNode {
    pub major: u32,
    pub minor: u32,
}

impl std::fmt::Display for DrmNode {
    /// `226:129`, the form `ls -l /dev/dri` prints, so a line in a bug report
    /// can be matched against the machine by eye.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.major, self.minor)
    }
}

/// The device nodes of the card a context is on. Either may be missing: a
/// driver that does not report them at all leaves both `None`, and that means
/// **not known**, never "a different card".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrmDevice {
    /// The node that can drive a display (`/dev/dri/card*`).
    pub primary: Option<DrmNode>,
    /// The node that can only draw (`/dev/dri/renderD*`).
    pub render: Option<DrmNode>,
}

impl DrmDevice {
    /// Nothing is known about the card: not Linux, not Vulkan, or a driver
    /// without `VK_EXT_physical_device_drm`.
    pub const UNKNOWN: Self = Self {
        primary: None,
        render: None,
    };

    /// From the fields of `VkPhysicalDeviceDrmPropertiesEXT`, passed as plain
    /// numbers so this can be tested where there is no Vulkan.
    ///
    /// Vulkan reports each number as a signed 64-bit integer and says whether
    /// the pair is there at all with a flag beside it. A pair is kept only when
    /// its flag is set *and* both numbers fit a device number. Anything else is
    /// treated as not reported, because a nonsense node compared against a real
    /// one would read as two different cards and switch the Viewer off on a
    /// machine with nothing wrong with it.
    #[must_use]
    pub fn from_vulkan(
        has_primary: bool,
        primary: (i64, i64),
        has_render: bool,
        render: (i64, i64),
    ) -> Self {
        let node = |present: bool, (major, minor): (i64, i64)| {
            if !present {
                return None;
            }
            Some(DrmNode {
                major: u32::try_from(major).ok()?,
                minor: u32::try_from(minor).ok()?,
            })
        };
        Self {
            primary: node(has_primary, primary),
            render: node(has_render, render),
        }
    }

    /// Whether the driver named the card at all.
    #[must_use]
    pub fn is_known(&self) -> bool {
        self.primary.is_some() || self.render.is_some()
    }
}

impl std::fmt::Display for DrmDevice {
    /// The tail of the "adapter selected" line. It says so when nothing was
    /// reported, because "why was the mismatch not caught" is answered by that
    /// and by nothing else in the log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.render, self.primary) {
            (Some(render), Some(primary)) => {
                write!(f, "DRM render node {render}, primary node {primary}")
            }
            (Some(render), None) => write!(f, "DRM render node {render}"),
            (None, Some(primary)) => write!(f, "DRM primary node {primary}"),
            (None, None) => write!(f, "DRM node not reported"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DrmDevice, DrmNode};

    /// The laptop in the crash report: NVIDIA on `card1` and `renderD129`.
    #[test]
    fn a_reported_pair_is_kept() {
        let device = DrmDevice::from_vulkan(true, (226, 1), true, (226, 129));
        assert_eq!(
            device.primary,
            Some(DrmNode {
                major: 226,
                minor: 1
            })
        );
        assert_eq!(
            device.render,
            Some(DrmNode {
                major: 226,
                minor: 129
            })
        );
        assert!(device.is_known());
        assert_eq!(
            device.to_string(),
            "DRM render node 226:129, primary node 226:1"
        );
    }

    /// A flag that is off means the numbers beside it are whatever the driver
    /// left there, and must not be read.
    #[test]
    fn an_unflagged_pair_is_not_known() {
        let device = DrmDevice::from_vulkan(false, (226, 0), false, (226, 128));
        assert_eq!(device, DrmDevice::UNKNOWN);
        assert!(!device.is_known());
        assert_eq!(device.to_string(), "DRM node not reported");
    }

    /// One node without the other is ordinary (a driver with no render node),
    /// and the one that is there is still kept.
    #[test]
    fn one_node_is_kept_without_the_other() {
        let device = DrmDevice::from_vulkan(true, (226, 0), false, (0, 0));
        assert!(device.is_known());
        assert_eq!(device.render, None);
        assert_eq!(device.to_string(), "DRM primary node 226:0");
    }

    /// **The case that would switch a healthy Viewer off.** A number that
    /// can't be a device number is "not reported", not a card of its own.
    #[test]
    fn a_number_that_is_no_device_number_is_not_known() {
        let negative = DrmDevice::from_vulkan(true, (-1, 0), true, (226, -5));
        assert_eq!(negative, DrmDevice::UNKNOWN);
        let huge = DrmDevice::from_vulkan(false, (0, 0), true, (i64::MAX, 128));
        assert_eq!(huge, DrmDevice::UNKNOWN);
    }
}
