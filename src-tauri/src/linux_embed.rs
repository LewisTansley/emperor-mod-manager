//! Linux-only overlay so the Assist child webview can be positioned
//! inside the Downloads host frame.
//!
//! Tauri packs Linux child webviews into the window `GtkBox` with expand=true,
//! so `set_bounds` is ignored. We wrap the box in a `GtkOverlay` and reparent
//! Assist onto that overlay. The WebView is the overlay child, so the window
//! GTK raises is the same window that paints and receives clicks.
//! Moving its `GdkWindow` by hand leaves the page visible but drops input.

use std::cell::{Cell, RefCell};
use std::sync::mpsc;

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use gtk::OverlaySignals;

use crate::assist::AssistBounds;

thread_local! {
    static HOST: RefCell<Option<AssistOverlayHost>> = RefCell::new(None);
    static GEOM: RefCell<AssistBounds> = RefCell::new(AssistBounds::default());
    /// Overlay child currently being positioned. Read from `get-child-position`,
    /// which can run while `HOST` is already borrowed.
    static ASSIST: RefCell<Option<gtk::Widget>> = RefCell::new(None);
    /// Whether the current assist view should receive pointer events.
    /// Read from `size-allocate`, which can run while `HOST` is borrowed.
    static ASSIST_ACCEPTS_INPUT: Cell<bool> = const { Cell::new(false) };
    static INPUT_REFRESH_QUEUED: Cell<bool> = const { Cell::new(false) };
}

struct AssistOverlayHost {
    overlay: gtk::Overlay,
    assist: Option<gtk::Widget>,
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
    // Keep the WebKit widget mapped. Hiding uses opacity and an empty input
    // region so the first viewport is the real host rect, not a 1×1 placeholder.
    widget.show();
}

/// `GtkOverlay` pass-through only marks the wrapper window. WebKit has its own
/// `GdkWindow`, so hide must punch that window (and every child) out of hit testing.
fn set_window_input(window: &gtk::gdk::Window, accept: bool) {
    window.set_pass_through(!accept);
    if accept {
        unsafe {
            gtk::gdk::ffi::gdk_window_input_shape_combine_region(
                window.to_glib_none().0,
                std::ptr::null(),
                0,
                0,
            );
        }
    } else {
        let empty = gtk::cairo::Region::create();
        window.input_shape_combine_region(&empty, 0, 0);
    }
    for child in window.children() {
        set_window_input(&child, accept);
    }
}

fn set_widget_input(widget: &gtk::Widget, accept: bool) {
    let Some(window) = widget.window() else {
        return;
    };
    set_window_input(&window, accept);
}

fn schedule_input_refresh() {
    let queued = INPUT_REFRESH_QUEUED.with(|flag| {
        if flag.get() {
            false
        } else {
            flag.set(true);
            true
        }
    });
    if !queued {
        return;
    }
    gtk::glib::idle_add_local_once(|| {
        INPUT_REFRESH_QUEUED.with(|flag| flag.set(false));
        let Some(widget) = ASSIST.with(|slot| slot.borrow().clone()) else {
            return;
        };
        let accept = ASSIST_ACCEPTS_INPUT.with(|flag| flag.get());
        set_widget_input(&widget, accept);
    });
}

fn watch_assist_input(widget: &gtk::Widget) {
    widget.connect_size_allocate(|widget, _| {
        let current = ASSIST.with(|slot| slot.borrow().clone());
        if current.as_ref() != Some(widget) {
            set_widget_input(widget, false);
            return;
        }
        let accept = ASSIST_ACCEPTS_INPUT.with(|flag| flag.get());
        set_widget_input(widget, accept);
        schedule_input_refresh();
    });
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

fn pixel_size(bounds: AssistBounds) -> (i32, i32) {
    (
        bounds.width.max(1.0).floor() as i32,
        bounds.height.max(1.0).floor() as i32,
    )
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
    gtk_window.add(&overlay);
    overlay.show_all();

    let position_overlay = overlay.clone();
    let position_main = main_webview.clone();
    overlay.connect_get_child_position(move |_overlay, widget| {
        let assist = ASSIST.with(|slot| slot.borrow().clone());
        let Some(assist) = assist else {
            return None;
        };
        if widget != &assist {
            return None;
        }
        let bounds = GEOM.with(|geom| *geom.borrow());
        let (ox, oy) = position_main
            .translate_coordinates(&position_overlay, 0, 0)
            .unwrap_or((0, 0));
        let x = ox + bounds.x.round() as i32;
        let y = oy + bounds.y.round() as i32;
        let (w, h) = pixel_size(bounds);
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

    HOST.with(|slot| {
        *slot.borrow_mut() = Some(AssistOverlayHost {
            overlay,
            assist: None,
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
    GEOM.with(|geom| *geom.borrow_mut() = host.bounds);
    let shown = host.visible && host.assist.is_some();
    ASSIST_ACCEPTS_INPUT.with(|flag| flag.set(shown));
    if let Some(assist) = &host.assist {
        let (w, h) = pixel_size(host.bounds);
        assist.set_size_request(w, h);
        assist.set_opacity(if shown { 1.0 } else { 0.0 });
        host.overlay.set_overlay_pass_through(assist, !shown);
        // Stay mapped at the host size. Unmapping or a 1×1 size blanks the next show.
        assist.show();
        if shown {
            enable_assist_input(assist);
        } else {
            assist.set_can_focus(false);
            assist.set_sensitive(false);
        }
        set_widget_input(assist, shown);
        schedule_input_refresh();
        log::debug!(
            "assist overlay bounds requested={}x{}@{},{} shown={}",
            w,
            h,
            host.bounds.x,
            host.bounds.y,
            shown
        );
    }
    host.overlay.clone()
}

fn queue_overlay_resize(overlay: gtk::Overlay) {
    overlay.queue_resize();
}

fn remember_assist(widget: Option<gtk::Widget>) {
    ASSIST.with(|slot| *slot.borrow_mut() = widget);
}

/// Wrap the window content in an overlay before Assist is packed into the GtkBox.
pub fn prepare(window: &tauri::Window) -> Result<(), String> {
    let window = window.clone();
    run_on_gtk(&window, {
        let window = window.clone();
        move || ensure_host(&window)
    })?
}

/// Reparent the Assist webview onto the overlay.
///
/// `visible` is applied before the view is realized, at `bounds` rather than
/// a 1×1 placeholder. The view must still be on `about:blank`.
pub fn attach_assist(
    window: &tauri::Window,
    bounds: AssistBounds,
    visible: bool,
) -> Result<(), String> {
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
                if let Some(old) = host.assist.take() {
                    set_widget_input(&old, false);
                    if old.parent().is_some() {
                        host.overlay.remove(&old);
                    }
                }
                // Start, not Fill: a missed get-child-position must not cover the window.
                assist.set_hexpand(false);
                assist.set_vexpand(false);
                assist.set_halign(gtk::Align::Start);
                assist.set_valign(gtk::Align::Start);
                watch_assist_input(&assist);
                host.assist = Some(assist.clone());
                host.bounds = bounds;
                host.visible = visible;
                remember_assist(Some(assist.clone()));
                GEOM.with(|geom| *geom.borrow_mut() = bounds);
                ASSIST_ACCEPTS_INPUT.with(|flag| flag.set(visible));
                // Pointer and bounds are set first so the initial allocation is the host rect.
                host.overlay.add_overlay(&assist);
                let overlay = apply_host_geometry(host);
                if !assist.is_realized() {
                    assist.realize();
                }
                set_widget_input(&assist, host.visible);
                schedule_input_refresh();
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
                host.visible = visible && host.assist.is_some();
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
                ASSIST_ACCEPTS_INPUT.with(|flag| flag.set(false));
                if let Some(assist) = host.assist.clone() {
                    set_widget_input(&assist, false);
                    host.overlay.set_overlay_pass_through(&assist, true);
                    if assist.parent().is_some() {
                        host.overlay.remove(&assist);
                    }
                }
                host.assist = None;
                remember_assist(None);
                apply_host_geometry(host)
            })
        });
        if let Some(overlay) = overlay {
            queue_overlay_resize(overlay);
        }
    })
}
