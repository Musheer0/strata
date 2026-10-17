// SPDX-License-Identifier: MIT

//! Add-network-location dialog and pin-after-successful-connect tracking.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    adapters::{gio_file_for_location, location_for_file},
    app::BrowserEvent,
    assets, i18n,
    model::Location,
    services::LocationValidationError,
    ui::{
        controls::{form_check_button, form_entry, form_error_label, form_label, modal_layout},
        modal::{
            ModalHost, close_on_escape, dismiss_modal_layer, modal_layer, remember_modal_focus,
            submit_on_enter,
        },
    },
};

use super::{PlaceMenuExtra, SidebarState};

#[cfg(test)]
pub(super) mod tests;

pub(super) fn menu_extra() -> PlaceMenuExtra {
    PlaceMenuExtra {
        icon: assets::icons::PLUS,
        label: "Add network location…",
        action: Rc::new(show),
    }
}

/// Registers the observer that pins a requested location after its first
/// successful load, plus the shared VolumeMonitor handler that unpins a network
/// location once it is removed. Returns the monitor handler id so the sidebar
/// can disconnect it (the monitor is process-wide).
pub(super) fn watch(state: &Rc<SidebarState>) -> gtk::glib::SignalHandlerId {
    let weak = Rc::downgrade(state);
    state.browser.observe(move |event| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let decision = state.network_pin.borrow_mut().handle(event);
        if let Some((location, label)) = decision {
            state.pin_location(location, label);
        }
    });

    let weak = Rc::downgrade(state);
    state.volume_monitor.connect_mount_removed(move |_, mount| {
        let Some(location) = location_for_file(&mount.root()) else {
            return;
        };
        if !is_network_mount(&location) {
            return;
        }
        // Defer past the monitor's emission; widgets are rebuilt from idle.
        let weak = weak.clone();
        gtk::glib::idle_add_local_once(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            // A replaced or reconnected mount keeps the pin: only a location
            // that is gone from the system loses it.
            let still_mounted = state
                .volume_monitor
                .mounts()
                .into_iter()
                .any(|other| location_for_file(&other.root()).as_ref() == Some(&location));
            if still_mounted {
                return;
            }
            state.unpin_location(&location);
        });
    })
}

/// Mount roots the app opens as network locations. Native disks and device
/// schemes (cameras, phones) keep their pinned folders when unplugged.
fn is_network_mount(location: &Location) -> bool {
    location
        .uri_value()
        .and_then(|uri| uri.split_once("://"))
        .is_some_and(|(scheme, _)| {
            matches!(
                scheme.to_ascii_lowercase().as_str(),
                "smb" | "sftp" | "ftp" | "ftps" | "dav" | "davs"
            )
        })
}

pub(super) fn show(state: &Rc<SidebarState>) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(&state.view.overlay())
    else {
        return;
    };

    let layout = modal_layout(
        assets::icons::NETWORK,
        &i18n::tr("Add network location"),
        &i18n::tr("Connect to a share or remote server"),
        &i18n::tr("Connect"),
    );
    layout.content.add_css_class("wide");

    let fields = gtk::Box::new(gtk::Orientation::Vertical, 5);
    fields.add_css_class("authentication-fields");
    let address_label = form_label(&i18n::tr("Server address"));
    let address = form_entry();
    address.set_placeholder_text(Some(&i18n::tr("smb://server/share")));
    let error = form_error_label();
    fields.append(&address_label);
    fields.append(&address);
    fields.append(&error);
    layout.body.append(&fields);

    let pin = form_check_button(&i18n::tr("Add to sidebar"));
    layout.body.append(&pin);

    {
        let watched = address.clone();
        let address_clear = address.clone();
        let error_clear = error.clone();
        watched.connect_changed(move |_| {
            address_clear.remove_css_class("error");
            error_clear.set_visible(false);
        });
    }

    let block_dismiss = {
        let address = address.clone();
        Rc::new(move || !address.text().trim().is_empty())
    };
    let layer = modal_layer(
        &layout.content,
        &window_overlay,
        blurred_root.clone(),
        Some(block_dismiss),
    );
    remember_modal_focus(&layer, &window_overlay);
    window_overlay.add_overlay(&layer);
    submit_on_enter(&layout.body, &layout.confirm);
    address.grab_focus();

    let dismiss: Rc<dyn Fn()> = Rc::new({
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        move || dismiss_modal_layer(&layer, &overlay, root.as_ref())
    });
    for button in [&layout.close, &layout.cancel] {
        let dismissed = dismiss.clone();
        button.connect_clicked(move |_| dismissed());
    }
    {
        let dismissed = dismiss.clone();
        close_on_escape(&layer, move || dismissed());
    }

    {
        let weak_state = Rc::downgrade(state);
        let address = address.clone();
        let error = error.clone();
        let dismiss = dismiss.clone();
        layout.confirm.connect_clicked(move |_| {
            let Some(state) = weak_state.upgrade() else {
                return;
            };
            let input = address.text().to_string();
            match state.view.typed_location(input.trim()) {
                Ok(location) => {
                    if pin.is_active() {
                        state.network_pin.borrow_mut().request(location);
                    }
                    dismiss();
                    state.browser.focus_active();
                }
                Err(err) => {
                    // `message()` is already localized; do not translate again.
                    error.set_text(&err.message());
                    error.set_visible(true);
                    address.add_css_class("error");
                }
            }
        });
    }
}

/// One in-flight pin request and the column chain it is matched against.
///
/// The chain mirrors the browser's column stack so a `LoadFinished` for an
/// unrelated directory cannot satisfy the request.
#[derive(Default)]
pub(super) struct PinTracker {
    pending: Option<(Location, String)>,
    chain: Vec<Option<Location>>,
}

impl PinTracker {
    pub(super) fn request(&mut self, location: Location) {
        // Pin GIO's normalized form: the bookmark parser reloads entries through
        // GIO (smb://host/share becomes smb://host/share/), and Unpin compares
        // the reloaded form against the row, so a raw typed URI would never match.
        let location = normalized_location(location);
        self.pending = Some((location.clone(), pin_label(&location)));
    }

    pub(super) fn handle(&mut self, event: &BrowserEvent) -> Option<(Location, String)> {
        match event {
            BrowserEvent::Reset => {
                self.chain.clear();
                None
            }
            BrowserEvent::ColumnsTruncated { len, .. } => {
                self.chain.truncate(*len);
                None
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                if self.chain.len() <= *depth {
                    self.chain.resize(depth + 1, None);
                }
                self.chain[*depth] = Some(location.clone());
                None
            }
            BrowserEvent::LoadFinished { depth, .. } => {
                let pending = self.pending.take()?;
                let matches = self
                    .chain
                    .get(*depth)
                    .and_then(Option::as_ref)
                    .is_some_and(|chain_location| same_location(chain_location, &pending.0));
                if matches {
                    Some(pending)
                } else {
                    self.pending = Some(pending);
                    None
                }
            }
            BrowserEvent::LoadFailed { .. } => None,
            BrowserEvent::LocationNavigationRejected { error }
            | BrowserEvent::NavigationRejected { error, .. } => {
                // A mount prompt is the normal path for remote locations; keep
                // the request so a successful mount can still pin.
                if matches!(
                    error,
                    LocationValidationError::Mountable(_) | LocationValidationError::NotMounted(_)
                ) {
                    None
                } else {
                    self.pending = None;
                    None
                }
            }
            _ => None,
        }
    }
}

fn pin_label(location: &Location) -> String {
    let name = location.display_name();
    if name.is_empty() {
        location.display_path()
    } else {
        name
    }
}

/// The location form the bookmark parser produces, so a pinned row survives the
/// reload Unpin performs. Falls back to the original if GIO rejects the URI.
fn normalized_location(location: Location) -> Location {
    location_for_file(&gio_file_for_location(&location)).unwrap_or(location)
}

fn same_location(a: &Location, b: &Location) -> bool {
    match (a.uri_value(), b.uri_value()) {
        (Some(a), Some(b)) => a.trim_end_matches('/') == b.trim_end_matches('/'),
        _ => a == b,
    }
}
