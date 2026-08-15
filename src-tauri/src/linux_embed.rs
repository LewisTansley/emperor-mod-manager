//! Linux-only overlay so the Assist child webview can be positioned
//! inside the Downloads host frame.
//!
//! Tauri packs Linux child webviews into the window `GtkBox` with expand=true,
//! so `set_bounds` is ignored. We wrap the box in a `GtkOverlay` and reparent
//! Assist into a `GtkLayout` whose bin window clips children to the host rect.

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
    layout: gtk::Layout,
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

fn clamp_layout_children(layout: &gtk::Layout, alloc: &gtk::Allocation) {
    if CLAMPING.get() {
        return;
    }
    CLAMPING.set(true);
    let w = alloc.width().max(1);
    let h = alloc.height().max(1);
    let clip = gtk::Allocation::new(0, 0, w, h);
    for child in layout.children() {
        child.set_hexpand(false);
        child.set_vexpand(false);
        child.set_halign(gtk::Align::Fill);
        child.set_valign(gtk::Align::Fill);
        child.set_size_request(w, h);
        child.size_allocate(&clip);
        child.set_clip(&clip);
        layout.move_(&child, 0, 0);
        let mut surface_w = 0;
        let mut surface_h = 0;
        if child.has_window() && child.is_realized() {
            if let Some(win) = child.window() {
                win.move_resize(0, 0, w, h);
                surface_w = win.width();
                surface_h = win.height();
            }
        }
        let ca = child.allocation();
        log::debug!(
            "assist overlay clamp requested={}x{} layout_alloc={}x{}@{},{} child={}x{}@{},{} has_window={} realized={} surface={}x{}",
            w,
            h,
            alloc.width(),
            alloc.height(),
            alloc.x(),
            alloc.y(),
            ca.width(),
            ca.height(),
            ca.x(),
            ca.y(),
            child.has_window(),
            child.is_realized(),
            surface_w,
            surface_h
        );
    }
    CLAMPING.set(false);
}

fn make_clip_layout() -> gtk::Layout {
    let layout = gtk::Layout::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    layout.set_hexpand(false);
    layout.set_vexpand(false);
    layout.set_halign(gtk::Align::Start);
    layout.set_valign(gtk::Align::Start);
    layout.connect_size_allocate(|layout, alloc| {
        clamp_layout_children(layout, alloc);
    });
    layout
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

    let layout = make_clip_layout();
    overlay.add_overlay(&layout);

    gtk_window.add(&overlay);
    overlay.show_all();
    layout.hide();

    let position_layout = layout.clone();
    let position_overlay = overlay.clone();
    let position_main = main_webview.clone();
    overlay.connect_get_child_position(move |_overlay, widget| {
        if widget != position_layout.upcast_ref::<gtk::Widget>() {
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
            layout,
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
    host.layout.set_size(w.max(0) as u32, h.max(0) as u32);
    host.layout.set_size_request(w, h);
    if host.visible && !host.layout.children().is_empty() {
        host.layout.show_all();
        let alloc = host.layout.allocation();
        clamp_layout_children(
            &host.layout,
            &gtk::Allocation::new(alloc.x(), alloc.y(), w, h),
        );
        log::debug!(
            "assist overlay bounds requested={}x{}@{},{} layout={}x{}@{},{} children={}",
            w,
            h,
            host.bounds.x,
            host.bounds.y,
            alloc.width(),
            alloc.height(),
            alloc.x(),
            alloc.y(),
            host.layout.children().len()
        );
        for child in host.layout.children() {
            let ca = child.allocation();
            let (surface_w, surface_h) = if child.has_window() {
                child
                    .window()
                    .map(|win| (win.width(), win.height()))
                    .unwrap_or((0, 0))
            } else {
                (0, 0)
            };
            log::debug!(
                "assist overlay child alloc={}x{}@{},{} has_window={} realized={} surface={}x{}",
                ca.width(),
                ca.height(),
                ca.x(),
                ca.y(),
                child.has_window(),
                child.is_realized(),
                surface_w,
                surface_h
            );
        }
    } else {
        host.layout.hide();
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

/// Reparent the Assist webview into the clipping overlay layout.
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
                for child in host.layout.children() {
                    host.layout.remove(&child);
                }
                assist.set_hexpand(false);
                assist.set_vexpand(false);
                assist.set_halign(gtk::Align::Fill);
                assist.set_valign(gtk::Align::Fill);
                host.layout.put(&assist, 0, 0);
                host.bounds = bounds;
                // Visibility is owned by set_visible / desired-visible flag, not size.
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
                host.visible = visible && !host.layout.children().is_empty();
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
