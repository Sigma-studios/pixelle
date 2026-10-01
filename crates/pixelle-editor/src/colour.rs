//! Showing colours as they are.
//!
//! Palette entries are sRGB. On a wide-gamut display (every recent Mac's is Display P3) a window
//! has to say so, or macOS hands its values to the panel untouched and every colour comes out more
//! saturated than it is — which in a pixel-art editor means picking colours that look different
//! in the game. Bevy renders to an `…UnormSrgb` surface, which macOS colour-matches as sRGB; egui
//! renders to a plain `…Unorm` one, which it does not. So the editor tags its Metal layer as sRGB
//! itself. wgpu clears the tag whenever it reconfigures the surface (on every resize), so this is
//! checked every frame; it costs two message sends.

#[cfg(target_os = "macos")]
pub fn tag_srgb(frame: &eframe::Frame) {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
    use objc2_quartz_core::{CALayer, CAMetalLayer};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// The Metal layer under `layer`: wgpu adds it as a sublayer of the view's own.
    fn find(layer: Retained<CALayer>) -> Option<Retained<CAMetalLayer>> {
        let layer = match layer.downcast::<CAMetalLayer>() {
            Ok(metal) => return Some(metal),
            Err(layer) => layer,
        };
        let subs: Option<Retained<AnyObject>> = unsafe { msg_send![&*layer, sublayers] };
        let subs = subs?;
        let count: usize = unsafe { msg_send![&*subs, count] };
        (0..count).find_map(|i| {
            let sub: Retained<CALayer> = unsafe { msg_send![&*subs, objectAtIndex: i] };
            find(sub)
        })
    }

    let Ok(handle) = frame.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    let view = h.ns_view.as_ptr().cast::<AnyObject>();
    // SAFETY: the handle is the live NSView of the window being drawn, on the main thread.
    let layer: Option<Retained<CALayer>> = unsafe { msg_send![&*view, layer] };
    let Some(metal) = layer.and_then(find) else { return };
    if metal.colorspace().is_none() {
        // SAFETY: `kCGColorSpaceSRGB` is a constant CoreGraphics provides.
        let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        metal.setColorspace(srgb.as_deref());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn tag_srgb(_: &eframe::Frame) {}
