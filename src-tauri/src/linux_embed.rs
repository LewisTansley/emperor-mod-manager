//! Linux-only overlay so the Assist child webview can be positioned
//! inside the Downloads host frame.
//!
//! Tauri packs Linux child webviews into the window `GtkBox` with expand=true,
//! so `set_bounds` is ignored. We wrap the box in a `GtkOverlay` and reparent
//! Assist into an `GtkEventBox`. The event box has its own `GdkWindow`, which
//! the overlay keeps above the main webview. WebKit is sized by that allocation.
//! Moving its `GdkWindow` by hand leaves the page visible but drops input.

use std::cell::RefCell;
use std::sync::mpsc;

use gtk::prelude::*;
use gtk::OverlaySignals;

use crate::assist::AssistBounds;

thread_local! {
    static HOST: RefCell<Option<AssistOverlayHost>> = RefCell::new(None);
    static GEOM: RefCell<(AssistBounds, bool)> = RefCell::new((AssistBounds::default(), false));
}

struct AssistOverlayHost {
    overlay: gtk::Overlay,
    event_box: gtk::EventBox,
    inner: gtk::Box,
    main_webview: gtk::Widget,
    bounds: AssistBounds,
    visible: bool,
}

fn is_webkit_view(widget: &gtk::Widget) -> bool {
    widget.type_().name().contains("WebView")
}

fn run_on_gtk<T: Send + 'static>(
    window: &tauri::Window,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::channel();
    window
        .run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())
}

fn find_main_webview(vbox: &gtk::Box) -> Result<gtk::Widget, String> {
    vbox.children()
        .into_iter()
        .find(|child| is_webkit_view(child))
        .ok_or_else(|| "Main webview widget not found".to_string())
}

fn enable_assist_input(widget: &gtk::Widget) {
    widget.set_can_focus(true);
    widget.set_focus_on_click(true);
    widget.set_sensitive(true);
    // Keep the WebKit widget's own visible flag set. Hiding parks the event
    // box off-screen; unmapping WebKit itself leaves a later show() blank.
    widget.show();
}

fn disable_accelerated_compositing(widget: &gtk::Widget) {
    let Some(webview) = widget.downcast_ref::<webkit2gtk::WebView>() else {
        log::warn!("assist overlay: widget is not a WebKit view");
        return;
    };
    if let Some(settings) = webkit2gtk::WebViewExt::settings(webview) {
        webkit2gtk::SettingsExt::set_hardware_acceleration_policy(
            &settings,
            webkit2gtk::HardwareAccelerationPolicy::Never,
        );
    }
}

fn assist_attached(host: &AssistOverlayHost) -> bool {
    !host.inner.children().is_empty()
}

fn make_assist_slot(overlay: &gtk::Overlay) -> (gtk::EventBox, gtk::Box) {
    let event_box = gtk::EventBox::new();
    event_box.set_visible_window(true);
    event_box.set_above_child(false);
    event_box.set_can_focus(false);
    event_box.set_hexpand(false);
    event_box.set_vexpand(false);
    event_box.set_halign(gtk::Align::Start);
    event_box.set_valign(gtk::Align::Start);

    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    inner.set_hexpand(true);
    inner.set_vexpand(true);
    event_box.add(&inner);

    let overlay_map = overlay.clone();
    event_box.connect_map(move |_| {
        overlay_map.queue_resize();
    });

    (event_box, inner)
}

fn ensure_host(window: &tauri::Window) -> Result<(), String> {
    let already = HOST.with(|slot| slot.borrow().is_some());
    if already {
        return Ok(());
    }

    let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
    let vbox = window.default_vbox().map_err(|e| e.to_string())?;
    let main_webview = find_main_webview(&vbox)?;

    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);

    gtk_window.remove(&vbox);
    overlay.add(&vbox);

    let (event_box, inner) = make_assist_slot(&overlay);
    overlay.add_overlay(&event_box);
    // Parked until a page is attached and Downloads asks to show it.
    overlay.set_overlay_pass_through(&event_box, true);

    gtk_window.add(&overlay);
    overlay.show_all();
    event_box.show_all();

    let position_slot = event_box.clone();
    let position_overlay = overlay.clone();
    let position_main = main_webview.clone();
    overlay.connect_get_child_position(move |_overlay, widget| {
        if widget != position_slot.upcast_ref::<gtk::Widget>() {
            return None;
        }
        let (bounds, visible) = GEOM.with(|geom| *geom.borrow());
        if !visible {
            return Some(gtk::Rectangle::new(-8, -8, 1, 1));
        }
        let (ox, oy) = position_main
            .translate_coordinates(&position_overlay, 0, 0)
            .unwrap_or((0, 0));
        let x = ox + bounds.x.round() as i32;
        let y = oy + bounds.y.round() as i32;
        let w = bounds.width.max(1.0).floor() as i32;
        let h = bounds.height.max(1.0).floor() as i32;
        log::debug!(
            "assist overlay position final={}x{}@{},{} (main_offset={}x{}, bounds={}x{}@{},{})",
            w,
            h,
            x,
            y,
            ox,
            oy,
            bounds.width,
            bounds.height,
            bounds.x,
            bounds.y
        );
        Some(gtk::Rectangle::new(x, y, w, h))
    });

    let overlay_focus = overlay.clone();
    gtk_window.connect_focus_in_event(move |_, _| {
        overlay_focus.queue_resize();
        gtk::glib::Propagation::Proceed
    });
    let overlay_configure = overlay.clone();
    gtk_window.connect_configure_event(move |_, _| {
        overlay_configure.queue_resize();
        false
    });

    HOST.with(|slot| {
        *slot.borrow_mut() = Some(AssistOverlayHost {
            overlay,
            event_box,
            inner,
            main_webview,
            bounds: AssistBounds::default(),
            visible: false,
        });
    });
    Ok(())
}

fn find_assist_in_vbox(vbox: &gtk::Box, main_webview: &gtk::Widget) -> Option<gtk::Widget> {
    vbox.children()
        .into_iter()
        .find(|child| is_webkit_view(child) && child != main_webview)
}

fn apply_host_geometry(host: &AssistOverlayHost) -> gtk::Overlay {
    let shown = host.visible && assist_attached(host);
    GEOM.with(|geom| *geom.borrow_mut() = (host.bounds, shown));
    host.overlay
        .set_overlay_pass_through(&host.event_box, !shown);
    if shown {
        let w = host.bounds.width.max(1.0).floor() as i32;
        let h = host.bounds.height.max(1.0).floor() as i32;
        host.event_box.set_size_request(w, h);
        for child in host.inner.children() {
            enable_assist_input(&child);
        }
        log::debug!(
            "assist overlay bounds requested={}x{}@{},{} children={}",
            w,
            h,
            host.bounds.x,
            host.bounds.y,
            host.inner.children().len()
        );
    } else {
        host.event_box.set_size_request(1, 1);
        log::debug!(
            "assist overlay parked requested={}x{}",
            host.bounds.width,
            host.bounds.height
        );
    }
    // Stay mapped. Showing again is a resize back over the host rect.
    host.event_box.show_all();
    host.overlay.clone()
}

fn queue_overlay_resize(overlay: gtk::Overlay) {
    overlay.queue_resize();
}

/// Wrap the window content in an overlay before Assist is packed into the GtkBox.
pub fn prepare(window: &tauri::Window) -> Result<(), String> {
    let window = window.clone();
    run_on_gtk(&window, {
        let window = window.clone();
        move || ensure_host(&window)
    })?
}

/// Reparent the Assist webview into the overlay event box.
///
/// The view must still be on `about:blank`. Callers navigate to the real page
/// after this returns, so WebKit realizes in its final parent.
pub fn attach_assist(window: &tauri::Window, bounds: AssistBounds) -> Result<(), String> {
    let window = window.clone();
    run_on_gtk(&window, {
        let window = window.clone();
        move || {
            ensure_host(&window)?;
            HOST.with(|slot| {
                let mut slot = slot.borrow_mut();
                let host = slot
                    .as_mut()
                    .ok_or_else(|| "Assist overlay host missing".to_string())?;
                let vbox = window.default_vbox().map_err(|e| e.to_string())?;
                let Some(assist) = find_assist_in_vbox(&vbox, &host.main_webview) else {
                    return Err("Assist webview was not packed into the window box".to_string());
                };
                vbox.remove(&assist);
                if assist.is_realized() {
                    assist.unrealize();
                }
                disable_accelerated_compositing(&assist);
                for child in host.inner.children() {
                    host.inner.remove(&child);
                }
                assist.set_hexpand(true);
                assist.set_vexpand(true);
                assist.set_halign(gtk::Align::Fill);
                assist.set_valign(gtk::Align::Fill);
                enable_assist_input(&assist);
                host.inner.pack_start(&assist, true, true, 0);
                host.bounds = bounds;
                host.event_box.show_all();
                if !assist.is_realized() {
                    assist.realize();
                }
                let overlay = apply_host_geometry(host);
                drop(slot);
                queue_overlay_resize(overlay);
                Ok(())
            })
        }
    })?
}

pub fn apply_bounds(window: &tauri::Window, bounds: AssistBounds) -> Result<(), String> {
    let window = window.clone();
    run_on_gtk(&window, move || {
        let overlay = HOST.with(|slot| {
            slot.borrow_mut().as_mut().map(|host| {
                host.bounds = bounds;
                // Do not derive visibility from size — that undoes hide-on-navigate.
                apply_host_geometry(host)
            })
        });
        if let Some(overlay) = overlay {
            queue_overlay_resize(overlay);
        }
    })
}

pub fn set_visible(window: &tauri::Window, visible: bool) -> Result<(), String> {
    let window = window.clone();
    run_on_gtk(&window, move || {
        let overlay = HOST.with(|slot| {
            slot.borrow_mut().as_mut().map(|host| {
                host.visible = visible && assist_attached(host);
                for child in host.inner.children() {
                    enable_assist_input(&child);
                }
                apply_host_geometry(host)
            })
        });
        if let Some(overlay) = overlay {
            queue_overlay_resize(overlay);
        }
    })
}

pub fn hide_and_clear(window: &tauri::Window) -> Result<(), String> {
    let window = window.clone();
    run_on_gtk(&window, move || {
        let overlay = HOST.with(|slot| {
            slot.borrow_mut().as_mut().map(|host| {
                host.visible = false;
                apply_host_geometry(host)
            })
        });
        if let Some(overlay) = overlay {
            queue_overlay_resize(overlay);
        }
    })
}
