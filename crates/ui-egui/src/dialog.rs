//! The file-dialog builder every picker in the UI is described with.
//!
//! Desktop and web use `rfd`'s. iPadOS has no `rfd` backend (and showing a UIKit document picker
//! needs Objective-C classes, i.e. `unsafe`, which this workspace forbids), so there the same
//! builder describes the in-app picker that lists the app's Documents folder
//! ([`crate::in_app_picker`]). Call sites build dialogs the same way on every platform.

#[cfg(not(target_os = "ios"))]
pub(crate) use rfd::AsyncFileDialog;

#[cfg(target_os = "ios")]
pub(crate) use crate::in_app_picker::AsyncFileDialog;
