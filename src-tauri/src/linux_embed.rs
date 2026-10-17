//! Linux-only overlay so the Assist child webview can be positioned
//! inside the Downloads host frame.
//!
//! Tauri packs Linux child webviews into the window `GtkBox` with expand=true,
//! so `set_bounds` is ignored. We wrap the box in a `GtkOverlay` and reparent
//! Assist into a `GtkFixed`. The fixed is positioned over the host rect.
//! WebKit is sized with GTK allocation only — moving its `GdkWindow` by hand
//! leaves the page visible but drops pointer and keyboard input.

use std::cell::{Cell, RefCell};
use std::sync::mpsc;

use gtk::prelude::*;
use gtk::OverlaySignals;

use crate::assist::AssistBounds;

thread_local! {
    static HOST: RefCell<Option<AssistOverlayHost>> = RefCell::new(None);
    static GEOM: RefCell<(AssistBounds, bool)> = RefCell::new((AssistBounds::default(), false));
    static CLAMPING: Cell<bool> = const { Cell::new(false) };
}

struct AssistOverlayHost {
    overlay: gtk::Overlay,
    fixed: gtk::Fixed,
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
    // Keep the WebKit widget's own visible flag set. Hiding is done by the
    // overlay fixed; unmapping WebKit itself leaves a later show() blank.
    widget.show();
}

fn raise_widget_window(widget: &gtk::Widget) {
    if widget.is_realized() {
        if let Some(win) = widget.window() {
            win.raise();
        }
    }
}

fn size_assist_children(fixed: &gtk::Fixed, bounds: AssistBounds) {
    if CLAMPING.get() {
        return;
    }
    CLAMPING.set(true);
    let w = bounds.width.max(1.0).floor() as i32;
    let h = bounds.height.max(1.0).floor() as i32;
    let alloc = gtk::Allocation::new(0, 0, w, h);
    for child in fixed.children() {
        child.set_hexpand(false);
        child.set_vexpand(false);
        child.set_halign(gtk::Align::Fill);
        child.set_valign(gtk::Align::Fill);
        child.set_size_request(w, h);
        fixed.move_(&child, 0, 0);
        child.size_allocate(&alloc);
        enable_assist_input(&child);
        raise_widget_window(&child);
        let ca = child.allocation();
        log::debug!(
            "assist overlay child alloc={}x{}@{},{} has_window={} realized={} requested={}x{}",
            ca.width(),
            ca.height(),
            ca.x(),
            ca.y(),
            child.has_window(),
            child.is_realized(),
            w,
            h
        );
    }
    CLAMPING.set(false);
}

fn make_assist_fixed() -> gtk::Fixed {
    let fixed = gtk::Fixed::new();
    fixed.set_hexpand(false);
    fixed.set_vexpand(false);
    fixed.set_halign(gtk::Align::Start);
    fixed.set_valign(gtk::Align::Start);
    fixed.connect_size_allocate(|fixed, _alloc| {
        let (bounds, visible) = GEOM.with(|geom| *geom.borrow());
        if !visible {
            return;
        }
        size_assist_children(fixed, bounds);
    });
    fixed.connect_map(|fixed| {
        for child in fixed.children() {
            enable_assist_input(&child);
            raise_widget_window(&child);
        }
    });
    fixed
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

    let fixed = make_assist_fixed();
    overlay.add_overlay(&fixed);
    overlay.set_overlay_pass_through(&fixed, false);

    gtk_window.add(&overlay);
    overlay.show_all();
    fixed.hide();

    let position_fixed = fixed.clone();
    let position_overlay = overlay.clone();
    let position_main = main_webview.clone();
    overlay.connect_get_child_position(move |_overlay, widget| {
        if widget != position_fixed.upcast_ref::<gtk::Widget>() {
            return None;
        }
        let (bounds, visible) = GEOM.with(|geom| *geom.borrow());
        if !visible {
            return Some(gtk::Rectangle::new(0, 0, 1, 1));
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

    HOST.with(|slot| {
        *slot.borrow_mut() = Some(AssistOverlayHost {
            overlay,
            fixed,
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
    GEOM.with(|geom| *geom.borrow_mut() = (host.bounds, host.visible));
    let w = host.bounds.width.max(1.0).floor() as i32;
    let h = host.bounds.height.max(1.0).floor() as i32;
    host.fixed.set_size_request(w, h);
    if host.visible && !host.fixed.children().is_empty() {
        // show_all maps an already-visible WebKit child. Do not hide that child.
        host.fixed.show_all();
        size_assist_children(&host.fixed, host.bounds);
        log::debug!(
            "assist overlay bounds requested={}x{}@{},{} children={}",
            w,
            h,
            host.bounds.x,
            host.bounds.y,
            host.fixed.children().len()
        );
    } else {
        host.fixed.hide();
        log::debug!("assist overlay hidden requested={}x{}", w, h);
    }
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

/// Reparent the Assist webview into the overlay fixed.
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
                for child in host.fixed.children() {
                    host.fixed.remove(&child);
                }
                assist.set_hexpand(false);
                assist.set_vexpand(false);
                assist.set_halign(gtk::Align::Fill);
                assist.set_valign(gtk::Align::Fill);
                enable_assist_input(&assist);
                assist.connect_map(|widget| {
                    enable_assist_input(widget);
                    raise_widget_window(widget);
                });
                host.fixed.put(&assist, 0, 0);
                host.bounds = bounds;
                // Map once so WebKit realizes with its visible flag set, then
                // let desired visibility hide the fixed only.
                host.fixed.show_all();
                if !assist.is_realized() {
                    assist.realize();
                }
                size_assist_children(&host.fixed, bounds);
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
                host.visible = visible && !host.fixed.children().is_empty();
                for child in host.fixed.children() {
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
