//! Phase 3's prerequisite spike (see the master plan's "COM strategy —
//! corrected"): a real-host answer to whether COM can be used safely
//! in-process alongside Tauri/WebView2, run once and its conclusion
//! recorded here rather than re-run on every launch.
//!
//! `CoInitializeSecurity` is process-wide and callable only once for the
//! life of the process; a dedicated thread does not isolate it (only
//! `CoInitializeEx`'s apartment choice is per-thread). This spike answers,
//! on a real Windows host with the real app running:
//!
//! 1. What COM state does the main thread already have when our `setup()`
//!    runs (has WebView2 already initialized an apartment)?
//! 2. Does `CoInitializeSecurity` succeed, or does it return
//!    `RPC_E_TOO_LATE` because WebView2 (or Tauri/wry) already called it?
//! 3. From a dedicated MTA worker thread, with **no** process-wide
//!    `CoInitializeSecurity` call of our own: does WMI work using process
//!    default security plus `CoSetProxyBlanket` on the acquired proxy?
//!    Does Task Scheduler's COM object work the same way?
//! 4. Does any of the above disturb WebView2 — does the window still
//!    render, does the app still exit cleanly?
//!
//! ## Result (run on a real Windows 11 host, real `system-pulse.exe`,
//! `run_spike()` called from the first line of `windows::setup()` — since
//! removed from that call site; only this recorded finding remains)
//!
//! Raw `SpikeResult` observed:
//! ```text
//! SpikeResult {
//!     main_thread_co_init_hresult: 1,            // S_FALSE
//!     co_init_security_hresult: -2147417831,     // RPC_E_TOO_LATE (0x80010119)
//!     co_init_security_called_twice_hresult: -2147417831, // RPC_E_TOO_LATE, same
//!     wmi_connect_ok: true,
//!     wmi_query_ok: true,
//!     task_service_connect_ok: true,
//! }
//! ```
//!
//! 1. **Main thread COM state**: `CoInitializeEx` returned `S_FALSE`, not
//!    `S_OK` — something (wry/WebView2, initialized before
//!    `tauri::Builder::setup` runs our code) has **already** initialized
//!    COM on the main thread by the time `setup()` executes.
//! 2. **`CoInitializeSecurity`**: returned `RPC_E_TOO_LATE` on the very
//!    first call, not just the second — process-wide COM security had
//!    **already been claimed** before our code ever ran, presumably by
//!    the same wry/WebView2 initialization. There is no window in which
//!    this app could successfully call `CoInitializeSecurity` itself from
//!    `setup()`; the plan's cautious default ("we should probably not
//!    call it at all") is not just cautious, it's the *only* option here.
//! 3. **WMI and Task Scheduler from an MTA worker thread**: both
//!    connected and queried successfully using only `CoSetProxyBlanket`
//!    on the acquired proxy — with **no** successful process-wide
//!    `CoInitializeSecurity` call anywhere in the process (confirmed by
//!    #2). The plan's fallback design is therefore not a fallback here;
//!    it's the *only* viable path, and it works.
//! 4. **WebView2 interference**: none observed. The window rendered
//!    normally, `msedgewebview2.exe` subprocesses started as expected,
//!    the app produced no error/panic output, and it exited cleanly on
//!    `taskkill` after roughly 15 seconds of running with the spike's
//!    COM/WMI/Task-Scheduler activity already completed on the worker
//!    thread.
//!
//! **Conclusion: WMI and Task Scheduler COM are safe to use in-process,
//! via a dedicated MTA worker thread using `CoSetProxyBlanket` per proxy
//! and *no* process-wide `CoInitializeSecurity` call.** Phase 3's
//! Task-Scheduler-backed collector is therefore in scope (not deferred).
//! `CoInitializeSecurity` must never be called by this app's own code —
//! it will always fail with `RPC_E_TOO_LATE` in-process, and the failure
//! is harmless (every collector here is built on the per-proxy blanket
//! pattern, which doesn't need it to have succeeded).

#![allow(dead_code)]

use windows::core::BSTR;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoInitializeSecurity, CoSetProxyBlanket, CoUninitialize,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::TaskScheduler::{ITaskService, TaskScheduler};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::{
    IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
    WBEM_GENERIC_FLAG_TYPE,
};

/// `Result<(), windows::core::Error>` -> the HRESULT `i32` this spike
/// records (`0` for success, matching `S_OK`, since `windows-rs` no
/// longer exposes the raw `HRESULT` on the `Ok` path — only `Error`
/// carries a `.code()`).
fn hresult_of(r: &windows::core::Result<()>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => e.code().0,
    }
}

#[derive(Debug, Default)]
pub struct SpikeResult {
    pub main_thread_co_init_hresult: i32,
    pub co_init_security_hresult: i32,
    pub co_init_security_called_twice_hresult: i32,
    pub wmi_connect_ok: bool,
    pub wmi_query_ok: bool,
    pub task_service_connect_ok: bool,
}

/// Runs the full spike from wherever it's called. Intended to be invoked
/// once, manually, from inside the real app's `setup()` for the empirical
/// run recorded in this module's doc comment above — not part of the
/// normal collector startup path.
#[allow(unsafe_code)]
pub fn run_spike() -> SpikeResult {
    let mut result = SpikeResult::default();

    // SAFETY: called once, on the thread that will become the main/UI
    // thread, before any COM object is created on it.
    let main_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    result.main_thread_co_init_hresult = main_init.0;

    // SAFETY: NULL security descriptor + documented default authentication
    // parameters, exactly per `CoInitializeSecurity`'s "typical client"
    // usage shown in the Win32 docs; called at most once successfully per
    // process, which this spike itself verifies by calling it twice.
    let sec1 = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        )
    };
    result.co_init_security_hresult = hresult_of(&sec1);

    // SAFETY: same call again — expected to fail with RPC_E_TOO_LATE,
    // verifying the "once per process" rule empirically rather than
    // trusting documentation alone.
    let sec2 = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        )
    };
    result.co_init_security_called_twice_hresult = hresult_of(&sec2);

    let worker = std::thread::spawn(worker_thread_probe);
    if let Ok((wmi_connect_ok, wmi_query_ok, task_service_connect_ok)) = worker.join() {
        result.wmi_connect_ok = wmi_connect_ok;
        result.wmi_query_ok = wmi_query_ok;
        result.task_service_connect_ok = task_service_connect_ok;
    }

    // SAFETY: matches the successful `CoInitializeEx` above; called once,
    // from the same thread, after all COM use on this thread is done.
    if main_init.is_ok() {
        unsafe { CoUninitialize() };
    }

    result
}

#[allow(unsafe_code)]
fn worker_thread_probe() -> (bool, bool, bool) {
    // SAFETY: fresh thread, MTA, no prior COM state.
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if init.is_err() {
        return (false, false, false);
    }

    let (wmi_connect_ok, wmi_query_ok) = probe_wmi();
    let task_ok = probe_task_scheduler();

    // SAFETY: matches the `CoInitializeEx` above, same thread, after all
    // COM use here is finished.
    unsafe { CoUninitialize() };

    (wmi_connect_ok, wmi_query_ok, task_ok)
}

#[allow(unsafe_code)]
fn probe_wmi() -> (bool, bool) {
    // SAFETY: standard `CoCreateInstance` usage; `WbemLocator`/`IWbemLocator`
    // are the documented WMI entry point.
    let locator: windows::core::Result<IWbemLocator> =
        unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) };
    let Ok(locator) = locator else {
        return (false, false);
    };

    // SAFETY: `ConnectServer` per its documented signature; a NULL/empty
    // BSTR for user/password/locale means "use the caller's own token,"
    // exactly the unprivileged, no-credentials-prompt path this needs.
    let services: windows::core::Result<IWbemServices> = unsafe {
        locator.ConnectServer(
            &BSTR::from(r"ROOT\CIMV2"),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            0,
            &BSTR::new(),
            None,
        )
    };
    let Ok(services) = services else {
        return (true, false);
    };

    // SAFETY: `CoSetProxyBlanket` on the freshly-acquired proxy, exactly
    // the plan's "no process-wide call, per-proxy blanket" pattern —
    // deliberately applied here even though the process-wide
    // `CoInitializeSecurity` call above already succeeded in this spike,
    // so the collectors built on this pattern don't depend on that also
    // being true in every future build.
    let blanket = unsafe {
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
    };
    if blanket.is_err() {
        return (true, false);
    }

    // SAFETY: a minimal, read-only WQL query; `ExecQuery` is documented to
    // return a valid enumerator or an error, never a dangling pointer.
    let query = unsafe {
        services.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT Name FROM Win32_OperatingSystem"),
            WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0),
            None,
        )
    };
    let Ok(enumerator) = query else {
        return (true, false);
    };

    // SAFETY: `Next` with a 1-item buffer and a bounded timeout, exactly
    // the documented single-row-fetch idiom.
    let mut row = [None; 1];
    let mut returned = 0u32;
    let next = unsafe { enumerator.Next(5000, &mut row, &mut returned) };
    (true, next.is_ok() && returned > 0)
}

#[allow(unsafe_code)]
fn probe_task_scheduler() -> bool {
    // SAFETY: standard `CoCreateInstance` usage for the documented Task
    // Scheduler 2.0 entry point.
    let service: windows::core::Result<ITaskService> =
        unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) };
    let Ok(service) = service else {
        return false;
    };
    // SAFETY: `Connect` with all-empty variants means "connect to the
    // local machine as the calling user," the same unprivileged,
    // no-credentials-prompt shape as the WMI probe above.
    let connected = unsafe {
        service.Connect(
            &VARIANT::default(),
            &VARIANT::default(),
            &VARIANT::default(),
            &VARIANT::default(),
        )
    };
    if connected.is_err() {
        return false;
    }
    // SAFETY: `GetFolder` on the root folder path, a read-only call.
    let root: windows::core::Result<_> = unsafe { service.GetFolder(&BSTR::from("\\")) };
    root.is_ok()
}
