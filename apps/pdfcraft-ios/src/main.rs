//! PdfCraft for iPad: the same egui interface as the desktop app, in one full-screen UIKit window
//! (winit's iOS backend), drawn with wgpu on Metal.
//!
//! Build, bundle and run in the simulator: `packaging/ios/package.sh --run`. For an iPad, see
//! `packaging/ios/package.sh --help` (`--device` plus a signing identity and provisioning profile).
//!
//! Touch and Apple Pencil input is handled in the UI crate (`pdfcraft_ui_egui::touch`): fingers
//! scroll, zoom and tap; the Pencil is the precise pointer and draws.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod documents;

#[cfg(target_os = "ios")]
fn main() -> eframe::Result {
    use pdfcraft_ui_egui::{OsEvent, PdfCraftApp};

    log::set_logger(&STDERR_LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info)).ok();
    // Last-resort guard (AGENTS.md §4): commands, edits, opens and saves catch panics and report
    // them; this hook logs every panic, caught or not. Standard error is the simulator console
    // (`package.sh --run`) and the device log (Console.app).
    std::panic::set_hook(Box::new(|info| {
        let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("pdfcraft: internal error: {info}\n"));
    }));

    let folders = documents::Folders::current();
    if let Some(f) = &folders {
        f.create();
        log::info!("PdfCraft {} for iPad; documents in {}", env!("CARGO_PKG_VERSION"), f.documents.display());
    }
    let mut native = eframe::NativeOptions {
        persistence_path: folders.as_ref().map(|f| f.settings.join("app.ron")),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    configure_gpu(&mut native);
    eframe::run_native(
        "PdfCraft",
        native,
        Box::new(move |cc| {
            let mut app = PdfCraftApp::new();
            // Before `restore`, which then remembers whether a Pencil has been used.
            app.enable_tablet(pdfcraft_ui_egui::touch::Tablet::IPad);
            if let Some(json) = cc.storage.and_then(|s| s.get_string("pdfcraft")) {
                app.restore(&json);
            }
            if let Some(f) = folders {
                app.use_documents_folder(&f.documents);
                app.enable_recovery(pdfcraft_ui_egui::RecoveryStore::new(f.settings.join("Recovery")));
                let mut inbox = documents::Inbox::new(f.documents.clone());
                // Files shared while the app wasn't running are waiting already.
                for p in inbox.take() {
                    app.open_path(&p.to_string_lossy());
                }
                let ctx = cc.egui_ctx.clone();
                app.os_events = Some(Box::new(move || {
                    // Keep polling while idle, so a file shared while PdfCraft is in front opens.
                    ctx.request_repaint_after(documents::POLL_EVERY);
                    let opened = inbox.poll(std::time::Instant::now());
                    if opened.is_empty() {
                        Vec::new()
                    } else {
                        vec![OsEvent::Open(opened.iter().map(|p| p.to_string_lossy().into_owned()).collect())]
                    }
                }));
            }
            app.reopen_last_files(&[]);
            Ok(Box::new(app))
        }),
    )
}

/// Ask the GPU for exactly the limits it has. egui-wgpu asks for wgpu's desktop defaults, which
/// the iOS Simulator's Metal device doesn't meet (15 inter-stage shader variables, not 16), so
/// the app quit at start. Requesting the adapter's own limits always succeeds, and they include
/// its full texture size, which a 13-inch iPad's 2752-pixel screen needs anyway.
#[cfg(target_os = "ios")]
fn configure_gpu(native: &mut eframe::NativeOptions) {
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut native.wgpu_options.wgpu_setup else { return };
    let device_descriptor = std::sync::Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = std::sync::Arc::new(move |adapter| {
        let mut descriptor = device_descriptor(adapter);
        descriptor.required_limits = adapter.limits();
        descriptor
    });
}

/// Records to standard error; the desktop app's log file and rotation aren't needed on iPadOS,
/// where the system keeps the device log.
#[cfg(target_os = "ios")]
struct StderrLogger;

#[cfg(target_os = "ios")]
static STDERR_LOGGER: StderrLogger = StderrLogger;

#[cfg(target_os = "ios")]
impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        // wgpu and naga are chatty: at every start in the Simulator they warn that its Metal
        // device lacks WebGPU features PdfCraft doesn't use. Their errors still show.
        let quiet = metadata.target().starts_with("wgpu") || metadata.target().starts_with("naga");
        let level = if quiet { log::Level::Error } else { log::Level::Warn };
        metadata.level() <= level || metadata.target().starts_with("pdfcraft")
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            // Never `eprintln!`: it panics on a closed standard error.
            let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("pdfcraft [{}] {}\n", record.level(), record.args()));
        }
    }

    fn flush(&self) {}
}

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("pdfcraft-ios targets iPadOS: run packaging/ios/package.sh --run (simulator) or --device");
}
