use std::marker::PhantomData;
use std::rc::Rc;

use warp_core::send_telemetry_from_ctx;
use warpui::ModelContext;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::SecurityCenter::*;

use crate::antivirus::telemetry::AntivirusInfoTelemetryEvent;
use crate::antivirus::{AntivirusInfo, AntivirusInfoEvent};

impl AntivirusInfo {
    #[cfg(windows)]
    pub(super) async fn scan() -> anyhow::Result<Option<String>> {
        let _com = ComGuard::new()?;

        unsafe {
            // Read out all of the registered antivirus products.
            let pl: IWSCProductList = CoCreateInstance(&WSCProductList, None, CLSCTX_ALL)?;
            pl.Initialize(WSC_SECURITY_PROVIDER_ANTIVIRUS)?;

            let n = pl.Count().unwrap_or(0) as u32;
            for i in 0..n {
                let Ok(p) = pl.get_Item(i) else {
                    continue;
                };

                // If the product is on (meaning it's running), return it.
                if let Ok(WSC_SECURITY_PRODUCT_STATE_ON) = p.ProductState() {
                    return Ok(p.ProductName().ok().map(|s| s.to_string()));
                }
            }
        }

        Ok(None)
    }

    pub(super) fn on_scan_complete(
        &mut self,
        software: anyhow::Result<Option<String>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let software = match software {
            Ok(software) => software,
            Err(err) => {
                log::warn!("Failed to scan for antivirus / EDR software: {err:#}");
                return;
            }
        };

        match software.as_ref() {
            None => {
                log::info!("No antivirus / EDR software detected");
            }
            Some(software) => {
                log::info!("Detected antivirus / EDR software {software:#?}");
                send_telemetry_from_ctx!(
                    AntivirusInfoTelemetryEvent::AntivirusDetected {
                        name: software.into()
                    },
                    ctx
                );
            }
        }

        self.0 = software;

        ctx.emit(AntivirusInfoEvent::ScannedComplete);
    }
}

/// RAII guard that initializes the Windows COM library for the current thread and uninitializes it
/// when dropped.
///
/// Per the Windows docs (https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-coinitializeex)
/// each successful call to [`CoInitializeEx`] must be paired with a call to [`CoUninitialize`] on
/// the same thread.
// TODO(alokedesai): Move this to a shared place in `core` so we can use it in other places in the
// app.
struct ComGuard {
    // Tie the guard to its creating thread so `CoUninitialize` only runs there.
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl ComGuard {
    /// Initializes COM as a single-threaded apartment for the current thread.
    fn new() -> Result<Self, windows::core::Error> {
        // SAFETY: balanced by `CoUninitialize` in `Drop`, on the same thread.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self {
            not_send_or_sync: PhantomData,
        })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: `new` succeeded on this thread (otherwise this guard would not exist), and this
        // runs in the synchronous `scan` scope rather than a TLS destructor, so it is safe to
        // balance the init here.
        unsafe { CoUninitialize() };
    }
}
