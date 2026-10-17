// SPDX-License-Identifier: MIT

use super::*;
use crate::app::BrowserEvent;
use crate::model::Location;
use crate::services::LocationValidationError;

fn location(uri: &str) -> Location {
    Location::uri(uri)
}

fn column_added(depth: usize, location: Location) -> BrowserEvent {
    BrowserEvent::ColumnAdded { depth, location }
}

fn load_finished(depth: usize) -> BrowserEvent {
    BrowserEvent::LoadFinished {
        depth,
        truncated: false,
    }
}

/// The pinned tuple for a requested URI: the normalized location and its label.
fn expected_pin(uri: &str) -> (Location, String) {
    let normalized = normalized_location(location(uri));
    (normalized.clone(), pin_label(&normalized))
}

#[test]
fn load_finished_for_the_requested_location_pins_it() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(
        tracker.handle(&load_finished(0)),
        Some(expected_pin("smb://host/share")),
    );
    // The request is consumed.
    assert_eq!(tracker.handle(&load_finished(0)), None);
}

#[test]
fn a_trailing_slash_still_matches() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(
        tracker.handle(&column_added(0, location("smb://host/share/"))),
        None,
    );
    assert_eq!(
        tracker.handle(&load_finished(0)),
        Some(expected_pin("smb://host/share")),
    );
}

/// Unpin reloads the bookmark file through GIO, so the pinned row must carry
/// the same normalized URI the parser produces or Unpin can never match it.
#[test]
fn the_pinned_location_matches_the_bookmark_parser_form() {
    let mut tracker = PinTracker::default();
    tracker.request(location("smb://host/share"));
    tracker.handle(&column_added(0, location("smb://host/share")));

    let (pinned, _) = tracker
        .handle(&load_finished(0))
        .expect("the request pins on load");
    let parsed = crate::adapters::location_for_file(&gtk::gio::File::for_uri("smb://host/share"))
        .expect("GIO location");
    assert_eq!(pinned, parsed);
}

#[test]
fn an_unrelated_load_keeps_the_request_pending() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(
        tracker.handle(&column_added(0, location("smb://host/other"))),
        None,
    );
    assert_eq!(tracker.handle(&load_finished(0)), None);

    // A later visit to the requested location still pins it.
    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(
        tracker.handle(&load_finished(0)),
        Some(expected_pin("smb://host/share")),
    );
}

#[test]
fn a_nested_load_at_the_requested_depth_pins() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(
        tracker.handle(&column_added(0, location("smb://host"))),
        None,
    );
    assert_eq!(tracker.handle(&column_added(1, share.clone())), None);
    assert_eq!(
        tracker.handle(&load_finished(1)),
        Some(expected_pin("smb://host/share")),
    );
}

#[test]
fn reset_clears_the_chain_but_keeps_the_request() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(tracker.handle(&BrowserEvent::Reset), None);
    // The chain was cleared, so this load cannot satisfy the request.
    assert_eq!(tracker.handle(&load_finished(0)), None);

    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(
        tracker.handle(&load_finished(0)),
        Some(expected_pin("smb://host/share")),
    );
}

#[test]
fn truncation_drops_deeper_columns() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(
        tracker.handle(&column_added(0, location("smb://host"))),
        None,
    );
    assert_eq!(tracker.handle(&column_added(1, share.clone())), None);
    assert_eq!(
        tracker.handle(&BrowserEvent::ColumnsTruncated {
            len: 1,
            replacing: false,
        }),
        None,
    );
    assert_eq!(tracker.handle(&load_finished(1)), None);
}

#[test]
fn a_failed_load_keeps_the_request_for_retry() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(
        tracker.handle(&BrowserEvent::LoadFailed {
            depth: 0,
            message: "boom".to_owned(),
        }),
        None,
    );
    assert_eq!(
        tracker.handle(&load_finished(0)),
        Some(expected_pin("smb://host/share")),
    );
}

#[test]
fn a_mount_prompt_rejection_keeps_the_request() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(
        tracker.handle(&BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::Mountable(share.clone()),
        }),
        None,
    );
    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(
        tracker.handle(&load_finished(0)),
        Some(expected_pin("smb://host/share")),
    );
}

#[test]
fn a_terminal_rejection_clears_the_request() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(
        tracker.handle(&BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::Unavailable("no route".to_owned()),
        }),
        None,
    );
    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(tracker.handle(&load_finished(0)), None);
}

#[test]
fn a_deeper_terminal_rejection_also_clears_the_request() {
    let share = location("smb://host/share");
    let mut tracker = PinTracker::default();
    tracker.request(share.clone());

    assert_eq!(
        tracker.handle(&BrowserEvent::NavigationRejected {
            parent_depth: 0,
            error: LocationValidationError::Missing,
        }),
        None,
    );
    assert_eq!(tracker.handle(&column_added(0, share.clone())), None);
    assert_eq!(tracker.handle(&load_finished(0)), None);
}

#[test]
fn removed_mounts_unpin_only_network_schemes() {
    for uri in [
        "smb://host/share",
        "sftp://host/path",
        "ftp://host",
        "ftps://host",
        "dav://host/files",
        "davs://host/files",
        "SMB://HOST/share",
    ] {
        assert!(is_network_mount(&location(uri)), "{uri}");
    }
    for uri in ["mtp://usb0", "gphoto2://usb0", "network:///", "trash:///"] {
        assert!(!is_network_mount(&location(uri)), "{uri}");
    }
    assert!(
        !is_network_mount(&Location::local(std::path::PathBuf::from(
            "/media/usb0/share"
        ))),
        "native mounts keep their pins"
    );
}
